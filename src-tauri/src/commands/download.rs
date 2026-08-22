use crate::db::settings::{get_setting, KEY_PROXY_MODE, KEY_PROXY_URL};
use crate::http;
use crate::types::AppState;

// 剪贴板图片等场景的单次下载上限：25MB
const MAX_DOWNLOAD_BYTES: usize = 25 * 1024 * 1024;

/// 下载任意 http(s) URL 的原始字节并返回给前端。
/// 前端复制图片时走 Rust 端下载：绕过 webview CORS 限制，并自动继承应用的代理设置。
/// 返回 `Vec<u8>`，IPC 序列化为 number[]。
///
/// SSRF 防护（H-2）：禁用 reqwest 自动重定向，改为手动跟随（最多 10 跳），
/// **每一跳都重新执行 `ensure_public_url` 校验**——自动跟随不会对跳转目标
/// 重新校验，恶意服务器可用 302 把请求导向内网（如 169.254.169.254 云元数据）。
#[tauri::command]

#[specta::specta]pub async fn fetch_url_bytes(
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<Vec<u8>, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("err.invalid_url".to_string());
    }
    let (proxy_url, proxy_mode);
    {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        proxy_url = get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        proxy_mode = get_setting(&conn, KEY_PROXY_MODE)?.unwrap_or_else(|| {
            if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
        });
        // conn 随作用域结束归还连接池，网络请求期间不占用
    }
    let client = http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        // 禁自动重定向：跳转目标必须逐跳重新校验（SSRF 防护）
        follow_redirects: false,
        ..Default::default()
    })?;

    // 手动跟随重定向：每跳先校验目标地址，再发请求；
    // 非重定向响应（2xx/4xx/5xx）直接处理，Location 缺失视为普通响应。
    let mut current = url;
    for _ in 0..10 {
        // SSRF 校验在命令层（download_bytes 被 wiremock 测试以 127.0.0.1 直连，
        // 下载函数本身保持通用）：拒绝私网/回环/链路本地/保留地址（含云元数据）。
        http::ensure_public_url(&current).await?;
        let resp = client
            .get(&current)
            .send()
            .await
            .map_err(|e| format!("err.request_failed|{}", e))?;
        if let Some(loc) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
        {
            current = reqwest::Url::parse(&current)
                .map_err(|_| "err.invalid_url".to_string())?
                .join(loc)
                .map_err(|_| "err.invalid_url".to_string())?
                .to_string();
            // 只跟随 http/https 跳转（join 已保证绝对 URL 合法，这里再收紧 scheme）
            if !(current.starts_with("https://") || current.starts_with("http://")) {
                return Err("err.invalid_url".to_string());
            }
            continue;
        }
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("err.download_failed|HTTP {}", status.as_u16()));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_DOWNLOAD_BYTES {
                return Err(format!("err.download_failed|file too large ({} bytes)", len));
            }
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("err.request_failed|{}", e))?;
        if bytes.len() > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "err.download_failed|file too large ({} bytes)",
                bytes.len()
            ));
        }
        return Ok(bytes.to_vec());
    }
    Err("err.download_failed|too many redirects".to_string())
}
