use tauri::{
    Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent},
};

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
                        let _ = app.emit("navigate", "sources");
                    }
                }
                "tray_releases" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = app.emit("navigate", "releases");
                    }
                }
                "tray_settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = app.emit("navigate", "settings");
                    }
                }
                "tray_check_now" => {
                    crate::poll::trigger_poll_async(app.clone());
                }
                "quit" => {
                    crate::poll::stop_poll();
                    let _ = app.run_on_main_thread(|| {
                        crate::notify::uninit_com();
                    });
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
