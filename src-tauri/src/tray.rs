use std::collections::HashSet;

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

/// 判断是否存在应亮红点的未读版本：
/// 静音源的 release 不参与托盘红点判定（静音语义 = 不要打扰，红点也是一种打扰）。
/// 未读列表本身不区分来源是否被静音，过滤留在这里做，避免污染全局未读语义。
fn has_unread_badge(unread: &[crate::db::releases::ReleaseInfo], muted_ids: &HashSet<i64>) -> bool {
    unread.iter().any(|r| !muted_ids.contains(&r.source_id))
}

/// 从数据库读取未读集合与静音源集合，判定是否应亮托盘红点。
/// 独立成纯 DB 读取 + 判定，便于单测锁住"静音源→红点消失 / 取消→恢复"整条链路；
/// update_tray_badge 只做"图标选择 + set_icon"，判定逻辑不依赖 AppHandle 也能测。
fn should_show_badge(conn: &rusqlite::Connection) -> bool {
    let unread = crate::db::releases::get_unread_releases(conn).unwrap_or_default();
    let muted_ids: HashSet<i64> = crate::db::sources::list_muted_source_ids(conn)
        .unwrap_or_default()
        .into_iter()
        .collect();
    has_unread_badge(&unread, &muted_ids)
}

/// 根据未读版本数量更新托盘图标
/// 有未读（且不属于静音源）release 时显示带小红点的图标，否则显示原始图标
pub fn update_tray_badge(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let state = app.state::<AppState>();
        let conn = match state.db.get() {
            Ok(c) => c,
            Err(_) => return,
        };
        let icon = if should_show_badge(&conn) {
            tauri::image::Image::from_bytes(include_bytes!("../icons/icon-badge.png")).ok()
        } else {
            tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).ok()
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
    use super::*;

    /// 构造最小 ReleaseInfo，仅填充托盘红点判定所需的 source_id。
    fn release(source_id: i64) -> crate::db::releases::ReleaseInfo {
        crate::db::releases::ReleaseInfo {
            id: source_id,
            source_id,
            source_type: "github".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            tag_name: "v1".to_string(),
            release_name: "R".to_string(),
            html_url: "https://x".to_string(),
            published_at: "2024-01-01T00:00:00Z".to_string(),
            prerelease: false,
            body: None,
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            notification_status: "pending".to_string(),
            snooze_until: None,
            ai_summary: None,
            ai_importance: None,
            body_translated: None,
            extra_metadata: None,
            source_description: None,
        }
    }

    #[test]
    fn test_has_unread_badge_empty_no_badge() {
        assert!(!has_unread_badge(&[], &HashSet::new()));
    }

    #[test]
    fn test_has_unread_badge_unread_not_muted() {
        let unread = vec![release(1), release(2)];
        assert!(has_unread_badge(&unread, &HashSet::new()));
    }

    #[test]
    fn test_has_unread_badge_all_muted_no_badge() {
        let unread = vec![release(1), release(2)];
        let muted: HashSet<i64> = [1, 2].into_iter().collect();
        assert!(!has_unread_badge(&unread, &muted));
    }

    #[test]
    fn test_has_unread_badge_mixed_muted_and_unmuted() {
        let unread = vec![release(1), release(2), release(3)];
        let muted: HashSet<i64> = [1, 3].into_iter().collect();
        // source 2 未静音 → 应亮红点
        assert!(has_unread_badge(&unread, &muted));
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
