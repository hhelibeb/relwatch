use std::str::FromStr;

use tauri::Manager;
use tauri::{WebviewUrl, WebviewWindowBuilder};

use crate::db;
use crate::db::settings;
use crate::net::ProxyPolicy;
use crate::services::setting_service;
use crate::types::AppState;

const BILI_COOKIE_DOMAIN: &str = "https://www.bilibili.com";
const BILI_LOGIN_URL: &str = "https://passport.bilibili.com/login";
const BILI_LOGIN_WIDTH: f64 = 460.0;
const BILI_LOGIN_HEIGHT: f64 = 640.0;

/// 与 capabilities/bilibili-login.json 的 windows 白名单绑定，修改需两处同步。
pub const BILI_LOGIN_WINDOW_LABEL: &str = "bilibili-login";

/// 登录窗口独立 WebView2 数据目录：避免与主窗口的代理启动参数冲突导致创建静默失败。
pub const BILI_LOGIN_DATA_DIR: &str = "bilibili-login";

fn require_login_window_label(window_label: &str) -> Result<(), String> {
    if window_label != BILI_LOGIN_WINDOW_LABEL {
        log::warn!(
            "拒绝非登录窗口 label 的 B 站登录命令: {:?}（仅允许 {}）",
            window_label,
            BILI_LOGIN_WINDOW_LABEL
        );
        return Err("err.bili_login_window_missing".to_string());
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn read_bilibili_login_cookie(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    window_label: String,
) -> Result<bool, String> {
    require_login_window_label(&window_label)?;
    let win = app
        .get_webview_window(&window_label)
        .ok_or_else(|| "err.bili_login_window_missing".to_string())?;

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
        .ok_or_else(|| "err.bili_login_not_logged_in".to_string())?;

    setting_service::verify_and_save_bilibili_cookie(&state.db, &sessdata)
        .await
        .map_err(|e| e.to_ipc_string())
}

/// 必须在 async + spawn_blocking 中建窗：Windows 下 WebView2 build() 的 WaitWithPump
/// 在主线程或 tokio reactor 线程执行会导致自我死锁或阻塞事件循环。
#[tauri::command]
#[specta::specta]
pub async fn open_bilibili_login_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    title: String,
) -> Result<(), String> {
    if app.get_webview_window(BILI_LOGIN_WINDOW_LABEL).is_some() {
        return Ok(());
    }

    let login_url: tauri::Url = BILI_LOGIN_URL
        .parse()
        .map_err(|_| "err.invalid_url".to_string())?;

    let proxy = db::run_blocking(&state.db, |conn| {
        let cfg = settings::load_proxy_config(conn).unwrap_or_default();
        Ok(match ProxyPolicy::resolve(&cfg.mode, &cfg.url) {
            Ok(crate::net::ProxyDecision::Proxy(url)) => Some(url),
            _ => None,
        })
    })
    .await
    .map_err(|e| e.to_ipc_string())?;

    let app_for_build = app.clone();
    let login_data_dir = app_for_build
        .path()
        .resolve(BILI_LOGIN_DATA_DIR, tauri::path::BaseDirectory::AppLocalData)
        .map_err(|e| format!("err.bili_login_window_create|{}", e))?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut builder = WebviewWindowBuilder::new(
            &app_for_build,
            BILI_LOGIN_WINDOW_LABEL,
            WebviewUrl::External(login_url),
        )
        .data_directory(login_data_dir)
        .title(title)
        .inner_size(BILI_LOGIN_WIDTH, BILI_LOGIN_HEIGHT)
        .center()
        .resizable(false)
        // 只放行 B 站域，防止第三方页面跳转到本地 media 协议源获得 IPC 能力
        .on_navigation(|url| {
            let host = url.host_str().unwrap_or_default();
            let ok = host == "bilibili.com"
                || host.ends_with(".bilibili.com")
                || host == "hdslb.com"
                || host.ends_with(".hdslb.com");
            if !ok {
                log::warn!("已拦截 B 站登录窗口的非白名单导航: {}", url);
            }
            ok
        });
        if let Some(url) = proxy {
            builder = builder.proxy_url(url);
        }
        builder
            .build()
            .map(|_| ())
            .map_err(|e| format!("err.bili_login_window_create|{}", e))
    })
    .await
    .map_err(|e| format!("err.bili_login_window_create|{}", e))?
}

#[tauri::command]
#[specta::specta]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_login_window_label_accepts_own_label() {
        assert_eq!(require_login_window_label(BILI_LOGIN_WINDOW_LABEL), Ok(()));
    }

    #[test]
    fn test_require_login_window_label_rejects_other_labels() {
        for label in ["main", "", "bilibili-login-2", "Bilibili-Login"] {
            let err = require_login_window_label(label).unwrap_err();
            assert_eq!(err, "err.bili_login_window_missing", "label={:?}", label);
        }
    }

    #[test]
    fn test_require_login_window_label_constant_is_stable() {
        assert_eq!(BILI_LOGIN_WINDOW_LABEL, "bilibili-login");
    }

    #[test]
    fn test_login_data_dir_isolated_from_main_window() {
        assert_eq!(BILI_LOGIN_DATA_DIR, "bilibili-login");
        let p = std::path::Path::new(BILI_LOGIN_DATA_DIR);
        assert_eq!(p.components().count(), 1);
        assert!(!BILI_LOGIN_DATA_DIR.is_empty());
        assert_ne!(BILI_LOGIN_DATA_DIR, ".");
        assert_ne!(BILI_LOGIN_DATA_DIR, "..");
    }
}
