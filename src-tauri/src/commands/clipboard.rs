use std::borrow::Cow;

/// 在主线程执行剪贴板写入（失败重试 3 次）。
///
/// 背景：tauri 命令运行在 tokio worker 线程，而 arboard 的 Windows 实现要求
/// 剪贴板的 open→set→close 全部在同一线程完成；在 worker 线程上
/// `SetClipboardData` 会以 1418（线程没有打开的剪贴板）失败。
/// 统一切到主线程（GUI 线程，与 Chromium/Firefox 的剪贴板写入策略一致）执行。
async fn clipboard_write<F>(app: tauri::AppHandle, mut write: F) -> Result<(), String>
where
    F: FnMut(&mut arboard::Clipboard) -> Result<(), arboard::Error> + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let mut result = Ok(());
        for attempt in 0..3 {
            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    result = write(&mut cb).map_err(|e| e.to_string());
                    if result.is_ok() {
                        break;
                    }
                }
                Err(e) => result = Err(e.to_string()),
            }
            // 其他程序可能短暂占用剪贴板，稍作等待后重试（仅失败路径会阻塞主线程）
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "剪贴板写入中断".to_string())?
}

/// 写入文本到系统剪贴板。
/// 前端 navigator.clipboard 依赖文档焦点/用户激活，在右键菜单等场景不可靠，
/// 统一走 Rust 端写入。
#[tauri::command]

#[specta::specta]pub async fn set_clipboard_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    clipboard_write(app, move |cb| cb.set_text(text.as_str())).await
}

/// 写入图片到系统剪贴板。`bytes` 为 PNG 编码字节（前端已用 canvas 统一转码），
/// 这里解码为 RGBA 后交给 arboard（由它生成 Windows 需要的 DIBV5/PNG 格式）。
#[tauri::command]

#[specta::specta]pub async fn set_clipboard_image(app: tauri::AppHandle, bytes: Vec<u8>) -> Result<(), String> {
    let img = tauri::image::Image::from_bytes(&bytes)
        .map_err(|e| format!("err.image_decode|{}", e))?;
    let width = img.width() as usize;
    let height = img.height() as usize;
    let rgba = img.rgba().to_vec();
    clipboard_write(app, move |cb| {
        cb.set_image(arboard::ImageData {
            bytes: Cow::Borrowed(&rgba),
            width,
            height,
        })
    })
    .await
}
