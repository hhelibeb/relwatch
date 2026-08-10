use tauri::{
    Listener, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent},
};
use crate::types::AppState;
use tauri_specta::Event;

pub fn create_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let sources = MenuItemBuilder::with_id("tray_sources", "监控源").build(app)?;
    let releases = MenuItemBuilder::with_id("tray_releases", "版本列表").build(app)?;
    let settings = MenuItemBuilder::with_id("tray_settings", "设置").build(app)?;
    let check_now = MenuItemBuilder::with_id("tray_check_now", "立即检查").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&sources)
        .item(&releases)
        .item(&settings)
        .separator()
        .item(&check_now)
        .separator()
        .item(&quit)
        .build()?;

    let icon = app.default_window_icon().cloned().ok_or("no icon")?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .menu(&menu)
        .tooltip("RelWatch")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "tray_sources" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = crate::events::Navigate("sources".to_string()).emit(app);
                    }
                }
                "tray_releases" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = crate::events::Navigate("releases".to_string()).emit(app);
                    }
                }
                "tray_settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = crate::events::Navigate("settings".to_string()).emit(app);
                    }
                }
                "tray_check_now" => {
                    crate::poll::trigger_poll_async(app.clone());
                }
                "quit" => {
                    crate::poll::stop_poll();
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// 根据未读版本数量更新托盘图标
/// 有 unread release 时显示带小红点的图标，否则显示原始图标
pub fn update_tray_badge(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let state = app.state::<AppState>();
        let conn = match state.db.get() {
            Ok(c) => c,
            Err(_) => return,
        };
        let unread = crate::db::releases::get_unread_releases(&conn).unwrap_or_default();

        let icon = if unread.is_empty() {
            tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).ok()
        } else {
            tauri::image::Image::from_bytes(include_bytes!("../icons/icon-badge.png")).ok()
        };

        if let Some(icon) = icon {
            let _ = tray.set_icon(Some(icon));
        }
    }
}

/// 注册事件监听器，在版本状态变化或轮询完成时自动更新托盘图标
pub fn setup_tray_listeners(app: &tauri::AppHandle) {
    let app1 = app.clone();
    app.listen("release-state-changed", move |_| {
        update_tray_badge(&app1);
    });
    let app2 = app.clone();
    app.listen("poll-completed", move |_| {
        update_tray_badge(&app2);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_from_bytes_valid_png() {
        let bytes = include_bytes!("../icons/32x32.png");
        let result = tauri::image::Image::from_bytes(bytes);
        assert!(result.is_ok(), "Should load valid PNG");
    }

    #[test]
    fn test_from_bytes_invalid_bytes() {
        let result = tauri::image::Image::from_bytes(b"not a png file");
        assert!(result.is_err(), "Should error on invalid bytes");
    }

    #[test]
    fn test_from_bytes_empty_bytes() {
        let result = tauri::image::Image::from_bytes(&[]);
        assert!(result.is_err(), "Should error on empty bytes");
    }

    #[test]
    fn test_badge_icon_exists() {
        let bytes = include_bytes!("../icons/icon-badge.png");
        let result = tauri::image::Image::from_bytes(bytes);
        assert!(result.is_ok(), "icon-badge.png should be a valid PNG");
    }

    #[test]
    fn test_badge_icon_differs_from_original() {
        let original = include_bytes!("../icons/32x32.png");
        let badge = include_bytes!("../icons/icon-badge.png");
        assert_ne!(original.len(), badge.len(), "Badge icon should differ from original");
    }
}
