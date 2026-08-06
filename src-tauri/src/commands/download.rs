use crate::db::settings::{get_setting, KEY_PROXY_MODE, KEY_PROXY_URL};
use crate::http;
use crate::types::AppState;

// 剪贴板图片等场景的单次下载上限：25MB
const MAX_DOWNLOAD_BYTES: usize = 25 * 1024 * 1024;

/// 下载任意 http(s) URL 的原始字节并返回给前端。
/// 前端复制图片时走 Rust 端下载：绕过 webview CORS 限制，并自动继承应用的代理设置。
/// 返回 `Vec<u8>`，IPC 序列化为 number[]。
#[tauri::command]
pub async fn fetch_url_bytes(
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<Vec<u8>, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("err.invalid_url".to_string());
    }
    // SSRF 防护：拒绝私网/回环/链路本地/保留地址（含云元数据 169.254.169.254），
    // 防止通过下载命令探测内网。校验在命令层而非 download_bytes 内（后者被
    // wiremock 测试以 127.0.0.1 直连，且下载函数本身保持通用）。
    http::ensure_public_url(&url).await?;
    let (proxy_url, proxy_mode);
    {
        let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
        proxy_url = get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        proxy_mode = get_setting(&conn, KEY_PROXY_MODE)?.unwrap_or_else(|| {
            if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
        });
        // conn 随作用域结束归还连接池，网络请求期间不占用
    }
    let client = http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        ..Default::default()
    })?;
    http::download_bytes(&client, &url, MAX_DOWNLOAD_BYTES).await
}
