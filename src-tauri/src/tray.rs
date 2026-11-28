use std::sync::OnceLock;

use tauri::{
    Listener, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent},
};
use crate::types::AppState;
use tauri_specta::Event;

fn cached_tray_icon(has_badge: bool) -> Option<tauri::image::Image<'static>> {
    static BADGE_ICON: OnceLock<Option<tauri::image::Image<'static>>> = OnceLock::new();
    static NORMAL_ICON: OnceLock<Option<tauri::image::Image<'static>>> = OnceLock::new();
    if has_badge {
        BADGE_ICON
            .get_or_init(|| {
                tauri::image::Image::from_bytes(include_bytes!("../icons/icon-badge.png"))
                    .ok()
                    .map(|img| img.to_owned())
            })
            .clone()
    } else {
        NORMAL_ICON
            .get_or_init(|| {
                tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
                    .ok()
                    .map(|img| img.to_owned())
            })
            .clone()
    }
}

pub fn create_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let sources = MenuItemBuilder::with_id("tray_sources", "监控源").build(app)?;
    let releases = MenuItemBuilder::with_id("tray_releases", "版本列表").build(app)?;
    let settings = MenuItemBuilder::with_id("tray_settings", "设置").build(app)?;
    let check_now = MenuItemBuilder::with_id("tray_check_now", "立即检查").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    // 仅 debug 构建：手动触发一条通知，用于验证「点击通知主体」链路
    // （Windows 上该行为无法自动断言，必须真人点击一次）
    #[cfg(debug_assertions)]
    let test_notification =
        MenuItemBuilder::with_id("tray_test_notification", "发送测试通知（debug）").build(app)?;

    // 用 shadowing 而非 mut：release 下 debug_assertions 为 false，
    // 若声明 mut 会出现 unused_mut 警告（dev clippy 测不到，release 构建会红）
    let menu = MenuBuilder::new(app)
        .item(&sources)
        .item(&releases)
        .item(&settings)
        .separator()
        .item(&check_now);
    #[cfg(debug_assertions)]
    let menu = menu.separator().item(&test_notification);
    let menu = menu.separator().item(&quit).build()?;

    let icon = cached_tray_icon(false)
        .or_else(|| app.default_window_icon().cloned())
        .ok_or("no icon")?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .menu(&menu)
        .tooltip("RelWatch")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                // 菜单点击同样受 Windows 前台锁限制（点击发生在 shell 进程的消息队列里），
                // 故一律走 focus::show_and_focus 而不是 show + set_focus
                "tray_sources" => {
                    if let Some(window) = app.get_webview_window("main") {
                        crate::focus::show_and_focus(&window);
                        let _ = crate::events::Navigate("sources".to_string()).emit(app);
                    }
                }
                "tray_releases" => {
                    if let Some(window) = app.get_webview_window("main") {
                        crate::focus::show_and_focus(&window);
                        let _ = crate::events::Navigate("releases".to_string()).emit(app);
                    }
                }
                "tray_settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        crate::focus::show_and_focus(&window);
                        let _ = crate::events::Navigate("settings".to_string()).emit(app);
                    }
                }
                "tray_check_now" => {
                    crate::poll::trigger_poll_async(app.clone());
                }
                #[cfg(debug_assertions)]
                "tray_test_notification" => {
                    crate::notify::send_test_notification(app);
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
                        // 托盘点击的输入归属 explorer.exe，SetForegroundWindow 会被系统
                        // 拒绝 → 只 show 会出现「任务栏有按钮但窗口没到最前」，故走统一实现
                        crate::focus::show_and_focus(&window);
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn should_show_badge(conn: &rusqlite::Connection) -> bool {
    crate::db::releases::has_due_unread_release(conn).unwrap_or(false)
}

pub fn update_tray_badge(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let state = app.state::<AppState>();
        let conn = match state.db.get() {
            Ok(c) => c,
            Err(_) => return,
        };
        if let Some(icon) = cached_tray_icon(should_show_badge(&conn)) {
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
    use super::*;

    #[test]
    fn test_should_show_badge_snoozed_due_and_future() {
        let conn = crate::db::init::init_memory_db().unwrap();
        let sid = crate::db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let rid = crate::db::releases::insert_release(
            &conn,
            sid,
            "v1",
            "R",
            "https://x",
            "2024-01-01T00:00:00Z",
            false,
            None,
        )
        .unwrap();

        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        crate::db::releases::set_notification_state(&conn, rid, "snoozed", Some(&future)).unwrap();
        assert!(!should_show_badge(&conn));

        let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        crate::db::releases::set_notification_state(&conn, rid, "snoozed", Some(&past)).unwrap();
        assert!(should_show_badge(&conn));

        crate::db::releases::set_notification_state(&conn, rid, "clicked", None).unwrap();
        assert!(!should_show_badge(&conn));
    }

    /// 整条链路集成测试：DB 读取未读 + muted → 判定 → 是否亮红点。
    /// 覆盖核心行为「静音 → 红点消失；取消静音 → 红点恢复」。
    #[test]
    fn test_should_show_badge_db_chain() {
        let conn = crate::db::init::init_memory_db().unwrap();
        // 无数据 → 不亮
        assert!(!should_show_badge(&conn));

        // 非静音源新版本 → 亮
        let sid = crate::db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        crate::db::releases::insert_release(&conn, sid, "v1", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(should_show_badge(&conn));

        // 静音该源 → 红点消失
        crate::db::sources::set_source_muted(&conn, sid, true).unwrap();
        assert!(!should_show_badge(&conn));

        // 取消静音 → 红点恢复
        crate::db::sources::set_source_muted(&conn, sid, false).unwrap();
        assert!(should_show_badge(&conn));
    }

    /// 多源场景：仅静音源有未读 → 不亮；非静音源出现未读 → 亮。
    #[test]
    fn test_should_show_badge_db_multi_source() {
        let conn = crate::db::init::init_memory_db().unwrap();
        let muted_sid = crate::db::sources::add_source(&conn, "github", "a", "b", "").unwrap();
        let active_sid = crate::db::sources::add_source(&conn, "github", "c", "d", "").unwrap();
        crate::db::sources::set_source_muted(&conn, muted_sid, true).unwrap();

        // 仅静音源有未读 → 不亮
        crate::db::releases::insert_release(&conn, muted_sid, "v1", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(!should_show_badge(&conn));

        // 非静音源出现未读 → 亮
        crate::db::releases::insert_release(&conn, active_sid, "v2", "R", "https://y", "2024-01-02T00:00:00Z", false, None).unwrap();
        assert!(should_show_badge(&conn));
    }

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
