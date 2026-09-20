//! 窗口「显示 + 置前」的统一实现。
//!
//! 为什么不能只靠 `show()` + `set_focus()`：
//!
//! Windows 的 `SetForegroundWindow` 受前台锁（foreground lock）限制，只有在
//! 「调用进程是前台进程 / 进程刚收到最后一次输入事件 / 由前台进程启动 /
//! 当前无前台窗口 / 前台锁超时」之一满足时才被允许。托盘图标点击与通知主体
//! 点击这两类入口，鼠标消息归属 explorer.exe（Shell_TrayWnd），relwatch 进程
//! 不满足其中任何一条，于是 `set_focus()` 静默失败 —— tao 内部即
//! `SetForegroundWindow` 返回 FALSE，其备用的「SendInput 模拟 ALT 键」hack 只在
//! 本进程是输入接收者时有效，托盘场景同样救不回来
//! （见 tao-0.35.3 `platform_impl/windows/window.rs` 的 `set_focus` / `force_window_active`）。
//!
//! 于是症状是：窗口确实被 `show()` 出来了（任务栏多了按钮），但停在原 z-order
//! （被其它窗口挡着，看起来「没显示」），必须再点一次任务栏按钮 —— 那是系统认可的
//! 合法激活路径 —— 才会跑到最前。
//!
//! 本模块用「topmost 抖动」兜底：`HWND_TOPMOST` → `HWND_NOTOPMOST` 两次
//! `SetWindowPos` 会把窗口抬到 z-order 顶部，且取消置顶不会把它压回原位。
//! 这是「保证看得见」，**不是**「保证拿到键盘焦点」：窗口不会抢走当前应用的输入
//! 焦点，用户若要直接打字还需再点一下窗口。真正抢焦点要走 `AttachThreadInput`
//! 那一套 Win32 hack，代价与风险都更大，此处刻意不做。

use tauri::{Runtime, WebviewWindow};

/// 显示窗口并尽力置于最前。全部窗口操作都容错（窗口可能已销毁、mock runtime 不支持）。
pub(crate) fn show_and_focus<R: Runtime>(window: &WebviewWindow<R>) {
    // 最小化态先还原：`show()` 只切可见性，不还原最小化窗口；而 tao 的 `set_focus`
    // 在窗口处于最小化时直接跳过激活 → 只 show 会表现为「点了托盘没反应」。
    let _ = window.unminimize();
    let _ = window.show();
    // 前台锁允许时（正常场景）这一步就足够，无额外副作用。
    let _ = window.set_focus();

    // 兜底只在真的没拿到激活时才做：已聚焦、或窗口本就置顶时跳过，
    // 避免无谓的 z-order 抖动，也避免把窗口原有的置顶状态抖掉。
    let focused = window.is_focused().unwrap_or(false);
    let already_on_top = window.is_always_on_top().unwrap_or(false);
    if !focused && !already_on_top {
        let _ = window.set_always_on_top(true);
        let _ = window.set_always_on_top(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    /// Windows 的真实前台/置顶行为无法在 mock runtime 上断言（没有真 HWND），
    /// 这里锁的是另一条约束：整条调用链必须容错、不 panic、不求 Result ——
    /// mock 下 `is_focused` / `is_always_on_top` 都会报错，函数仍要正常走完。
    #[test]
    fn test_show_and_focus_smoke_on_mock_window() {
        let app = mock_builder()
            .setup(|app| {
                let win = tauri::WebviewWindowBuilder::new(
                    app.handle(),
                    "main",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .build()
                .map_err(|e| e.to_string())?;
                show_and_focus(&win);
                Ok(())
            })
            .build(mock_context(noop_assets()))
            .unwrap();
        drop(app);
    }
}
