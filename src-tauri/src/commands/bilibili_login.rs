//! B 站一键登录：从登录 WebView 读取 SESSDATA、验证登录态并加密存储。
//!
//! 流程：设置页「一键登录」→ 前端新建 WebviewWindow 加载 B 站登录页 →
//! 用户扫码/账号登录 → 前端轮询本模块命令 → 读到有效 SESSDATA 后加密存库 →
//! 前端关闭登录窗口。
//!
//! 不读取系统浏览器的 cookie（Chrome/Edge cookie 为 DPAPI+AES-GCM 加密且新版
//! 加 app-bound encryption，自动导出脆弱且有安全争议），改为应用内 WebView 登录，
//! SESSDATA 只存在于应用自己的 webview cookie 存储中，登录后即加密入库。

use std::str::FromStr;

use tauri::Manager;

use crate::crypto;
use crate::db::settings::{self, KEY_BILIBILI_COOKIE, KEY_PROXY_URL, KEY_PROXY_MODE};
use crate::types::AppState;

/// 读取 cookie 的匹配域（SESSDATA 域为 .bilibili.com，www 子域可读）。
const BILI_COOKIE_DOMAIN: &str = "https://www.bilibili.com";
/// 验证登录态用（isLogin 字段）。
const BILI_NAV_URL: &str = "https://api.bilibili.com/x/web-interface/nav";
/// B 站对非浏览器 UA 敏感，单请求覆盖 client 级 UA。
const BILI_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 登录窗口固定 label（capabilities/bilibili-login.json 的 windows 白名单也依赖此值；
/// 改动时两处必须同步）。
pub const BILI_LOGIN_WINDOW_LABEL: &str = "bilibili-login";

/// 校验调用方传入的 window_label 必须是登录窗口自身。
///
/// 命令只面向登录窗口：`read_bilibili_login_cookie` 会读取 webview cookie、
/// `close_bilibili_login_window` 会关闭窗口，若接受任意 label，本地内容一旦被
/// 注入即可关任意窗口/读任意窗口 cookie（放大面）。这里在命令层做硬校验，
/// 与 capability 白名单构成纵深防御。
fn require_login_window_label(window_label: &str) -> Result<(), String> {
    if window_label != BILI_LOGIN_WINDOW_LABEL {
        log::warn!(
            "拒绝非登录窗口 label 的 B 站登录命令: {:?}（仅允许 {}",
            window_label,
            BILI_LOGIN_WINDOW_LABEL
        );
        return Err("err.bili_login_window_missing".to_string());
    }
    Ok(())
}

/// 从登录 WebView 读取 SESSDATA，验证有效后加密存储。
///
/// - `Ok(true)`：读取并保存成功（前端应关闭登录窗口）
/// - `Err(err.bili_login_not_logged_in)`：窗口在但尚未登录（前端继续轮询）
/// - `Err(err.bili_login_window_missing)`：窗口已关闭（前端停止轮询）
/// - 其它 Err：读取/验证失败（前端提示）
#[tauri::command]
pub async fn read_bilibili_login_cookie(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    window_label: String,
) -> Result<bool, String> {
    require_login_window_label(&window_label)?;
    let win = app
        .get_webview_window(&window_label)
        .ok_or("err.bili_login_window_missing".to_string())?;

    // 1) 读 webview cookie（同步 API）
    let url = tauri::Url::from_str(BILI_COOKIE_DOMAIN)
        .map_err(|e| format!("err.parse_failed|{}", e))?;
    let cookies = win
        .cookies_for_url(url)
        .map_err(|e| format!("err.bili_login_cookie_read|{}", e))?;
    let sessdata = cookies
        .iter()
        .find(|c| c.name() == "SESSDATA")
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty())
        .ok_or("err.bili_login_not_logged_in".to_string())?;

    // 2) 验证登录态（nav isLogin），避免无效/过期 cookie 入库
    //    先从 settings 取代理配置构建 client（与轮询链路一致）
    let (proxy_url, proxy_mode) = {
        let conn = state
            .db
            .get()
            .map_err(|e| format!("err.db_lock|{}", e))?;
        let pu = settings::get_setting(&conn, KEY_PROXY_URL)
            .ok()
            .flatten()
            .unwrap_or_default();
        let pm = settings::get_setting(&conn, KEY_PROXY_MODE)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if pu.is_empty() {
                    "none".to_string()
                } else {
                    "custom".to_string()
                }
            });
        (pu, pm)
    };
    let client = crate::http::build_http_client(crate::http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        bearer_token: None,
        ..Default::default()
    })?;
    let nav = client
        .get(BILI_NAV_URL)
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Cookie", format!("SESSDATA={}", sessdata))
        .send()
        .await
        .map_err(|e| format!("err.request_failed|{}", e))?;
    let body: serde_json::Value = nav
        .json()
        .await
        .map_err(|e| format!("err.parse_failed|{}", e))?;
    if body["data"]["isLogin"].as_bool() != Some(true) {
        return Err("err.bili_login_not_logged_in".to_string());
    }

    // 3) 加密存储
    let conn = state
        .db
        .get()
        .map_err(|e| format!("err.db_lock|{}", e))?;
    settings::set_setting(&conn, KEY_BILIBILI_COOKIE, &crypto::encrypt(&sessdata))?;
    crate::db::logs::write_log_key(
        &conn,
        "INFO",
        "setting.bilibili_cookie_updated",
        &serde_json::json!({"source": "webview-login"}).to_string(),
    );
    Ok(true)
}

/// 关闭登录窗口（前端在登录成功或用户放弃时调用）。
#[tauri::command]
pub fn close_bilibili_login_window(
    app: tauri::AppHandle,
    window_label: String,
) -> Result<(), String> {
    require_login_window_label(&window_label)?;
    if let Some(win) = app.get_webview_window(&window_label) {
        let _ = win.close();
    }
    Ok(())
}
