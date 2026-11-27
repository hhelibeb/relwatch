mod ai_jobs;
mod check;
mod notify_flow;
mod settings;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use tauri::Manager;
use tauri_specta::Event;

use crate::db;
use crate::db::settings::{KEY_LOG_RETENTION, KEY_NEXT_POLL_AT, KEY_POLL_INTERVAL};
use crate::deepseek;
use crate::types::{AppState, PollResult};
use ai_jobs::{
    filter_ai_eligible, schedule_summaries_background, schedule_translations_background,
};
use check::poll_all_sources_async;
use notify_flow::collect_pending_and_notify;
use settings::{load_poll_settings, PollSettings};

pub use check::check_single_source;

const MAX_CONSECUTIVE_FAILURES: i64 = 3;
const MAX_CONCURRENCY: usize = 10;
const SOURCE_FETCH_TIMEOUT_SECS: u64 = 300;

/// 轮询与后台 AI 批的运行时状态容器（默认单例供应用使用，测试可独立实例化隔离并发）。
pub struct PollRuntime {
    pub(crate) lock: AtomicBool,
    pub(crate) running: AtomicBool,
    pub(crate) wake: tokio::sync::Notify,
    pub(crate) translation_batch_running: AtomicBool,
    pub(crate) summary_batch_running: AtomicBool,
}

impl PollRuntime {
    pub const fn const_new() -> Self {
        Self {
            lock: AtomicBool::new(false),
            running: AtomicBool::new(true),
            wake: tokio::sync::Notify::const_new(),
            translation_batch_running: AtomicBool::new(false),
            summary_batch_running: AtomicBool::new(false),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn notify_wake(&self) {
        self.wake.notify_waiters();
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub(crate) fn acquire_lock(&self) -> Result<PollGuard<'_>, String> {
        if self
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("err.poll_in_progress".to_string());
        }
        Ok(PollGuard { lock: &self.lock })
    }
}

pub(crate) static RUNTIME: PollRuntime = PollRuntime::const_new();

pub fn is_poll_running() -> bool {
    RUNTIME.is_running()
}

pub fn notify_poll_wake() {
    RUNTIME.notify_wake();
}

pub fn stop_poll() {
    RUNTIME.stop();
}

pub(crate) struct PollGuard<'a> {
    lock: &'a AtomicBool,
}

impl Drop for PollGuard<'_> {
    fn drop(&mut self) {
        self.lock.store(false, Ordering::Release);
    }
}

pub(crate) fn acquire_lock() -> Result<PollGuard<'static>, String> {
    RUNTIME.acquire_lock()
}


pub async fn trigger_poll(app: tauri::AppHandle) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;
    do_trigger_poll_async(app).await
}

/// 两个 do_*_poll_async 函数的公共核心：拉取所有源 → AI 摘要 → 通知 →
/// 派发后台翻译/摘要补全批
async fn do_poll_core(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &std::sync::Arc<dyn crate::types::Emitter>,
    sources: &[db::sources::Source],
    settings: &PollSettings,
    is_manual: bool,
) -> (Vec<i64>, Vec<db::releases::ReleaseInfo>) {
    let (all_new_ids, all_saved) = poll_all_sources_async(db_pool, sources, settings).await;

    // 跳过 ai_eligible=false 的源（youtube/bilibili）后生成摘要与译文。
    // fail-closed：过滤失败（DB 错误/panic）时跳过本轮 AI 并记日志，
    // 宁可下轮重试也不放行 youtube 条目给 DeepSeek。
    match filter_ai_eligible(db_pool, &all_saved).await {
        Ok(ai_saved) if !ai_saved.is_empty() => {
            // 摘要保持同步：桌面通知正文带重要度（🔴🟡🟢），需在摘要落库后
            // 才发通知。摘要 max_tokens=800、秒回，等待成本可忽略。
            deepseek::generate_summaries_for_new(db_pool, deepseek_semaphore, &ai_saved).await;
        }
        Ok(_) => {}
        Err(e) => log::error!("err.filter_ai_eligible|{}", e),
    }

    let (_, new_releases) = collect_pending_and_notify(db_pool, &**emitter, &all_new_ids, is_manual).await;

    // 译文后台批（fire-and-forget）：读 DB 中全部未完成翻译的 release
    // （= 本轮新入库 + 历史失败重试，两者此时均已入库），交给后台任务翻译，
    // 本轮回立即结束，不被翻译的最长 ~300s 超时拖累。
    // 译文成功落库后经 emitter 逐条 emit release-state-changed 刷新前端。
    schedule_translations_background(db_pool, deepseek_semaphore, emitter);

    // 摘要后台补全批（fire-and-forget）：补历史失败/未完成的摘要（原 retry 段
    // 的语义），不阻塞本轮；只处理已入库旧 release，不产生新通知。
    schedule_summaries_background(db_pool, deepseek_semaphore, emitter);

    (all_new_ids, new_releases)
}

async fn do_trigger_poll_async(app: tauri::AppHandle) -> Result<PollResult, String> {
    let (sources, settings);

    {
        let state = app.state::<AppState>();
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        sources = db::sources::list_sources(&conn)?
            .into_iter()
            .filter(|s| s.enabled)
            .collect::<Vec<_>>();
        settings = load_poll_settings(&conn)?;
    }

    let (db_pool, deepseek_semaphore) = {
        let state = app.state::<AppState>();
        (state.db.clone(), state.deepseek_semaphore.clone())
    };
    let emitter: std::sync::Arc<dyn crate::types::Emitter> =
        std::sync::Arc::new(app.clone());
    let (_, new_releases) = do_poll_core(
        &db_pool, &deepseek_semaphore, &emitter, &sources, &settings, true,
    ).await;

    // 手动触发特有逻辑：更新 next_poll_at
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        let now = chrono::Utc::now().timestamp();
        let interval = db::settings::get_setting(&conn, KEY_POLL_INTERVAL)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(30);
        let next = now + interval * 60;
        state.next_poll_at.store(next, Ordering::Relaxed);
        let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
    }

    let _ = crate::events::PollCompleted.emit(&app);

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}


pub fn trigger_poll_async(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = trigger_poll(app).await;
    });
}

async fn do_poll_async(app: tauri::AppHandle) {
    let Ok(_guard) = acquire_lock() else {
        return;
    };

    if !is_poll_running() {
        return;
    }

    let (sources, settings, retention_days);
    {
        let state = app.state::<AppState>();
        let conn = match state.db.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("err.db_lock|{}", e);
                return;
            }
        };
        sources = db::sources::list_sources(&conn).unwrap_or_default();
        settings = load_poll_settings(&conn).unwrap_or_default();
        retention_days = db::settings::get_setting(&conn, KEY_LOG_RETENTION)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if retention_days > 0 {
            db::logs::delete_old_logs(&conn, retention_days);
            // 使用统计与操作日志共用同一保留天数设置，同步清理过期分桶
            db::usage::prune_old_usage_stats(&conn, retention_days);
        }
    }

    // 自动禁用连续失败过多的监控源（断路器）
    let disabled_ids: Vec<i64> = {
        let state = app.state::<AppState>();
        let mut ids = Vec::new();
        if let Ok(conn) = state.db.get() {
            for source in &sources {
                if source.enabled && source.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    let _ = db::sources::update_source(&conn, source.id, false, source.poll_interval_minutes);
                    let (log_owner, log_repo) = crate::source::source_log_ident(&source.source_type, &source.owner, &source.repo, source.description.as_deref());
                    db::logs::write_log_key(
                        &conn,
                        "WARN",
                        "source.log_auto_disabled",
                        &serde_json::json!({"owner": &log_owner, "repo": &log_repo, "id": source.id}).to_string(),
                    );
                    log::warn!(
                        "自动禁用 {}/{} (id={})：连续失败 {} 次",
                        source.owner,
                        source.repo,
                        source.id,
                        source.consecutive_failures
                    );
                    let _ = crate::events::SourceAutoDisabled {
                        owner: source.owner.clone(),
                        repo: source.repo.clone(),
                        id: source.id,
                        failures: source.consecutive_failures,
                    }
                    .emit(&app);
                    ids.push(source.id);
                }
            }
        }
        ids
    };

    let enabled: Vec<db::sources::Source> =
        sources.into_iter().filter(|s| s.enabled && !disabled_ids.contains(&s.id)).collect();
    if enabled.is_empty() {
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.get() {
            db::logs::write_log_key(&conn, "INFO", "check.skipped", "{}");
        }
        return;
    }

    // 调度只读全局周期（`KEY_POLL_INTERVAL`）：每轮检查**全部**启用源。
    {
        let state = app.state::<AppState>();
        let db_pool = state.db.clone();
        let semaphore = state.deepseek_semaphore.clone();
        let emitter: std::sync::Arc<dyn crate::types::Emitter> =
            std::sync::Arc::new(app.clone());
        do_poll_core(
            &db_pool, &semaphore, &emitter, &enabled, &settings, false,
        ).await;
    }

    let _ = crate::events::PollCompleted.emit(&app);
}

/// 全局轮询周期的取值范围（分钟），与设置页输入框的 min/max 一致。
/// 写入侧（`commands::setting::update_settings`）用它做边界归一，避免越界值落库。
pub const MIN_POLL_INTERVAL_MINUTES: i64 = 5;
pub const MAX_POLL_INTERVAL_MINUTES: i64 = 1440;


/// 轮询等待的分片上限：单次等待不超过这个时长，超出部分留给下一片用挂钟重算。
///
/// 取值远小于可配间隔（分钟级），既不会把一个短间隔切碎成多次无效唤醒，又把
/// 「系统休眠唤醒后恢复检查」的延迟上界压到它（详见 `start_poll_thread`）。
const POLL_WAIT_SLICE: std::time::Duration = std::time::Duration::from_secs(60);

/// 本轮轮询还需等待多久；`None` = 已到点（含已过期），应当立即检查。
///
/// 返回值按 [`POLL_WAIT_SLICE`] 截断，调用方据此分段等待。判定用**挂钟**
/// （`chrono::Utc::now`）而不是单调时钟，正是为了让系统休眠唤醒后能立刻发现
/// 「已经过期」—— 单调时钟在休眠期间不推进，一次睡到底时它看不出时间已经走过。
fn poll_wait_slice(target_ts: i64, now_ts: i64) -> Option<std::time::Duration> {
    if target_ts <= now_ts {
        return None;
    }
    Some(std::time::Duration::from_secs((target_ts - now_ts) as u64).min(POLL_WAIT_SLICE))
}

pub fn start_poll_thread(app_handle: tauri::AppHandle, next_poll: std::sync::Arc<AtomicI64>) {
    tauri::async_runtime::spawn(async move {
        // Save the initial next_poll_at so restart can restore it
        let initial = next_poll.load(Ordering::Relaxed);
        if let Ok(conn) = app_handle.state::<AppState>().db.get() {
            let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &initial.to_string());
        }

        loop {
            // 等待到下一个轮询点。**分段重算，而非一次睡到底**：tokio 的 sleep 基于
            // 单调时钟（Windows 为 QPC），而系统休眠期间单调时钟不推进 —— 一次睡到底时，
            // 唤醒后仍要「补等」休眠前剩余的时长。分段后每片醒来都用挂钟重算，休眠
            // 唤醒后最多一个分片（`POLL_WAIT_SLICE`）内就会立即检查。
            //
            // `is_poll_running` 放在内层循环开头：`stop_poll()` 也是经 `POLL_WAKE`
            // 唤醒的，必须让它能在一段分片内就退出，而不是先白等到本轮到点。
            loop {
                if !is_poll_running() {
                    return;
                }
                let Some(slice) = poll_wait_slice(
                    next_poll.load(Ordering::Relaxed),
                    chrono::Utc::now().timestamp(),
                ) else {
                    break;
                };
                // 设置变更（update_settings 改间隔）、手动触发或 stop_poll 都经
                // POLL_WAKE 提前唤醒重算，替代逐秒 sleep 轮询。
                tokio::select! {
                    _ = RUNTIME.wake.notified() => {}
                    _ = tokio::time::sleep(slice) => {}
                }
            }

            do_poll_async(app_handle.clone()).await;

            let now = chrono::Utc::now().timestamp();
            let interval = {
                let state = app_handle.state::<AppState>();
                if let Ok(conn) = state.db.get() {
                    db::settings::get_setting(&conn, KEY_POLL_INTERVAL)
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(30)
                } else {
                    30
                }
            };
            let next = now + (interval as i64) * 60;
            next_poll.store(next, Ordering::Relaxed);
            if let Ok(conn) = app_handle.state::<AppState>().db.get() {
                let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
            }
        }
    });
}


#[cfg(test)]
mod tests;
