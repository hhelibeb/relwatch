//! 跨进程事件类型（tauri-specta 类型化事件）。
//!
//! 事件名默认取结构体名的 kebab-case（`ReleaseStateChanged` → `release-state-changed`），
//! 与历史事件名保持一致，前端经 `src/bindings.ts` 的 `events` 对象类型化监听。

use serde::Serialize;
use specta::Type;
use tauri_specta::Event;

/// release 状态变更（新增/已读/忽略/删除等），payload 为 release id。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct ReleaseStateChanged(pub i64);

/// 一次轮询完成（手动或定时），前端据此刷新列表与倒计时。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct PollCompleted;

/// 源因连续失败被自动禁用，payload 携带源信息供前端提示。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct SourceAutoDisabled {
    pub owner: String,
    pub repo: String,
    pub id: i64,
    pub failures: i64,
}

/// 托盘菜单请求前端切换标签页，payload 为目标标签名。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct Navigate(pub String);

/// 点击桌面通知**主体**请求前端聚焦某条 release，payload 为 release id。
/// 与「前往」按钮区分：点主体只唤起窗口并定位，**不改通知状态**（语义为「去看一眼」，
/// 不代表已处理），因此不 emit ReleaseStateChanged，托盘红点与未读计数保持不变。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct FocusRelease(pub i64);

/// 一次 Agent 提交运行结束（成功/失败/超时/取消），前端据此刷新工作区记录。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct AgentRunFinished {
    pub run_id: i64,
    /// 所属工作区会话标识（前端按 session_key 过滤刷新）。
    pub session_key: String,
    /// success | failed | timeout | cancelled
    pub status: String,
    /// 失败/超时的人类可读原因（成功时 null）。
    pub message: Option<String>,
}

/// pi RPC 事件流实时转发（打字机文本 / 工具状态 / 流式 bash 输出）。
/// `event` 为 pi RPC 协议的原始事件 JSON 序列化字符串（前端 JSON.parse 还原）。
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct AgentRpcStream {
    pub session_key: String,
    pub run_id: i64,
    pub event: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件名由 tauri-specta 从结构体名推导（kebab-case）。
    /// 前端 `src/bindings.ts` 依赖此名监听，改名会静默断链，故锁在这里。
    #[test]
    fn focus_release_event_name_is_kebab_case() {
        assert_eq!(<FocusRelease as tauri_specta::Event>::NAME, "focus-release");
    }

    /// 对照：既有事件的命名规则未受影响。
    #[test]
    fn navigate_event_name_unchanged() {
        assert_eq!(<Navigate as tauri_specta::Event>::NAME, "navigate");
    }
}
