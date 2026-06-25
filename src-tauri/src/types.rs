use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicI64;
use tokio::sync::Semaphore;

use crate::db::logs::LogEntry;
use crate::db::releases::ReleaseInfo;

/// 可测试的事件发射器抽象层。
///
/// 编排层核心函数通过 `&dyn Emitter` 依赖注入替代 `&tauri::AppHandle`，
/// 使并发/状态机/错误传播编排链路可在纯测试环境中验证。
pub trait Emitter: Send + Sync {
    /// 发送 release 桌面通知。
    /// 这是 `collect_pending_and_notify` 对 Tauri 主线程的唯一依赖。
    fn notify_release(&self, params: ReleaseNotifyParams);
}

/// `Emitter::notify_release` 的参数载体：把原先 8 个位置参数收敛为结构体，
/// 既消除 `clippy::too_many_arguments` 警告，也便于未来扩展通知字段。
#[derive(Clone)]
pub struct ReleaseNotifyParams {
    pub release_id: i64,
    pub html_url: String,
    pub owner: String,
    pub repo: String,
    pub tag: String,
    pub name: String,
    pub importance: Option<String>,
}

/// Tauri AppHandle 实现：通过 `run_on_main_thread` 派发通知。
impl Emitter for tauri::AppHandle {
    fn notify_release(&self, params: ReleaseNotifyParams) {
        let app = self.clone();
        let _ = self.run_on_main_thread(move || {
            let ReleaseNotifyParams {
                release_id,
                html_url,
                owner,
                repo,
                tag,
                name,
                importance,
            } = params;
            crate::notify::send_release_notification(
                &app, release_id, html_url, owner, repo, tag, name, importance,
            );
        });
    }
}

/// 测试用 Noop Emitter：不发送任何通知，仅记录调用次数（可扩展）。
#[cfg(test)]
pub struct NoopEmitter(pub std::sync::atomic::AtomicUsize);

#[cfg(test)]
impl NoopEmitter {
    pub const fn new() -> Self {
        NoopEmitter(std::sync::atomic::AtomicUsize::new(0))
    }

    /// 已记录的通知调用次数。
    pub fn call_count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
impl Emitter for NoopEmitter {
    fn notify_release(&self, _params: ReleaseNotifyParams) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct AppState {
    pub db: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    pub next_poll_at: std::sync::Arc<AtomicI64>,
    pub deepseek_semaphore: std::sync::Arc<Semaphore>,
}

#[derive(Serialize)]
pub struct PollResult {
    pub new_releases: Vec<ReleaseInfo>,
}

#[derive(Serialize)]
pub struct LogSearchResult {
    pub entries: Vec<LogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    pub poll_interval_minutes: i64,
    pub proxy_url: String,
    pub proxy_mode: String,
    pub auto_start: bool,
    pub minimize_to_tray: bool,
    pub log_retention_days: i64,
    pub deepseek_enabled: bool,
    pub deepseek_model: String,
    pub deepseek_base_url: String,
    pub deepseek_api_key_set: bool,
    pub deepseek_proxy_bypass: bool,
    pub deepseek_prompt: String,
    pub deepseek_min_importance: String,
    pub deepseek_translate_release: bool,

    pub check_prereleases: bool,
    pub fetch_history: bool,
    pub fetch_history_count: i64,
    pub language: String,
    pub theme: String,
    pub show_source_type_icons: bool,
    pub github_token_set: bool,
}
