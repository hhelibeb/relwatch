//! media 图片网关：让 WebView 里的远程图片经 Rust 的 reqwest 下载，继承应用代理。
//!
//! ## 背景
//!
//! 封面 / release 正文图片原本由 `<img>` 直接指向远程 URL，请求由 Chromium 网络栈
//! 发出，只认系统代理，与应用 `proxy_mode` 设置零关联（代理脱管问题）。修复思路：
//! 前端把远程图片 URL 改写成本应用的 `http://media.localhost/<url>`，注册本协议，
//! 由 Rust 端用**已按 ProxyPolicy 构建的 client** 下载后返回给 Chromium。
//! 磁盘缓存避免同一图片反复滚动时重复回源下载。
//!
//! ## 平台差异
//!
//! Windows WebView2 只认 `http://media.localhost/` 形式（裸 scheme 不认），
//! Linux/macOS 的 webkitgtk / WKWebView 对自定义协议同样接受该形式。
//! 因此前端统一用 `http://media.localhost/`。
//!
//! ## URL 格式
//!
//! path 是原始 URL 的 percent-encoding（encodeURIComponent 语义），解码后得
//! 原始 `https://...`。仅接受 http/https。
//!
//! ## 安全
//!
//! - 仅转发公网可访问的 http(s)（`ensure_public_url` 逐跳校验，防 SSRF）；
//! - 磁盘缓存 key 是 URL 的 SHA-256，不信任外部输入做文件名；
//! - 单文件 25MB 上限（与 commands/download.rs 对齐），防异常响应撑爆内存/磁盘；
//! - 非 http(s) 直接 400；请求失败 502；缓存/下载错误不暴露内部细节。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use tauri::http::{Response, StatusCode};
use tauri::{AppHandle, Manager};

use crate::db::init::app_data_dir;
use crate::db::settings::{get_setting, KEY_PROXY_MODE, KEY_PROXY_URL};
use crate::http;
use crate::types::AppState;

/// media 缓存目录（RelWatch 数据目录下）。进程生命周期内只建一次。
pub fn media_cache_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = app_data_dir().join("media-cache");
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

/// 单次下载/缓存文件大小上限：25MB（与 commands/download.rs 一致，改动需两处同步）。
pub const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;

/// 缓存 TTL：图片 URL 通常稳定，7 天过期足够覆盖活跃 release，又不至于让失效
/// 封面永久驻留。TTL 以 `fetched_at` 元数据计（而非文件 mtime），与清理逻辑解耦。
const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// 磁盘缓存总量上限（约 200MB）。超出后按 mtime 升序清理最旧文件。
const MAX_CACHE_BYTES: u64 = 200 * 1024 * 1024;

/// 每次请求后最多清理的文件数：大目录下分批次收敛，避免单次全量扫描卡线程。
const EVICT_BATCH: usize = 32;

/// 缓存条目元数据文件名后缀：`<hash>.json` 存 content_type + fetched_at(unix secs)。
const META_SUFFIX: &str = ".json";

// ── 缓存 key 与文件布局 ──

/// URL → 缓存文件绝对路径的确定性映射（不信任外部文件名）。
fn cache_path_for_url(url: &str) -> PathBuf {
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    media_cache_dir().join(digest)
}

/// URL → 元数据文件路径。
fn meta_path_for_url(url: &str) -> PathBuf {
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    media_cache_dir().join(format!("{digest}{META_SUFFIX}"))
}

/// 元数据条目。
struct CacheMeta {
    content_type: String,
    fetched_at: SystemTime,
}

/// 读元数据文件（不存在视为未命中）。
fn read_meta(path: &Path) -> Option<CacheMeta> {
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let content_type = v.get("content_type")?.as_str()?.to_string();
    let fetched_at = v
        .get("fetched_at")?
        .as_i64()
        .and_then(|s| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(s as u64)))?;
    Some(CacheMeta { content_type, fetched_at })
}

/// 命中缓存且未过期则返回 (bytes, content_type)；否则 Ok(None)。
fn read_cached(url: &str) -> Result<Option<(Vec<u8>, String)>, String> {
    let bytes_path = cache_path_for_url(url);
    if !bytes_path.exists() {
        return Ok(None);
    }
    let Some(meta) = read_meta(&meta_path_for_url(url)) else {
        // 元数据缺失/损坏：当作未命中并清理孤儿文件
        let _ = std::fs::remove_file(&bytes_path);
        return Ok(None);
    };
    if SystemTime::now()
        .duration_since(meta.fetched_at)
        .map(|age| age > CACHE_TTL)
        .unwrap_or(true)
    {
        // 过期：删除本体与元数据，让回源重新拉取
        let _ = std::fs::remove_file(&bytes_path);
        let _ = std::fs::remove_file(meta_path_for_url(url));
        return Ok(None);
    }
    let bytes = std::fs::read(&bytes_path).map_err(|e| e.to_string())?;
    Ok(Some((bytes, meta.content_type)))
}

/// 写缓存（原子：先写临时再 rename，避免半截文件被当完整命中）。
fn write_cache(url: &str, bytes: &[u8], content_type: &str) {
    let bytes_path = cache_path_for_url(url);
    let tmp = bytes_path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &bytes_path);
    }
    let meta = serde_json::json!({
        "content_type": content_type,
        "fetched_at": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    let _ = std::fs::write(meta_path_for_url(url), meta.to_string());
}

/// 磁盘 LRU 收口：总量超限时按 mtime 升序删最旧文件（每次最多删 EVICT_BATCH）。
/// 在「新写入后」触发；用非阻塞 try_lock 避免并发重复全量扫描。
fn evict_if_needed() {
    static CLEANING: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
    let mut guard = match CLEANING.try_lock() {
        Ok(g) => g,
        Err(_) => return, // 已有清理在跑
    };
    if *guard {
        return;
    }
    *guard = true;

    let dir = media_cache_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        *guard = false;
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
    let mut total: u64 = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("tmp") {
            let _ = std::fs::remove_file(&p); // 遗留临时文件
            continue;
        }
        if let Ok(m) = e.metadata() {
            if m.is_file() {
                if let Ok(t) = m.modified() {
                    total += m.len();
                    files.push((t, p, m.len()));
                }
            }
        }
    }
    if total <= MAX_CACHE_BYTES {
        *guard = false;
        return;
    }
    files.sort_by_key(|(t, _, _)| *t);
    let mut removed: u64 = 0;
    let mut removed_count = 0usize;
    for (_, p, len) in files {
        if total - removed <= MAX_CACHE_BYTES {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            removed += len;
            removed_count += 1;
        }
        if removed_count >= EVICT_BATCH {
            break;
        }
    }
    *guard = false;
}

// ── media URL 解码 ──

/// 把 media path（percent-encoded 原始 URL）解码为 http(s) 地址。
pub fn decode_media_path(path: &str) -> Result<String, String> {
    let raw = percent_decode(path)?;
    if raw.starts_with("https://") || raw.starts_with("http://") {
        Ok(raw)
    } else {
        Err("err.invalid_url".to_string())
    }
}

fn percent_decode(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).map_err(|_| "err.invalid_url".to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── 服务入口 ──

/// 从 AppHandle 读代理设置，构建「已按 ProxyPolicy 配置」的下载 client。
fn build_proxied_client(app: &AppHandle) -> Result<reqwest::Client, String> {
    let (proxy_url, proxy_mode) = {
        let conn = app
            .state::<AppState>()
            .db
            .get()
            .map_err(|e| format!("err.db_connect|{}", e))?;
        let pu = get_setting(&conn, KEY_PROXY_URL)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let pm = get_setting(&conn, KEY_PROXY_MODE)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| {
                if pu.is_empty() {
                    "none".to_string()
                } else {
                    "custom".to_string()
                }
            });
        (pu, pm)
    };
    http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        // 禁自动重定向：逐跳 SSRF 校验必须手动跟随
        follow_redirects: false,
        ..Default::default()
    })
}

/// 解码 + 命中缓存/回源下载。返回 (bytes, content_type)。
async fn fetch_or_cache(app: &AppHandle, path: &str) -> Result<(Vec<u8>, String), String> {
    let url = decode_media_path(path.trim_start_matches('/'))?;
    if let Some(hit) = read_cached(&url)? {
        return Ok(hit);
    }
    let client = build_proxied_client(app)?;
    let (bytes, content_type) =
        http::fetch_public_with_headers(&client, &url, MAX_MEDIA_BYTES).await?;
    let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_string());
    write_cache(&url, &bytes, &content_type);
    evict_if_needed();
    Ok((bytes, content_type))
}

/// 处理一次 media 请求：组装 http 响应（成功 200 / 参数错 400 / 下载失败 502）。
/// 供 URI scheme 协议 handler 在异步上下文调用后喂给 responder。
pub async fn handle_media_request(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    match fetch_or_cache(app, path).await {
        Ok((bytes, content_type)) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", content_type)
            .header("cache-control", "private, max-age=86400")
            .body(bytes)
            .unwrap_or_else(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
        Err(e) if e == "err.invalid_url" => error_response(StatusCode::BAD_REQUEST, &e),
        Err(e) => error_response(StatusCode::BAD_GATEWAY, &e),
    }
}

fn error_response(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .body(message.as_bytes().to_vec())
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Vec::new())
                .expect("empty body response is valid")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_plain_https_url() {
        let u = decode_media_path("https%3A%2F%2Fi.ytimg.com%2Fvi%2Fx%2Fmqdefault.jpg").unwrap();
        assert_eq!(u, "https://i.ytimg.com/vi/x/mqdefault.jpg");
    }

    #[test]
    fn decode_keeps_query_and_slash() {
        let u = decode_media_path("https%3A%2F%2Fexample.com%2Fa%2Fb.png%3Fx%3D1%26y%3D2").unwrap();
        assert_eq!(u, "https://example.com/a/b.png?x=1&y=2");
    }

    #[test]
    fn decode_strips_leading_slash() {
        // handler 会把 uri path 整体传入（可能带前导 /），trim 交给调用层
        let u = decode_media_path("https%3A%2F%2Fexample.com%2Fx.png").unwrap();
        assert_eq!(u, "https://example.com/x.png");
    }

    #[test]
    fn decode_rejects_non_http() {
        assert!(decode_media_path("ftp%3A%2F%2Fexample.com").is_err());
        assert!(decode_media_path("file%3A%2F%2F%2Fetc%2Fpasswd").is_err());
        assert!(decode_media_path("javascript%3Aalert(1)").is_err());
        assert!(decode_media_path("").is_err());
    }

    #[test]
    fn decode_rejects_bad_encoding() {
        // %ZZ 不是合法 hex → 原样保留 → 不再是合法 URL 前缀 → 拒绝
        assert!(decode_media_path("https%ZZ%2F%2F").is_err());
    }

    #[test]
    fn cache_path_is_stable_and_scoped() {
        let p1 = cache_path_for_url("https://i.ytimg.com/vi/x/mqdefault.jpg");
        let p2 = cache_path_for_url("https://i.ytimg.com/vi/x/mqdefault.jpg");
        assert_eq!(p1, p2);
        assert!(p1.starts_with(media_cache_dir()));
        let p3 = cache_path_for_url("https://i.ytimg.com/vi/y/mqdefault.jpg");
        assert_ne!(p1, p3);
    }

    #[test]
    fn write_then_read_cache_roundtrip() {
        // 用 .invalid 域名仅测缓存读写，不触发回源
        let url = "https://example.invalid/img.png";
        write_cache(url, b"fake-image-bytes", "image/png");
        let got = read_cached(url).unwrap().expect("应命中缓存");
        assert_eq!(got.0, b"fake-image-bytes");
        assert_eq!(got.1, "image/png");
        // 清理测试痕迹
        let _ = std::fs::remove_file(cache_path_for_url(url));
        let _ = std::fs::remove_file(meta_path_for_url(url));
    }

    #[test]
    fn cache_meta_missing_treats_as_miss_and_cleans_orphan() {
        let url = "https://example.invalid/orphan.png";
        let bytes_path = cache_path_for_url(url);
        std::fs::write(&bytes_path, b"orphan").unwrap();
        let got = read_cached(url).unwrap();
        assert!(got.is_none());
        assert!(!bytes_path.exists(), "孤儿缓存文件应被清理");
    }
}
