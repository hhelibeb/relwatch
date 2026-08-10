use std::borrow::Cow;

/// 将 PNG 字节解码为 (宽, 高, RGBA)。抽为纯函数以便测试图片解码错误路径。
fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let img = tauri::image::Image::from_bytes(bytes).map_err(|e| format!("err.image_decode|{}", e))?;
    Ok((img.width() as usize, img.height() as usize, img.rgba().to_vec()))
}

/// 重试 3 次的通用包装：前两次失败间隔 50ms 重试，第三次失败返回最后一次错误。
/// 抽自 clipboard_write 的重试循环，便于脱离剪贴板环境单测重试语义。
fn with_retry<F, T>(mut attempt: F) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    let mut last_err = String::new();
    for i in 0..3 {
        match attempt() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e;
                if i < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    Err(last_err)
}

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
        let result = with_retry(|| {
            let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            write(&mut cb).map_err(|e| e.to_string())
        });
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
    let (width, height, rgba) = decode_png(&bytes)?;
    clipboard_write(app, move |cb| {
        cb.set_image(arboard::ImageData {
            bytes: Cow::Borrowed(&rgba),
            width,
            height,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_decode_png_valid_png_returns_dimensions_and_rgba() {
        // 1x1 红色 (255,0,0,255) PNG：标准 70 字节（IHDR 8bit RGBA + IDAT + IEND）
        let png: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8,
            6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192,
            240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];

        let (w, h, rgba) = decode_png(png).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(rgba, vec![255, 0, 0, 255]);
    }

    #[test]
    fn test_decode_png_invalid_bytes_returns_image_decode_error() {
        let err = decode_png(b"not a png").unwrap_err();
        assert!(err.starts_with("err.image_decode|"), "实际错误: {}", err);
    }

    #[test]
    fn test_decode_png_empty_bytes_errors() {
        assert!(decode_png(&[]).is_err());
    }

    #[test]
    fn test_with_retry_succeeds_on_first_try() {
        let calls = AtomicUsize::new(0);
        let result = with_retry(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>("ok")
        });
        assert_eq!(result.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "成功路径只尝试一次");
    }

    #[test]
    fn test_with_retry_recovers_on_second_try() {
        let calls = AtomicUsize::new(0);
        let result = with_retry(|| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err::<(), _>("first fail".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_with_retry_gives_up_after_three_failures() {
        let calls = AtomicUsize::new(0);
        let result = with_retry(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>("boom".into())
        });
        assert_eq!(result.unwrap_err(), "boom");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_with_retry_returns_last_error_message() {
        let mut i = 0;
        let result = with_retry(move || {
            i += 1;
            Err::<(), _>(format!("fail-{}", i))
        });
        assert_eq!(result.unwrap_err(), "fail-3", "返回最后一次失败的错误信息");
    }
}
