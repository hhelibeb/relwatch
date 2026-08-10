/// 隐藏主窗口（无 main 窗口时静默成功）。
/// 泛型化以便测试用 `tauri::test::mock_builder`（MockRuntime）直接验证。
fn hide_main_window<R: tauri::Runtime>(app: &impl tauri::Manager<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

/// 将主窗口隐藏到系统托盘
///
/// 由前端的 Escape 逐层退出链最后一层触发。仅在用户开启了
/// `minimize_to_tray` 设置时才会被调用。
#[tauri::command]

#[specta::specta]pub fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    hide_main_window(&app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        mock_builder().build(mock_context(noop_assets())).unwrap()
    }

    #[test]
    fn test_hide_main_window_without_main_window_is_noop() {
        // mock app 默认无任何窗口：应静默成功（不报错）
        let app = mock_app();
        assert_eq!(hide_main_window(app.handle()), Ok(()));
    }

    #[test]
    fn test_hide_main_window_with_main_window_succeeds() {
        // 通过 setup 创建名为 main 的窗口，隐藏应成功
        let app = mock_builder()
            .setup(|app| {
                let _win = tauri::WebviewWindowBuilder::new(
                    app.handle(),
                    "main",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .build()
                .map_err(|e| e.to_string())?;
                Ok(())
            })
            .build(mock_context(noop_assets()))
            .unwrap();
        assert_eq!(hide_main_window(app.handle()), Ok(()));
    }
}
