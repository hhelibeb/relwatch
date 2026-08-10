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
