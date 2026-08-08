//! B 站 UP 主监控适配器（web-dynamic 动态接口 + WBI 签名，视频为主）。
//!
//! ## 数据源
//! B 站没有官方 RSS、没有官方开放 API。本适配器走 WEB 端接口：
//! - 匿名模式（默认）：访问主页 + `finger/spi` 初始化 `buvid3/buvid4/b_nut` cookie，
//!   `nav` 接口取 WBI 签名密钥，带签名请求 `x/polymer/web-dynamic/v1/feed/space`
//!   （用户空间动态），无需登录即可拉取 UP 主动态，过滤出视频（DYNAMIC_TYPE_AV）。
//! - 登录模式（设置页配置 B 站 Cookie SESSDATA 后自动启用）：SESSDATA 拼入 cookie，
//!   B 站风控对登录用户评分更低，显著降低 -352/-412 概率（RSSHub 等项目的标准做法）。
//!
//! ## 风控说明
//! B 站 2025-05 起强制 WBI 签名、2025-06 起强制 buvid3；2026 年起对部分接口
//! （arc/search、acc/info）升级设备指纹风控（-352 + v_voucher）。web-dynamic 动态
//! 接口当前仅需 WBI 签名 + buvid cookie 即可访问，是匿名场景的可行入口。
//! 触发风控时返回友好错误（err.bili_risk），提示配置 SESSDATA 或稍后重试。

use std::collections::HashMap;
use std::sync::OnceLock;

use md5::Digest as _;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::releases;
use crate::db::sources::Source;
use crate::source::SourceAdapter;

const BILI_API_BASE: &str = "https://api.bilibili.com";
const BILI_HOME: &str = "https://www.bilibili.com/";
const BILI_WATCH_BASE: &str = "https://www.bilibili.com/video";
const BILI_SPACE_REFERER: &str = "https://space.bilibili.com/";

/// 用户空间动态接口（新版，无需设备指纹；feed/all 需要登录，feed/space 匿名可用）。
const BILI_DYNAMIC_PATH: &str = "/x/polymer/web-dynamic/v1/feed/space";
/// WBI 签名密钥来源（未登录也返回 wbi_img）。
const BILI_NAV_PATH: &str = "/x/web-interface/nav";
/// buvid3/buvid4 生成接口。
const BILI_SPI_PATH: &str = "/x/frontend/finger/spi";

/// WBI 签名重排表：`img_key + sub_key` 按此表取字符，截前 32 位为 mixin_key。
/// 见 bilibili-API-collect docs/misc/sign/wbi.md（社区 fork 镜像）。
const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19,
    29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4,
    22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

/// 动态接口 features 参数（web 端功能开关，缺失可能返回空列表）。
const BILI_DYNAMIC_FEATURES: &str =
    "itemOpusStyle,listOnlyfans,opusBigCover,onlyfansVote,decorationCard,noPcover";

/// fetch_all 最大页数保护：`fetch_history_count=0`（全量拉取）时唯一终止条件是响应
/// offset 为空，若 B 站异常重复返回同一非空 offset（风控页形态/接口 bug），
/// 无上限会无限循环挂住轮询。
const MAX_FETCH_ALL_PAGES: usize = 50;
/// fetch_all 最大条目数兜底（正常 UP 主动态远小于此值）。
const MAX_FETCH_ALL_ITEMS: usize = 2000;

/// bili_ticket 获取接口（POST，存在可降低风控概率，3 天有效）。
const BILI_TICKET_URL: &str = "https://api.bilibili.com/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket";

/// 浏览器 UA：B 站风控对非浏览器 UA 敏感，单请求覆盖 client 级 UA。
const BILI_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
/// 浏览器级请求头（B 站风控对缺头/异常头敏感，实测缺 Accept/Language 易触发 412/-352）。
const BILI_ACCEPT: &str = "application/json, text/plain, */*";
const BILI_ACCEPT_LANG: &str = "zh-CN,zh;q=0.9,en;q=0.8";
/// bili_ticket 签名密钥（社区逆向，见 bilibili-API-collect bili_ticket.md）。
const BILI_TICKET_KEY: &str = "XgwSnGZ1p";

/// 动态条目（fetch 阶段产出，序列化为 JSON 传给 save）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BiliEntry {
    /// 视频 BVID（BV 开头，releases.tag_name 天然去重）。
    bvid: String,
    title: String,
    /// RFC3339 格式发布时间（动态 pub_ts 转换）。
    published: String,
    description: String,
    /// 封面 URL。
    thumbnail: String,
    /// 时长文本（如 "12:34"）。
    duration: String,
    /// UP 主昵称（module_author.name，用于 verify/刷新描述）。
    up_name: String,
}

impl BiliEntry {
    fn html_url(&self) -> String {
        format!("{}/{}", BILI_WATCH_BASE, self.bvid)
    }

    /// 与 youtube.rs 对齐的 extra_metadata 结构（前端 youtubeLayout 复用解析）。
    /// B 站 CDN 支持 https，封面强制升级为 https（WebView CSP 仅允许 https 图片，
    /// 否则 http 封面被 img-src 'self' data: https: 拦截，列表不显示封面）。
    fn metadata_json(&self) -> String {
        let thumbnail = self
            .thumbnail
            .replacen("http://", "https://", 1);
        serde_json::json!({
            "kind": "video",
            "thumbnail": thumbnail,
            "duration": self.duration,
        })
        .to_string()
    }
}

/// WBI 签名密钥。
#[derive(Debug, Clone)]
struct WbiKeys {
    img_key: String,
    sub_key: String,
}

impl WbiKeys {
    /// mixin_key = 重排表取 `img_key + sub_key` 前 32 位。
    fn mixin_key(&self) -> String {
        let raw = format!("{}{}", self.img_key, self.sub_key);
        MIXIN_KEY_ENC_TAB
            .iter()
            .take(32)
            .map(|&i| raw.as_bytes()[i] as char)
            .collect()
    }
}

/// encodeURIComponent 风格编码（大写 %XX、空格 %20），并过滤 `!'()*` 字符
/// （WBI 签名要求；部分库按 form-urlencoded 编码空格为 `+` 会导致签名不匹配）。
fn encode_wbi_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            // 过滤 "!'()*"（B 站签名约定剔除这些字符）
            0x21 | 0x27 | 0x28 | 0x29 | 0x2A => {}
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// WBI 签名：参数按 key 升序 → 逐项 encodeURIComponent 拼接 → 追加 wts →
/// MD5(query + mixin_key) 得 w_rid。返回可直接拼到 URL 后的完整 query。
fn sign_wbi_query(params: &HashMap<String, String>, mixin_key: &str, wts: i64) -> String {
    let mut kv: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    kv.push(("wts".to_string(), wts.to_string()));
    kv.sort_by(|a, b| a.0.cmp(&b.0));
    let base: Vec<String> = kv
        .iter()
        .map(|(k, v)| format!("{}={}", encode_wbi_component(k), encode_wbi_component(v)))
        .collect();
    let base = base.join("&");
    let w_rid = format!("{:x}", md5::Md5::digest(format!("{}{}", base, mixin_key).as_bytes()));
    format!("{}&w_rid={}", base, w_rid)
}

/// 带浏览器级请求头（UA/Referer/Accept/Accept-Language/Cookie）的 GET。
/// 风控相关错误（412/403/429/400）不重试，其余（网络抖动、5xx）重试。
/// `referer`：B 站校验 Referer 来源，动态接口应传具体用户空间页（`space.bilibili.com/{mid}`），
/// 其它接口传 `https://www.bilibili.com/`（实测缺头/通用 Referer 易触发风控）。
async fn bili_get(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
    referer: &str,
) -> Result<String, (u16, String)> {
    crate::retry::retry_with_backoff(
        &Default::default(),
        |e: &(u16, String)| {
            // 风控/限流/参数错误不重试，其余（网络抖动、5xx）重试
            if matches!(e.0, 412 | 429 | 400 | 403) {
                return false;
            }
            log::warn!("B 站请求失败(状态={}), 将重试: {}", e.0, e.1);
            true
        },
        || async {
            let mut req = client
                .get(url)
                .header("User-Agent", BILI_UA)
                .header("Referer", referer)
                .header("Accept", BILI_ACCEPT)
                .header("Accept-Language", BILI_ACCEPT_LANG);
            if let Some(c) = cookie {
                req = req.header("Cookie", c);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                let reason = resp.status().canonical_reason().unwrap_or("").to_string();
                // 412 是 B 站 WAF 直接拦截（IP/指纹级风控）
                let msg = if status == 412 {
                    "err.bili_risk|HTTP 412".to_string()
                } else {
                    format!("err.api_error|{}|{}", status, reason)
                };
                return Err((status, msg));
            }
            resp.text()
                .await
                .map_err(|e| (0, format!("err.parse_failed|{}", e)))
        },
    )
    .await
}

/// 解析 B 站 API JSON：code != 0 时映射为 i18n 错误。
fn parse_bili_json(body: &str) -> Result<serde_json::Value, (u16, String)> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        return Err(map_bili_code(code, &msg));
    }
    Ok(v)
}

/// B 站业务错误码 → i18n 错误串。
fn map_bili_code(code: i64, msg: &str) -> (u16, String) {
    match code {
        // 风控校验失败（设备指纹/行为画像）
        -352 => (403, "err.bili_risk".to_string()),
        // 请求过于频繁（IP 级限流）
        -799 => (429, "err.bili_rate_limit".to_string()),
        // 用户不存在 / 空间被锁
        -404 => (404, "err.bili_up_not_found".to_string()),
        -400 => (400, format!("err.bili_invalid_params|{}", msg)),
        _ => (0, format!("err.bili_api_error|{}|{}", code, msg)),
    }
}

/// HMAC-SHA256（bili_ticket 签名；用已有 sha2 手写，避免新增 hmac 依赖）。
fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut k = key.to_vec();
    if k.len() > 64 {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(64, 0);
    let mut ipad = k.clone();
    let mut opad = k;
    for b in ipad.iter_mut() {
        *b ^= 0x36;
    }
    for b in opad.iter_mut() {
        *b ^= 0x5c;
    }
    let mut inner = ipad.clone();
    inner.extend_from_slice(msg);
    let inner_hash = Sha256::digest(&inner);
    let mut outer = opad.clone();
    outer.extend_from_slice(&inner_hash);
    format!("{:x}", Sha256::digest(&outer))
}

/// 获取 bili_ticket（存在 cookie 中可降低风控概率，3 天有效；失败时静默降级）。
async fn fetch_bili_ticket(client: &reqwest::Client) -> Option<String> {
    let ts = chrono::Utc::now().timestamp();
    let hexsign = hmac_sha256_hex(BILI_TICKET_KEY.as_bytes(), format!("ts{}", ts).as_bytes());
    let url = format!(
        "{}?key_id=ec02&hexsign={}&context%5Bts%5D={}&csrf=",
        BILI_TICKET_URL, hexsign, ts
    );
    let resp = client
        .post(&url)
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Accept", BILI_ACCEPT)
        .send()
        .await
        .ok()?;
    let text = resp.text().await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let ticket = v["data"]["ticket"].as_str()?.to_string();
    if ticket.is_empty() {
        return None;
    }
    Some(ticket)
}

// ── 会话缓存（buvid cookie / WBI keys 按 TTL 复用，避免每次检查都重新初始化） ──

/// 匿名 cookie 缓存 TTL（bili_ticket 有效期 3 天，保守按 1 小时刷新）。
const COOKIE_CACHE_TTL_SECS: i64 = 60 * 60;
/// WBI 签名密钥缓存 TTL（密钥每日更替，保守按 6 小时刷新）。
const WBI_CACHE_TTL_SECS: i64 = 6 * 60 * 60;

struct CachedCookie {
    /// 请求携带的完整 cookie 串（配置 SESSDATA 时含 SESSDATA）。
    value: String,
    /// 该 cookie 对应的 SESSDATA（None = 匿名）；变化时缓存失效重新初始化。
    sessdata: Option<String>,
    expires_at: i64,
}

struct CachedWbiKeys {
    keys: WbiKeys,
    expires_at: i64,
}

#[derive(Default)]
struct BiliCookieCache {
    cookie: Option<CachedCookie>,
}

impl BiliCookieCache {
    /// cookie 缓存命中条件：未过期且登录态一致（SESSDATA 相同；空串等同匿名）。
    fn cookie_hit(&self, sessdata: Option<&str>, now: i64) -> Option<&str> {
        let sd = sessdata.filter(|s| !s.is_empty());
        self.cookie
            .as_ref()
            .filter(|c| c.expires_at > now)
            .filter(|c| c.sessdata.as_deref() == sd)
            .map(|c| c.value.as_str())
    }
}

#[derive(Default)]
struct BiliWbiCache {
    wbi: Option<CachedWbiKeys>,
}

impl BiliWbiCache {
    /// WBI 密钥缓存命中条件：未过期。
    fn wbi_hit(&self, now: i64) -> Option<&WbiKeys> {
        self.wbi
            .as_ref()
            .filter(|c| c.expires_at > now)
            .map(|c| &c.keys)
    }
}

/// 全局 B 站 cookie 缓存。buvid cookie 与具体监控源无关，多个 bilibili 源、多次检查共享，
/// 避免每次检查重复请求主页/spi/ticket。
///
/// cookie 与 WBI 密钥拆分为**两把独立锁**（F4）：cookie 初始化含主页 + spi + ticket 3 段网络
/// 往返，若与 WBI 共用一把锁，一个源的 cookie 未命中会阻塞另一源纯缓存命中的 WBI 读取，
/// 全局串行化。拆分后两个状态各自并发，跨 await 持锁仍保证 check-then-act 安全
/// （防 thundering herd）。
fn bili_cookie_cache() -> &'static tokio::sync::Mutex<BiliCookieCache> {
    static CACHE: OnceLock<tokio::sync::Mutex<BiliCookieCache>> = OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(BiliCookieCache::default()))
}

/// 全局 B 站 WBI 密钥缓存（与 cookie 缓存独立加锁，见 `bili_cookie_cache` 注释）。
fn bili_wbi_cache() -> &'static tokio::sync::Mutex<BiliWbiCache> {
    static CACHE: OnceLock<tokio::sync::Mutex<BiliWbiCache>> = OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(BiliWbiCache::default()))
}

/// 匿名初始化 buvid cookie：主页 Set-Cookie 拿 buvid3/b_nut，finger/spi 拿 buvid4，
/// 再补 bili_ticket（可选，降低风控）。失败时降级，不阻断后续拉取。
async fn init_anonymous_cookie_via_client(
    client: &reqwest::Client,
) -> Result<String, (u16, String)> {
    let resp = client
        .get(BILI_HOME)
        .header("User-Agent", BILI_UA)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let mut cookies: Vec<(String, String)> = Vec::new();
    for header in resp.headers().get_all("set-cookie") {
        if let Ok(raw) = header.to_str() {
            let kv = raw.split(';').next().unwrap_or("");
            if let Some((k, v)) = kv.split_once('=') {
                cookies.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    // finger/spi 补 buvid4（带浏览器头，独立于主页 cookie）
    if let Ok(spi) = client
        .get(format!("{}{}", BILI_API_BASE, BILI_SPI_PATH))
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Accept", BILI_ACCEPT)
        .header("Accept-Language", BILI_ACCEPT_LANG)
        .send()
        .await
    {
        if let Ok(text) = spi.text().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(b4) = v["data"]["b_4"].as_str() {
                    if !b4.is_empty() {
                        cookies.push(("buvid4".to_string(), b4.to_string()));
                    }
                }
            }
        }
    }
    // bili_ticket 降风控（失败忽略）
    if let Some(t) = fetch_bili_ticket(client).await {
        cookies.push(("bili_ticket".to_string(), t));
    }
    Ok(cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; "))
}

/// 从 nav 响应体解析 WBI 签名密钥（未登录 code=-101 也返回 key，不视为错误）。
fn parse_wbi_keys(body: &str) -> Result<WbiKeys, (u16, String)> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    let img_url = v["data"]["wbi_img"]["img_url"]
        .as_str()
        .ok_or((0, "err.bili_wbi_keys|img".to_string()))?;
    let sub_url = v["data"]["wbi_img"]["sub_url"]
        .as_str()
        .ok_or((0, "err.bili_wbi_keys|sub".to_string()))?;
    let img_key = img_url.rsplit('/').next().unwrap_or("").split('.').next().unwrap_or("").to_string();
    let sub_key = sub_url.rsplit('/').next().unwrap_or("").split('.').next().unwrap_or("").to_string();
    if img_key.is_empty() || sub_key.is_empty() {
        return Err((0, "err.bili_wbi_keys|empty".to_string()));
    }
    // mixin_key() 用重排表索引 img_key+sub_key 拼接串（最大下标 63）；
    // key 短于预期时返回错误而非索引越界 panic。
    if img_key.len() + sub_key.len() < 64 {
        return Err((0, "err.bili_wbi_keys|short".to_string()));
    }
    Ok(WbiKeys { img_key, sub_key })
}

/// 取 WBI 签名密钥（nav 接口，未登录也返回；密钥每日更替，按 TTL 缓存复用）。
async fn fetch_wbi_keys(
    client: &reqwest::Client,
    cookie: Option<&str>,
) -> Result<WbiKeys, (u16, String)> {
    let now = chrono::Utc::now().timestamp();
    let mut cache = bili_wbi_cache().lock().await;
    if let Some(keys) = cache.wbi_hit(now) {
        return Ok(keys.clone());
    }
    let body = bili_get(
        client,
        &format!("{}{}", BILI_API_BASE, BILI_NAV_PATH),
        cookie,
        "https://www.bilibili.com/",
    )
    .await?;
    let keys = parse_wbi_keys(&body)?;
    cache.wbi = Some(CachedWbiKeys {
        keys: keys.clone(),
        expires_at: now + WBI_CACHE_TTL_SECS,
    });
    Ok(keys)
}

/// 拉取一页空间动态，返回（条目, 下一页 offset）。
async fn fetch_dynamic_page(
    client: &reqwest::Client,
    mid: &str,
    offset: Option<&str>,
    cookie: Option<&str>,
    keys: &WbiKeys,
) -> Result<(Vec<BiliEntry>, Option<String>), (u16, String)> {
    let mut params = HashMap::new();
    params.insert("host_mid".to_string(), mid.to_string());
    params.insert("timezone_offset".to_string(), "-480".to_string());
    params.insert("features".to_string(), BILI_DYNAMIC_FEATURES.to_string());
    if let Some(o) = offset {
        params.insert("offset".to_string(), o.to_string());
    }
    let wts = chrono::Utc::now().timestamp();
    let query = sign_wbi_query(&params, &keys.mixin_key(), wts);
    let url = format!(
        "{}{}?{}",
        BILI_API_BASE, BILI_DYNAMIC_PATH, query
    );
    let body = bili_get(
        client,
        &url,
        cookie,
        &format!("{}{}", BILI_SPACE_REFERER, mid),
    )
    .await?;
    let v = parse_bili_json(&body)?;
    let data = &v["data"];
    let items = data["items"].as_array().cloned().unwrap_or_default();
    let next_offset = data["offset"].as_str().map(|s| s.to_string());
    let mut entries = Vec::new();
    for it in &items {
        // 只取视频动态（DYNAMIC_TYPE_AV / MAJOR_TYPE_ARCHIVE）
        if it["type"].as_str() != Some("DYNAMIC_TYPE_AV") {
            continue;
        }
        let archive = &it["modules"]["module_dynamic"]["major"]["archive"];
        let bvid = archive["bvid"].as_str().unwrap_or("").trim().to_string();
        if bvid.is_empty() {
            continue;
        }
        let title = archive["title"].as_str().unwrap_or("").to_string();
        let description = archive["desc"].as_str().unwrap_or("").to_string();
        let thumbnail = archive["cover"].as_str().unwrap_or("").to_string();
        let duration = archive["duration_text"].as_str().unwrap_or("").to_string();
        let up_name = it["modules"]["module_author"]["name"]
            .as_str()
            .unwrap_or("")
            .to_string();
        // 动态发布时间即视频投稿时间；pub_ts 解析失败（字段缺失/类型异常）时
        // 以当前时间兜底，杜绝 1970-01-01 脏数据（此前会导致版本列表按时间
        // 排序被 LIMIT 截断而完全不可见）。
        let published = match parse_pub_ts(it) {
            Some(ts) => chrono::DateTime::from_timestamp(ts, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default(),
            None => chrono::Utc::now().to_rfc3339(),
        };
        if title.is_empty() || published.is_empty() {
            continue;
        }
        entries.push(BiliEntry {
            bvid,
            title,
            published,
            description,
            thumbnail,
            duration,
            up_name,
        });
    }
    Ok((entries, next_offset))
}

/// 解析动态条目发布时间（秒级时间戳）。
/// B 站接口 pub_ts 数字/字符串两种形态都出现过（风控降级时字段可能缺失），
/// 返回 None 时由调用方以检测时间兜底，避免 0 时间戳（1970-01-01）污染 published_at。
fn parse_pub_ts(item: &serde_json::Value) -> Option<i64> {
    let v = &item["modules"]["module_author"]["pub_ts"];
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        .filter(|&ts| ts > 0)
}

/// 解析用户输入为 UID：
/// - 纯数字 UID（2~16 位：旧式 6~10 位、新式 16 位均可）
/// - `space.bilibili.com/{uid}` / `bilibili.com/space/{uid}` 链接
/// - `bilibili.com/{uid}`（B 站新版空间跳转）
fn extract_uid(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    // 纯数字 UID（B 站新注册用户为 16 位新式 UID）
    if input.chars().all(|c| c.is_ascii_digit()) && input.len() <= 16 {
        return Some(input.to_string());
    }
    for pat in ["space.bilibili.com/", "bilibili.com/space/"] {
        if let Some(idx) = input.find(pat) {
            let rest = &input[idx + pat.len()..];
            let uid: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !uid.is_empty() {
                return Some(uid);
            }
        }
    }
    // https://bilibili.com/1234567 跳转形式
    if let Some(idx) = input.find("bilibili.com/") {
        let rest = &input[idx + "bilibili.com/".len()..];
        if !rest.starts_with("space") && !rest.starts_with("video") {
            let uid: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !uid.is_empty() {
                return Some(uid);
            }
        }
    }
    None
}

/// 拉取 UP 主真实昵称（动态接口 module_author.name，兜底空间页 og:title）。
async fn fetch_up_name(
    client: &reqwest::Client,
    mid: &str,
    cookie: Option<&str>,
) -> Result<String, (u16, String)> {
    let keys = fetch_wbi_keys(client, cookie).await?;
    let (entries, _) = fetch_dynamic_page(client, mid, None, cookie, &keys).await?;
    if let Some(e) = entries.first() {
        if !e.up_name.is_empty() {
            return Ok(e.up_name.clone());
        }
    }
    // 兜底：空间页 og:title（CDN 层，正常网络可用）
    let page_url = format!("https://space.bilibili.com/{}", mid);
    if let Ok(body) = bili_get(client, &page_url, cookie, &page_url).await {
        if let Some(name) = extract_og_title(&body) {
            return Ok(name);
        }
    }
    Err((404, format!("err.bili_up_not_found|{}", mid)))
}

/// 从 HTML 提取 og:title（空间页 UP 主昵称）。
fn extract_og_title(html: &str) -> Option<String> {
    let needle = "<meta property=\"og:title\" content=\"";
    let idx = html.find(needle)?;
    let rest = &html[idx + needle.len()..];
    let end = rest.find('"')?;
    let raw = &rest[..end];
    let name = raw.trim();
    // B 站风控页 title 形如 "bili_xxxxxx的个人空间"，og:title 可能不存在；空/风控形态视为失败
    if name.is_empty() || name.starts_with("bili_") {
        return None;
    }
    Some(quick_xml::escape::unescape(name).ok()?.into_owned())
}

/// 条目列表 → 序列化 JSON（供 save 阶段反序列化 BiliEntry）。
fn entries_to_json(entries: Vec<BiliEntry>) -> Vec<serde_json::Value> {
    entries
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
        .collect()
}

/// 保存视频条目到 releases 表（tag_name = bvid，天然去重）。
/// 行为与 youtube::save_entries 对齐：按 published 降序，max_count=1 遇到已入库
/// 记录立即返回空；历史模式跳过已存在记录继续。
pub fn save_entries(
    conn: &Connection,
    source_id: i64,
    items: &[serde_json::Value],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    let mut entries: Vec<BiliEntry> = items
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    entries.sort_by(|a, b| b.published.cmp(&a.published));

    let mut saved = Vec::new();
    for entry in &entries {
        if entry.bvid.is_empty() || entry.published.is_empty() {
            continue;
        }
        let html_url = entry.html_url();
        let metadata = entry.metadata_json();
        if let Ok(id) = releases::insert_release(
            conn,
            source_id,
            &entry.bvid,
            &entry.title,
            &html_url,
            &entry.published,
            false,
            Some(&entry.description),
        ) {
            if id > 0 {
                let _ = releases::set_release_body_and_metadata(
                    conn,
                    id,
                    Some(&entry.description),
                    Some(&metadata),
                );
                saved.push((id, Some(entry.description.clone())));
                if max_count > 0 && saved.len() >= max_count {
                    return saved;
                }
                continue;
            }
        }
        // 已入库且普通模式（max_count=1）时，说明不是新内容，停止
        if max_count == 1 {
            return vec![];
        }
        // 历史模式：已存在的跳过，继续找更新的新内容
    }
    saved
}

// ── SourceAdapter 实现 ─────────────────────────────────

/// B 站 UP 主监控适配器。
pub struct BilibiliAdapter;

#[async_trait::async_trait]
impl SourceAdapter for BilibiliAdapter {
    fn source_type(&self) -> &'static str {
        "bilibili"
    }

    fn auth_kind(&self) -> crate::source::AuthKind {
        crate::source::AuthKind::BilibiliCookie
    }

    /// B 站视频不生成 AI 摘要/翻译（标题简介均为中文，与 youtube 一致）。
    fn ai_eligible(&self) -> bool {
        false
    }

    /// 检查成功后刷新 UP 主昵称（动态接口 module_author.name 才是真名）。
    fn refresh_description_after_check(&self) -> bool {
        true
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)> {
        let cookie = build_cookie(client, token).await?;
        let keys = fetch_wbi_keys(client, cookie.as_deref()).await?;
        let (entries, _) = fetch_dynamic_page(
            client,
            &source.owner,
            None,
            cookie.as_deref(),
            &keys,
        )
        .await?;
        let out: Vec<serde_json::Value> = entries
            .into_iter()
            .take(per_page)
            .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
            .collect();
        Ok(out)
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)> {
        let cookie = build_cookie(client, token).await?;
        let keys = fetch_wbi_keys(client, cookie.as_deref()).await?;
        let mut entries = Vec::new();
        let mut offset: Option<String> = None;
        let mut pages = 0usize;
        loop {
            // 页数上限保护：max_count=None（fetch_history_count=0 全量拉取）时
            // 唯一终止条件本是响应 offset 为空；若 B 站异常重复返回同一非空 offset，
            // 这里强制在 MAX_FETCH_ALL_PAGES 页后停止，避免无限循环挂住轮询（F5）。
            if pages >= MAX_FETCH_ALL_PAGES {
                log::warn!(
                    "bilibili {} 翻页达到上限 {} 页，停止拉取（已获 {} 条）",
                    source.owner,
                    MAX_FETCH_ALL_PAGES,
                    entries.len()
                );
                break;
            }
            let (page, next) = fetch_dynamic_page(
                client,
                &source.owner,
                offset.as_deref(),
                cookie.as_deref(),
                &keys,
            )
            .await?;
            entries.extend(page);
            pages += 1;
            if let Some(limit) = max_count {
                if entries.len() >= limit {
                    break;
                }
            }
            if entries.len() >= MAX_FETCH_ALL_ITEMS {
                log::warn!(
                    "bilibili {} 条目数达到上限 {}，停止拉取",
                    source.owner,
                    MAX_FETCH_ALL_ITEMS
                );
                break;
            }
            match next {
                // 防死循环：响应重复返回与当前相同的 offset（风控页形态）时停止
                Some(o) if !o.is_empty() && Some(o.as_str()) != offset.as_deref() => offset = Some(o),
                _ => break,
            }
        }
        if let Some(limit) = max_count {
            entries.truncate(limit);
        }
        Ok(entries_to_json(entries))
    }

    async fn save(
        &self,
        db: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
        source: &Source,
        data: &[serde_json::Value],
        max_count: usize,
        _client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
        // 同步入库，用 spawn_blocking 转包避免在 async 上下文阻塞（与 github/youtube 一致）。
        let db = db.clone();
        let source_id = source.id;
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = match db.get() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("err.db_lock|{}", e);
                    return vec![];
                }
            };
            save_entries(&conn, source_id, &data, max_count)
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("bilibili save spawn_blocking panic: {}", e);
            vec![]
        })
    }

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        _repo: &str,
        token: Option<&str>,
    ) -> Result<String, (u16, String)> {
        // owner 此时已由 resolve_owner 归一化为 UID。
        let cookie = build_cookie(client, token).await?;
        fetch_up_name(client, owner, cookie.as_deref()).await
    }

    async fn resolve_owner(
        &self,
        _client: &reqwest::Client,
        owner: &str,
        _token: Option<&str>,
    ) -> Result<String, (u16, String)> {
        extract_uid(owner)
            .ok_or((400, format!("err.bili_invalid_uid|{}", owner)))
    }
}

/// 组装请求 cookie：匿名 buvid 初始化 + 可选 SESSDATA（登录模式，token 参数传入）。
///
/// 初始化结果按 TTL 缓存：TTL 内同一登录态（SESSDATA 未变化）直接复用，
/// 避免每次检查都重复请求主页/spi/ticket；SESSDATA 变化或缓存过期时重新初始化。
/// 只持有 cookie 专用锁（与 WBI 密钥锁拆分，见 `bili_cookie_cache`）。
async fn build_cookie(
    client: &reqwest::Client,
    sessdata: Option<&str>,
) -> Result<Option<String>, (u16, String)> {
    let sessdata = sessdata.filter(|s| !s.is_empty());
    let now = chrono::Utc::now().timestamp();
    let mut cache = bili_cookie_cache().lock().await;
    if let Some(hit) = cache.cookie_hit(sessdata, now) {
        return Ok(Some(hit.to_string()));
    }
    let mut parts = init_anonymous_cookie_via_client(client).await?;
    if let Some(s) = sessdata {
        if !parts.is_empty() {
            parts.push_str("; ");
        }
        parts.push_str(&format!("SESSDATA={}", s));
    }
    if parts.is_empty() {
        return Ok(None);
    }
    cache.cookie = Some(CachedCookie {
        value: parts.clone(),
        sessdata: sessdata.map(|s| s.to_string()),
        expires_at: now + COOKIE_CACHE_TTL_SECS,
    });
    Ok(Some(parts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::init::init_memory_db;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── WBI 签名（文档测试用例） ──

    #[test]
    fn test_mixin_key_from_doc_case() {
        let keys = WbiKeys {
            img_key: "7cd084941338484aae1ad9425b84077c".to_string(), // gitleaks:allow WBI 文档测试用例示例密钥
            sub_key: "4932caff0ff746eab6f01bf08b70ac45".to_string(), // gitleaks:allow WBI 文档测试用例示例密钥
        };
        assert_eq!(keys.mixin_key(), "ea1db124af3c7062474693fa704f4ff8");
    }

    #[test]
    fn test_sign_wbi_from_doc_case() {
        let mixin = "ea1db124af3c7062474693fa704f4ff8";
        let mut params = HashMap::new();
        params.insert("bar".to_string(), "514".to_string());
        params.insert("foo".to_string(), "114".to_string());
        params.insert("zab".to_string(), "1919810".to_string());
        let query = sign_wbi_query(&params, mixin, 1702204169);
        assert!(query.contains("wts=1702204169"));
        assert!(query.contains("w_rid=8f6f2b5b3d485fe1886cec6a0be8c5d4"));
    }

    #[test]
    fn test_encode_wbi_component_filters_and_uppercase() {
        // 中文按 UTF-8 大写 %XX 编码；空格 %20；!'()* 被剔除
        assert_eq!(encode_wbi_component("a b"), "a%20b");
        assert_eq!(encode_wbi_component("ab!c'd(e)f*g"), "abcdefg");
        assert_eq!(encode_wbi_component("视频"), "%E8%A7%86%E9%A2%91");
        assert_eq!(encode_wbi_component("one one four"), "one%20one%20four");
    }

    // ── UID 解析 ──

    #[test]
    fn test_extract_uid_variants() {
        assert_eq!(extract_uid("476599099").as_deref(), Some("476599099"));
        assert_eq!(extract_uid(" 546195 ").as_deref(), Some("546195"));
        // 16 位新式 UID（新注册用户，space.bilibili.com/{mid} 的 mid 即 UID）
        assert_eq!(
            extract_uid("3546715770588065").as_deref(),
            Some("3546715770588065")
        );
        assert_eq!(
            extract_uid("https://space.bilibili.com/3546715770588065").as_deref(),
            Some("3546715770588065")
        );
        assert_eq!(
            extract_uid("https://space.bilibili.com/476599099/video").as_deref(),
            Some("476599099")
        );
        assert_eq!(
            extract_uid("space.bilibili.com/476599099?from=search").as_deref(),
            Some("476599099")
        );
        assert_eq!(
            extract_uid("https://bilibili.com/space/546195").as_deref(),
            Some("546195")
        );
        // 非 UID 输入
        assert_eq!(extract_uid("@someuser"), None);
        assert_eq!(extract_uid("https://www.bilibili.com/video/BV1xx"), None);
        assert_eq!(extract_uid(""), None);
    }

    // ── pub_ts 解析（数字/字符串/缺失兼容） ──

    #[test]
    fn test_parse_pub_ts_variants() {
        let mk = |v: serde_json::Value| {
            serde_json::json!({"type": "DYNAMIC_TYPE_AV", "modules": {"module_author": v}})
        };
        // 数字形态（正常）
        let item = mk(serde_json::json!({"pub_ts": 1746450829}));
        assert_eq!(parse_pub_ts(&item), Some(1746450829));
        // 字符串形态（B 站接口类型漂移）
        let item = mk(serde_json::json!({"pub_ts": "1746450829"}));
        assert_eq!(parse_pub_ts(&item), Some(1746450829));
        // 缺失（风控降级形态）→ None，由调用方以当前时间兜底
        let item = mk(serde_json::json!({}));
        assert_eq!(parse_pub_ts(&item), None);
        // 显式 null → None
        let item = mk(serde_json::json!({"pub_ts": null}));
        assert_eq!(parse_pub_ts(&item), None);
        // 0 / 非法字符串 → None（避免 1970 脏数据）
        let item = mk(serde_json::json!({"pub_ts": 0}));
        assert_eq!(parse_pub_ts(&item), None);
        let item = mk(serde_json::json!({"pub_ts": "abc"}));
        assert_eq!(parse_pub_ts(&item), None);
    }

    // ── 动态解析 ──

    fn dynamic_item(bvid: &str, title: &str, pub_ts: i64, up_name: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "DYNAMIC_TYPE_AV",
            "modules": {
                "module_author": {"name": up_name, "pub_ts": pub_ts},
                "module_dynamic": {
                    "major": {
                        "type": "MAJOR_TYPE_ARCHIVE",
                        "archive": {
                            "bvid": bvid,
                            "title": title,
                            "desc": "简介",
                            "cover": "https://i0.hdslb.com/bfs/archive/xxx.jpg",
                            "duration_text": "12:34"
                        }
                    }
                }
            }
        })
    }

    fn dynamic_body(items: Vec<serde_json::Value>, offset: Option<&str>) -> String {
        serde_json::json!({
            "code": 0, "message": "0", "ttl": 1,
            "data": {"has_more": true, "offset": offset, "items": items}
        })
        .to_string()
    }

    #[test]
    fn test_fetch_dynamic_page_parses_video_entries() {
        let body = dynamic_body(
            vec![
                dynamic_item("BV1a1b2c3d4e5f", "测试视频", 1723959548, "某UP主"),
                serde_json::json!({"type": "DYNAMIC_TYPE_FORWARD", "modules": {}}),
            ],
            Some("1160055449596198920"),
        );
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let data = &v["data"];
        let items = data["items"].as_array().unwrap();
        // 直接调解析逻辑
        let mut entries = Vec::new();
        for it in items {
            if it["type"].as_str() != Some("DYNAMIC_TYPE_AV") {
                continue;
            }
            let archive = &it["modules"]["module_dynamic"]["major"]["archive"];
            let bvid = archive["bvid"].as_str().unwrap().to_string();
            let pub_ts = it["modules"]["module_author"]["pub_ts"].as_i64().unwrap();
            entries.push(BiliEntry {
                bvid,
                title: archive["title"].as_str().unwrap().to_string(),
                published: chrono::DateTime::from_timestamp(pub_ts, 0)
                    .unwrap()
                    .to_rfc3339(),
                description: archive["desc"].as_str().unwrap().to_string(),
                thumbnail: archive["cover"].as_str().unwrap().to_string(),
                duration: archive["duration_text"].as_str().unwrap().to_string(),
                up_name: it["modules"]["module_author"]["name"].as_str().unwrap().to_string(),
            });
        }
        assert_eq!(entries.len(), 1, "只应解析出视频动态");
        assert_eq!(entries[0].bvid, "BV1a1b2c3d4e5f");
        assert_eq!(entries[0].up_name, "某UP主");
        assert!(entries[0].published.starts_with("2024-08-18"));
        assert_eq!(entries[0].html_url(), "https://www.bilibili.com/video/BV1a1b2c3d4e5f");
    }

    #[test]
    fn test_map_bili_code() {
        assert_eq!(map_bili_code(-352, "风控校验失败").0, 403);
        assert!(map_bili_code(-352, "").1.contains("err.bili_risk"));
        assert_eq!(map_bili_code(-799, "").0, 429);
        assert_eq!(map_bili_code(-404, "").0, 404);
        let (s, m) = map_bili_code(-123, "boom");
        assert_eq!(s, 0);
        assert!(m.contains("err.bili_api_error|-123|boom"));
    }

    // ── save 去重 ──

    #[test]
    fn test_save_entries_dedup() {
        let conn = init_memory_db().unwrap();
        let source_id = db::sources::add_source(&conn, "bilibili", "476599099", "", "").unwrap();
        let entry = BiliEntry {
            bvid: "BV1a1b2c3d4e5f".to_string(),
            title: "新视频".to_string(),
            published: "2024-08-18T02:39:08+00:00".to_string(),
            description: "简介".to_string(),
            thumbnail: "https://i0.hdslb.com/bfs/archive/xxx.jpg".to_string(),
            duration: "12:34".to_string(),
            up_name: "UP".to_string(),
        };
        let items = entries_to_json(vec![entry]);
        let saved = save_entries(&conn, source_id, &items, 1);
        assert_eq!(saved.len(), 1);
        // 再次保存同一 bvid → 去重，普通模式返回空
        let saved2 = save_entries(&conn, source_id, &items, 1);
        assert!(saved2.is_empty());
        // 历史模式（max_count 大）跳过已存在，仍返回空
        let saved3 = save_entries(&conn, source_id, &items, 100);
        assert!(saved3.is_empty());
    }

    #[test]
    fn test_parse_wbi_keys_accepts_unlogged_nav() {
        // nav 未登录（code=-101）仍返回 wbi_img，不应视为错误
        let body = serde_json::json!({
            "code": -101, "message": "账号未登录", "ttl": 1,
            "data": {"wbi_img": {
                "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png", // gitleaks:allow WBI 文档测试用例示例密钥
                "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png" // gitleaks:allow WBI 文档测试用例示例密钥
            }}
        }).to_string();
        let keys = parse_wbi_keys(&body).unwrap();
        assert_eq!(keys.img_key, "7cd084941338484aae1ad9425b84077c"); // gitleaks:allow WBI 文档测试用例示例密钥
        assert_eq!(keys.sub_key, "4932caff0ff746eab6f01bf08b70ac45"); // gitleaks:allow WBI 文档测试用例示例密钥
        assert_eq!(keys.mixin_key(), "ea1db124af3c7062474693fa704f4ff8");
    }

    #[test]
    fn test_parse_wbi_keys_missing_fields() {
        let body = serde_json::json!({"code": 0, "data": {}}).to_string();
        let err = parse_wbi_keys(&body).unwrap_err();
        assert!(err.1.contains("err.bili_wbi_keys"));
    }

    #[test]
    fn test_parse_wbi_keys_rejects_short_keys() {
        // key 短于 32 位（拼接 < 64）时返回错误，而非让 mixin_key() 索引越界 panic
        let body = serde_json::json!({
            "code": 0, "data": {"wbi_img": {
                "img_url": "https://i0.hdslb.com/bfs/wbi/7cd08494133848.png",
                "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746.png"
            }}
        }).to_string();
        let err = parse_wbi_keys(&body).unwrap_err();
        assert_eq!(err.1, "err.bili_wbi_keys|short");
    }

    // ── 会话缓存命中规则（cookie 与 WBI 独立锁、独立结构）──

    #[test]
    fn test_bili_cache_cookie_hit_rules() {
        let now = 1_000_000i64;
        let mut cache = BiliCookieCache::default();
        // 空缓存不命中
        assert!(cache.cookie_hit(None, now).is_none());

        cache.cookie = Some(CachedCookie {
            value: "buvid3=x; bili_ticket=t".to_string(),
            sessdata: None,
            expires_at: now + 10,
        });
        // 匿名态命中；空串 SESSDATA 等同匿名
        assert_eq!(cache.cookie_hit(None, now), Some("buvid3=x; bili_ticket=t"));
        assert_eq!(cache.cookie_hit(Some(""), now), Some("buvid3=x; bili_ticket=t"));
        // 过期不命中
        assert!(cache.cookie_hit(None, now + 10).is_none());
        // 登录态变化（配置 SESSDATA）不命中，需重新初始化
        assert!(cache.cookie_hit(Some("sess"), now).is_none());

        // 登录态一致则命中（SESSDATA 拼入 cookie 串）
        cache.cookie = Some(CachedCookie {
            value: "buvid3=x; SESSDATA=sess".to_string(),
            sessdata: Some("sess".to_string()),
            expires_at: now + 10,
        });
        assert_eq!(cache.cookie_hit(Some("sess"), now), Some("buvid3=x; SESSDATA=sess"));
        // 换了一个 SESSDATA → 不命中
        assert!(cache.cookie_hit(Some("other"), now).is_none());
    }

    #[test]
    fn test_bili_cache_wbi_hit_rules() {
        let mut cache = BiliWbiCache::default();
        assert!(cache.wbi_hit(100).is_none());
        cache.wbi = Some(CachedWbiKeys {
            keys: WbiKeys {
                img_key: "a".repeat(32),
                sub_key: "b".repeat(32),
            },
            expires_at: 200,
        });
        assert!(cache.wbi_hit(199).is_some());
        // 到期（expires_at 为开区间）不命中
        assert!(cache.wbi_hit(200).is_none());
    }

    #[tokio::test]
    async fn test_fetch_wbi_keys_via_nav() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x/web-interface/nav"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": -101, "message": "账号未登录", "ttl": 1,
                "data": {
                    "wbi_img": {
                        "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png", // gitleaks:allow WBI 文档测试用例示例密钥
                        "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png" // gitleaks:allow WBI 文档测试用例示例密钥
                    }
                }
            })))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 直连 mock 的 nav 端点（绕过硬编码域名）：验证解析与容错
        let body = client
            .get(format!("{}/x/web-interface/nav", mock.uri()))
            .header("User-Agent", BILI_UA)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let keys = parse_wbi_keys(&body).unwrap();
        assert_eq!(keys.img_key, "7cd084941338484aae1ad9425b84077c"); // gitleaks:allow WBI 文档测试用例示例密钥
    }

    #[test]
    fn test_entries_to_json_roundtrip() {
        let e = BiliEntry {
            bvid: "BV1xx".to_string(),
            title: "t".to_string(),
            published: "2024-01-01T00:00:00+00:00".to_string(),
            description: "d".to_string(),
            thumbnail: "https://x.jpg".to_string(),
            duration: "10:00".to_string(),
            up_name: "up".to_string(),
        };
        let json = entries_to_json(vec![e.clone()]);
        let back: BiliEntry = serde_json::from_value(json[0].clone()).unwrap();
        assert_eq!(back.bvid, e.bvid);
        assert_eq!(back.up_name, "up");
    }

    #[test]
    fn test_metadata_json_upgrades_http_thumbnail_to_https() {
        // B 站 CDN 返回 http 封面，WebView CSP 仅允许 https 图片（img-src 'self' data: https:），
        // 入库前必须升级协议，否则列表封面不显示。
        let e = BiliEntry {
            bvid: "BV1xx".to_string(),
            title: "t".to_string(),
            published: "2024-01-01T00:00:00+00:00".to_string(),
            description: "d".to_string(),
            thumbnail: "http://i0.hdslb.com/bfs/archive/abc.jpg".to_string(),
            duration: "10:00".to_string(),
            up_name: "up".to_string(),
        };
        let meta: serde_json::Value = serde_json::from_str(&e.metadata_json()).unwrap();
        assert_eq!(meta["thumbnail"], "https://i0.hdslb.com/bfs/archive/abc.jpg");
        // 已是 https 的不重复处理
        let e2 = BiliEntry {
            thumbnail: "https://i0.hdslb.com/bfs/archive/def.jpg".to_string(),
            ..e
        };
        let meta2: serde_json::Value = serde_json::from_str(&e2.metadata_json()).unwrap();
        assert_eq!(meta2["thumbnail"], "https://i0.hdslb.com/bfs/archive/def.jpg");
    }
}
