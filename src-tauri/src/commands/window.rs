use tauri::Manager;

/// 将主窗口隐藏到系统托盘
///
/// 由前端的 Escape 逐层退出链最后一层触发。仅在用户开启了
/// `minimize_to_tray` 设置时才会被调用。
#[tauri::command]

#[specta::specta]pub fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}
