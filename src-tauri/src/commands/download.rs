use crate::db::settings::{get_setting, KEY_PROXY_MODE, KEY_PROXY_URL};
use crate::http;
use crate::types::AppState;

// 剪贴板图片等场景的单次下载上限：25MB
const MAX_DOWNLOAD_BYTES: usize = 25 * 1024 * 1024;

/// 下载任意 http(s) URL 的原始字节并返回给前端。
/// 前端复制图片时走 Rust 端下载：绕过 webview CORS 限制，并自动继承应用的代理设置。
/// 返回 `Vec<u8>`，IPC 序列化为 number[]。
///
/// SSRF 防护（H-2）：下载核心见 `http::fetch_public_bytes`——禁自动重定向、手动
/// 跟随（最多 10 跳）且每跳重新执行 `ensure_public_url`（含云元数据私网拦截）。
#[tauri::command]

#[specta::specta]pub async fn fetch_url_bytes(
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<Vec<u8>, String> {
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
    http::fetch_public_bytes(&client, &url, MAX_DOWNLOAD_BYTES).await
}
