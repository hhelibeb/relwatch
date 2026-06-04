use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri::Emitter;

/// 连续失败超过此次数后自动禁用监控源，防止无限重试
const MAX_CONSECUTIVE_FAILURES: i64 = 3;

use crate::db;
use crate::db::settings::{
    KEY_POLL_INTERVAL, KEY_PROXY_URL, KEY_PROXY_MODE, KEY_LOG_RETENTION, KEY_GITHUB_TOKEN, KEY_NEXT_POLL_AT,
    KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT,
};
use crate::crypto;
use crate::github;
use crate::http;
use crate::deepseek;
use serde_json::json;
use crate::notify;
use crate::types::{AppState, PollResult};

static POLL_LOCK: AtomicBool = AtomicBool::new(false);
static POLL_RUNNING: AtomicBool = AtomicBool::new(true);
const MAX_CONCURRENCY: usize = 10;

pub fn is_poll_running() -> bool {
    POLL_RUNNING.load(Ordering::Relaxed)
}

pub fn stop_poll() {
    POLL_RUNNING.store(false, Ordering::SeqCst);
}

pub(crate) struct PollGuard;
impl Drop for PollGuard {
    fn drop(&mut self) {
        POLL_LOCK.store(false, Ordering::Release);
    }
}

pub(crate) fn acquire_lock() -> Result<PollGuard, String> {
    if POLL_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err("err.poll_in_progress".to_string());
    }
    Ok(PollGuard)
}

fn get_github_token(conn: &rusqlite::Connection) -> Option<String> {
    let encrypted = db::settings::get_setting(conn, KEY_GITHUB_TOKEN).ok()??;
    if encrypted.is_empty() {
        return None;
    }
    let (plain, new_v2) = crypto::decrypt_with_migration(&encrypted)?;
    if let Some(new_val) = new_v2 {
        if let Err(e) = db::settings::set_setting(conn, KEY_GITHUB_TOKEN, &new_val) {
            log::warn!("迁移 v1→v2 GitHub Token 回写失败: {}", e);
        }
    }
    Some(plain)
}

async fn fetch_for_source_async(
    client: &reqwest::Client,
    source: &db::sources::Source,
    per_page: usize,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    match source.source_type.as_str() {
        "github" => github::fetch_releases(client, &source.owner, &source.repo, per_page).await,
        other => Err((0, format!("err.unsupported_source|{}", other))),
    }
}

/// 使用分页拉取全部 releases（仅首次全量查询时使用）。
async fn fetch_all_for_source_async(
    client: &reqwest::Client,
    source: &db::sources::Source,
    max_count: Option<usize>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    match source.source_type.as_str() {
        "github" => github::fetch_all_releases_with_limit(client, &source.owner, &source.repo, max_count).await,
        other => Err((0, format!("err.unsupported_source|{}", other))),
    }
}

fn save_for_source(
    conn: &rusqlite::Connection,
    source: &db::sources::Source,
    data: &[serde_json::Value],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    match source.source_type.as_str() {
        "github" => github::save_releases(conn, source.id, data, max_count),
        other => {
            log::error!("不支持的监控源类型: {}", other);
            vec![]
        }
    }
}

/// 根据 fetch_history 配置计算分页参数。
///
/// 返回 `(max_count, per_page, needs_pagination)`：
/// - `max_count`: `None` 表示不设上限（全量分页），`Some(n)` 表示最多拉取 n 条
/// - `per_page`: 每次 API 调用的页面大小
/// - `needs_pagination`: 是否需要翻页（true = 多次 API 调用）
fn compute_fetch_plan(
    fetch_history: bool,
    is_first_query: bool,
    fetch_history_count: usize,
) -> (Option<usize>, usize, bool) {
    if fetch_history && is_first_query && (fetch_history_count == 0 || fetch_history_count > 100) {
        // 0 = 全量分页（None），>100 = 受限分页（前端限制 max=100，此分支作为安全兜底）
        let max_count = if fetch_history_count == 0 { None } else { Some(fetch_history_count) };
        (max_count, 100, true)
    } else if fetch_history && is_first_query {
        // 小数量，单次 API 调用即可
        (Some(fetch_history_count), (fetch_history_count + 5).clamp(10, 100), false)
    } else {
        (Some(fetch_history_count), 10, false)
    }
}

pub async fn trigger_poll(app: tauri::AppHandle) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;
    do_trigger_poll_async(app).await
}

/// 两个 do_*_poll_async 函数的公共核心：拉取所有源 → AI 摘要 → 通知 → 重试失败摘要
#[allow(clippy::too_many_arguments)]
async fn do_poll_core(
    app: &tauri::AppHandle,
    sources: &[db::sources::Source],
    proxy_url: &str,
    proxy_mode: &str,
    github_token: Option<&str>,
    fetch_history: bool,
    fetch_history_count: usize,
    is_manual: bool,
) -> (Vec<i64>, Vec<db::releases::ReleaseInfo>) {
    let (all_new_ids, all_saved) = poll_all_sources_async(
        app, sources, proxy_url, proxy_mode, github_token, fetch_history, fetch_history_count,
    )
    .await;

    if !all_saved.is_empty() {
        deepseek::generate_summaries_for_new(app, &all_saved).await;
    }

    let (_, new_releases) = collect_pending_and_notify(app, &all_new_ids, is_manual);

    // 重试之前失败的 AI 摘要生成
    let retry_releases = {
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.get() {
            db::releases::get_releases_without_summary(&conn).unwrap_or_default()
        } else {
            vec![]
        }
    };
    if !retry_releases.is_empty() {
        log::info!("正在重试 {} 个之前失败的 AI 摘要", retry_releases.len());
        deepseek::generate_summaries_for_new(app, &retry_releases).await;
    }

    (all_new_ids, new_releases)
}

async fn do_trigger_poll_async(app: tauri::AppHandle) -> Result<PollResult, String> {
    let (sources, proxy_url, proxy_mode, github_token, fetch_history, fetch_history_count);

    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        sources = db::sources::list_sources(&conn)?
            .into_iter()
            .filter(|s| s.enabled)
            .collect::<Vec<_>>();
        proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        proxy_mode = db::settings::get_setting(&conn, KEY_PROXY_MODE)?.unwrap_or_else(|| {
            if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
        });
        github_token = get_github_token(&conn);
        fetch_history = db::settings::get_setting_bool(&conn, KEY_FETCH_HISTORY, false).unwrap_or(false);
        fetch_history_count = db::settings::get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1).unwrap_or(1).max(0) as usize;
    }

    let (_, new_releases) = do_poll_core(
        &app, &sources, &proxy_url, &proxy_mode, github_token.as_deref(),
        fetch_history, fetch_history_count, true,
    ).await;

    // 手动触发特有逻辑：更新 next_poll_at
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
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

    let _ = app.emit("poll-completed", ());

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}

pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;

    let (source_obj, proxy_url, proxy_mode, github_token, fetch_history, fetch_history_count);
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        let sources = db::sources::list_sources(&conn)?;
        let source = sources
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("err.source_not_found")?;
        proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        proxy_mode = db::settings::get_setting(&conn, KEY_PROXY_MODE)?.unwrap_or_else(|| {
            if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
        });
        github_token = get_github_token(&conn);
        fetch_history = db::settings::get_setting_bool(&conn, KEY_FETCH_HISTORY, false).unwrap_or(false);
        fetch_history_count = db::settings::get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1).unwrap_or(1).max(0) as usize;
        source_obj = source;
    }

    let is_first_query = source_obj.last_checked_at.is_none()
        || source_obj.last_checked_at.as_deref() == Some("");
    let (max_count, per_page, needs_pagination) = compute_fetch_plan(fetch_history, is_first_query, fetch_history_count);

    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        bearer_token: github_token.as_deref(),
        ..Default::default()
    }) {
        Ok(client) => client,
        Err(e) => {
            let state = app.state::<AppState>();
            let conn = state.db.get().unwrap();
            let _ = db::sources::record_check_failure(&conn, source_obj.id, &e);
            db::logs::write_log_key(
                &conn,
                "WARN",
                "check.failed",
                &json!({"owner": &source_obj.owner, "repo": &source_obj.repo, "error": &e}).to_string(),
            );
            return Err(e);
        }
    };

    let releases = match if needs_pagination {
        let limit = max_count;
        fetch_all_for_source_async(&client, &source_obj, limit).await
    } else {
        fetch_for_source_async(&client, &source_obj, per_page).await
    } {
        Ok(releases) => releases,
        Err((status, msg)) => {
            let state = app.state::<AppState>();
            let conn = state.db.get().unwrap();
            let _ = db::sources::record_check_failure(&conn, source_obj.id, &msg);
            // 网络错误(0)、认证/限流(401/403/429)、服务端错误(5xx) 均为临时性，记为 WARN
            let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 { "WARN" } else { "ERROR" };
            db::logs::write_log_key(
                &conn,
                level,
                "check.failed",
                &json!({"owner": &source_obj.owner, "repo": &source_obj.repo, "error": &msg}).to_string(),
            );
            return Err(msg);
        }
    };
    let saved: Vec<(i64, Option<String>)>;
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        saved = save_for_source(&conn, &source_obj, &releases, max_count.unwrap_or(usize::MAX));
        if !saved.is_empty() {
            let latest_id = saved[0].0;
            let has_newer = db::releases::get_release(&conn, latest_id)
                .ok()
                .flatten()
                .and_then(|r| {
                    db::releases::has_newer_release(&conn, source_obj.id, &r.published_at).ok()
                })
                .unwrap_or(false);
            if has_newer {
                for (id, _) in &saved {
                    let _ = db::releases::set_notification_state(&conn, *id, "clicked", None);
                }
            } else if saved.len() > 1 {
                for (id, _) in saved.iter().skip(1) {
                    let _ = db::releases::set_notification_state(&conn, *id, "clicked", None);
                }
            }
        }
        let _ = db::sources::record_check_success(&conn, source_obj.id, saved.len());
        db::logs::write_log_key(
            &conn,
            "INFO",
            "check.manual",
            &json!({"owner": &source_obj.owner, "repo": &source_obj.repo, "count": saved.len()}).to_string(),
        );
    }

    if let Ok(desc) = github::fetch_repo_info(&client, &source_obj.owner, &source_obj.repo).await {
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.get() {
            let _ = db::sources::update_source_description(&conn, source_obj.id, &desc);
        }
    }

    let new_ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();

    if !saved.is_empty() {
        deepseek::generate_summaries_for_new(&app, &saved).await;
    }

    let (_, new_releases) = collect_pending_and_notify(&app, &new_ids, false);

    let _ = app.emit("poll-completed", ());

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}

#[allow(dead_code)]
pub fn do_poll(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        do_poll_async(app).await;
    });
}

pub fn trigger_poll_async(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = trigger_poll(app).await;
    });
}

async fn do_poll_async(app: tauri::AppHandle) {
    if POLL_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let _guard = PollGuard;

    if !is_poll_running() {
        return;
    }

    let (sources, proxy_url, proxy_mode, retention_days, github_token, fetch_history, fetch_history_count);
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        sources = db::sources::list_sources(&conn).unwrap_or_default();
        proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)
            .ok()
            .flatten()
            .unwrap_or_default();
        proxy_mode = db::settings::get_setting(&conn, KEY_PROXY_MODE)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
            });
        retention_days = db::settings::get_setting(&conn, KEY_LOG_RETENTION)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if retention_days > 0 {
            db::logs::delete_old_logs(&conn, retention_days);
        }
        github_token = get_github_token(&conn);
        fetch_history = db::settings::get_setting_bool(&conn, KEY_FETCH_HISTORY, false).unwrap_or(false);
        fetch_history_count = db::settings::get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1).unwrap_or(1).max(0) as usize;
    }

    // 自动禁用连续失败过多的监控源（断路器）
    {
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.get() {
            for source in &sources {
                if source.enabled && source.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    let _ = db::sources::update_source(&conn, source.id, false, source.poll_interval_minutes);
                    db::logs::write_log_key(
                        &conn,
                        "WARN",
                        "source.log_auto_disabled",
                        &serde_json::json!({"owner": &source.owner, "repo": &source.repo, "id": source.id}).to_string(),
                    );
                    log::warn!(
                        "自动禁用 {}/{} (id={})：连续失败 {} 次",
                        source.owner,
                        source.repo,
                        source.id,
                        source.consecutive_failures
                    );
                    let _ = app.emit("source-auto-disabled", serde_json::json!({
                        "owner": &source.owner,
                        "repo": &source.repo,
                        "id": source.id,
                        "failures": source.consecutive_failures,
                    }));
                }
            }
        }
    }

    let enabled: Vec<db::sources::Source> =
        sources.into_iter().filter(|s| s.enabled).collect();
    if enabled.is_empty() {
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.get() {
            db::logs::write_log_key(&conn, "INFO", "check.skipped", "{}");
        }
        return;
    }

    do_poll_core(
        &app, &enabled, &proxy_url, &proxy_mode, github_token.as_deref(),
        fetch_history, fetch_history_count, false,
    ).await;

    let _ = app.emit("poll-completed", ());
}

async fn poll_all_sources_async(
    app: &tauri::AppHandle,
    sources: &[db::sources::Source],
    proxy_url: &str,
    proxy_mode: &str,
    github_token: Option<&str>,
    fetch_history: bool,
    fetch_history_count: usize,
) -> (Vec<i64>, Vec<(i64, Option<String>)>) {
    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url,
        proxy_mode,
        bearer_token: github_token,
        ..Default::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            let state = app.state::<AppState>();
            if let Ok(conn) = state.db.get() {
                for source in sources {
                    let _ = db::sources::record_check_failure(&conn, source.id, &e);
                }
                db::logs::write_log_key(&conn, "ERROR", "check.http_client_error", &json!({"error": &e}).to_string());
            }
            return (vec![], vec![]);
        }
    };

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENCY));
    let mut handles = Vec::new();

    for source in sources {
        let sem = semaphore.clone();
        let client = client.clone();
        let source = source.clone();
        let app = app.clone();

        let is_first_query = source.last_checked_at.is_none()
            || source.last_checked_at.as_deref() == Some("");
        let (max_count, per_page, needs_pagination) = compute_fetch_plan(fetch_history, is_first_query, fetch_history_count);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let fetch_result = if needs_pagination {
                let limit = max_count;
                fetch_all_for_source_async(&client, &source, limit).await
            } else {
                fetch_for_source_async(&client, &source, per_page).await
            };
            match fetch_result {
                Ok(releases) => {
                    let state = app.state::<AppState>();
                    let conn = state.db.get().unwrap();
                    let saved = save_for_source(&conn, &source, &releases, max_count.unwrap_or(usize::MAX));
                    if !saved.is_empty() {
                        // 检查是否有比最新新版本更新的版本已存在库中
                        let latest_id = saved[0].0;
                        let has_newer = db::releases::get_release(&conn, latest_id)
                            .ok()
                            .flatten()
                            .and_then(|r| {
                                db::releases::has_newer_release(&conn, source.id, &r.published_at).ok()
                            })
                            .unwrap_or(false);
                        if has_newer {
                            // 库中已有更新版本 → 全部是中间版本，全部标记已读
                            for (id, _) in &saved {
                                let _ = db::releases::set_notification_state(&conn, *id, "clicked", None);
                            }
                        } else if saved.len() > 1 {
                            // 库中无更新版本 → 最新一条通知，其余标记已读
                            for (id, _) in saved.iter().skip(1) {
                                let _ = db::releases::set_notification_state(&conn, *id, "clicked", None);
                            }
                        }
                    }
                    let ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();
                    let new_count = ids.len();
                    let _ = db::sources::record_check_success(&conn, source.id, new_count);
                    db::logs::write_log_key(
                        &conn,
                        "INFO",
                        "check.auto",
                        &json!({"owner": &source.owner, "repo": &source.repo, "count": new_count}).to_string(),
                    );
                    (ids, saved)
                }
                Err((status, msg)) => {
                    let state = app.state::<AppState>();
                    let conn = state.db.get().unwrap();
                    let _ = db::sources::record_check_failure(&conn, source.id, &msg);
                    let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 { "WARN" } else { "ERROR" };
                    db::logs::write_log_key(
                        &conn,
                        level,
                        "check.failed",
                        &json!({"owner": &source.owner, "repo": &source.repo, "error": &msg}).to_string(),
                    );
                    (vec![], vec![])
                }
            }
        }));
    }

    let mut all_new_ids = Vec::new();
    let mut all_saved = Vec::new();
    for handle in handles {
        if let Ok((ids, saved)) = handle.await {
            all_new_ids.extend(ids);
            all_saved.extend(saved);
        }
    }

    (all_new_ids, all_saved)
}

fn collect_pending_and_notify(
    app: &tauri::AppHandle,
    new_ids: &[i64],
    is_manual: bool,
) -> (Vec<db::releases::ReleaseInfo>, Vec<db::releases::ReleaseInfo>) {
    let pending;
    let muted_source_ids;
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        pending = db::releases::get_pending_releases(&conn).unwrap_or_default();
        muted_source_ids = db::sources::list_muted_source_ids(&conn)
            .unwrap_or_default()
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
    }

    // Pending here means unread and eligible now, including expired snoozes.
    for release in &pending {
        // 标记该发布已通知
        {
            let state = app.state::<AppState>();
            if let Ok(conn) = state.db.get() {
                let _ = db::releases::set_last_notified_at(&conn, release.id);
            }
        }

        // 静默的源：只标记已通知，不发送桌面通知
        if muted_source_ids.contains(&release.source_id) {
            continue;
        }

        let app_clone = app.clone();
        let release_id = release.id;
        let html_url = release.html_url.clone();
        let owner = release.owner.clone();
        let repo = release.repo.clone();
        let tag = release.tag_name.clone();
        let name = release.release_name.clone();
        let importance = release.ai_importance.clone();
        let _ = app.run_on_main_thread(move || {
            notify::send_release_notification(
                &app_clone,
                release_id,
                html_url,
                owner,
                repo,
                tag,
                name,
                importance,
            );
        });
    }

    let new_releases: Vec<db::releases::ReleaseInfo> = pending
        .into_iter()
        .filter(|r| new_ids.contains(&r.id))
        .collect();

    if is_manual {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        db::logs::write_log_key(
            &conn,
            "INFO",
            "check.manual_all_done",
            &json!({"count": new_ids.len()}).to_string(),
        );
    }

    let all_pending = {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        db::releases::get_pending_releases(&conn).unwrap_or_default()
    };

    (all_pending, new_releases)
}

pub fn start_poll_thread(app_handle: tauri::AppHandle, next_poll: std::sync::Arc<AtomicI64>) {
    tauri::async_runtime::spawn(async move {
        // Save the initial next_poll_at so restart can restore it
        let initial = next_poll.load(Ordering::Relaxed);
        if let Ok(conn) = app_handle.state::<AppState>().db.get() {
            let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &initial.to_string());
        }

        loop {
            let target = next_poll.load(Ordering::Relaxed);
            let now = chrono::Utc::now().timestamp();
            let sleep_secs = (target - now).max(0) as u64;

            for _ in 0..sleep_secs {
                if !is_poll_running() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if chrono::Utc::now().timestamp() >= target {
                    break;
                }
            }

            do_poll_async(app_handle.clone()).await;

            let now = chrono::Utc::now().timestamp();
            let interval = {
                let state = app_handle.state::<AppState>();
                let conn = state.db.get().unwrap();
                db::settings::get_setting(&conn, KEY_POLL_INTERVAL)
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(30)
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
mod tests {
    use super::*;

    #[test]
    fn test_poll_lock_acquire_and_release() {
        POLL_LOCK.store(false, Ordering::Release);

        assert!(POLL_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok());
        assert!(POLL_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err());
        POLL_LOCK.store(false, Ordering::Release);
        assert!(POLL_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok());
        POLL_LOCK.store(false, Ordering::Release);
    }

    #[test]
    fn test_poll_guard_drop_releases_lock() {
        POLL_LOCK.store(false, Ordering::Release);

        {
            assert!(POLL_LOCK
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok());
            let _guard = PollGuard;
            assert!(POLL_LOCK.load(Ordering::Relaxed));
        }
        assert!(!POLL_LOCK.load(Ordering::Relaxed));
    }

    #[test]
    fn test_poll_running_flag() {
        POLL_RUNNING.store(true, Ordering::SeqCst);
        assert!(is_poll_running());

        stop_poll();
        assert!(!is_poll_running());

        POLL_RUNNING.store(true, Ordering::SeqCst);
    }

    use crate::db::init::init_memory_db;

    #[test]
    fn test_read_deepseek_config_defaults() {
        let conn = init_memory_db().unwrap();
        let (enabled, model, base_url, api_key, _prompt) = deepseek::read_config(&conn);
        assert!(!enabled);
        assert_eq!(model, "deepseek-v4-flash");
        assert_eq!(base_url, "https://api.deepseek.com");
        assert!(api_key.is_none());
    }

    #[test]
    fn test_read_deepseek_config_configured() {
        crate::crypto::set_test_master_key();
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_MODEL, "deepseek-v4-pro").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, "https://custom.api").unwrap();
        let encrypted = crate::crypto::encrypt("sk-test");
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_API_KEY, &encrypted).unwrap();

        let (enabled, model, base_url, api_key, _prompt) = deepseek::read_config(&conn);
        assert!(enabled);
        assert_eq!(model, "deepseek-v4-pro");
        assert_eq!(base_url, "https://custom.api");
        assert_eq!(api_key.unwrap(), "sk-test");
    }

    use crate::http;

    #[test]
    fn test_build_http_client_empty() {
        let result = http::build_http_client(http::HttpClientConfig::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_invalid() {
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_url: "://invalid",
            proxy_mode: "custom",
            ..Default::default()
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid proxy URL"));
    }

    #[test]
    fn test_build_http_client_valid() {
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_url: "http://127.0.0.1:1080",
            proxy_mode: "custom",
            ..Default::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_none_ignores_proxy_url() {
        // mode=none 时传无效 URL 也不报错
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_url: "://invalid",
            proxy_mode: "none",
            ..Default::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_system() {
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_mode: "system",
            ..Default::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_system_proxy() {
        let result = deepseek::build_client("sk-test", "", "system");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_success() {
        let result = deepseek::build_client("sk-test", "", "none");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_with_proxy() {
        let result = deepseek::build_client("sk-test", "http://127.0.0.1:1080", "custom");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_invalid_key() {
        let result = deepseek::build_client("key\nwith\nnewlines", "", "none");
        assert!(result.is_err());
    }

    // Simulates the startup logic in lib.rs
    fn calc_next_poll(conn: &rusqlite::Connection) -> i64 {
        let now = chrono::Utc::now().timestamp();
        db::settings::get_setting(conn, KEY_NEXT_POLL_AT)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|&v| v > now)
            .unwrap_or(now)
    }

    #[test]
    fn test_next_poll_at_startup_restores_future() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        // Saved next_poll_at is 20 min in the future → should be used as-is
        db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now + 20 * 60).to_string()).unwrap();
        let result = calc_next_poll(&conn);
        assert_eq!(result - now, 20 * 60);
    }

    #[test]
    fn test_next_poll_at_startup_falls_back_when_expired() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        // Saved next_poll_at is 10 min in the past → expired, fallback to now → immediate check
        db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now - 10 * 60).to_string()).unwrap();
        let result = calc_next_poll(&conn);
        assert_eq!(result, now);
    }

    #[test]
    fn test_acquire_lock_succeeds_and_fails() {
        // Bug #2 相关：验证 pub(crate) acquire_lock API
        POLL_LOCK.store(false, Ordering::Release);

        // 第一次获取应成功
        let guard1 = acquire_lock();
        assert!(guard1.is_ok(), "first acquire should succeed");
        assert!(POLL_LOCK.load(Ordering::Relaxed));

        // 第二次获取应失败（锁已被持有）
        let guard2 = acquire_lock();
        assert!(guard2.is_err(), "second acquire should fail while locked");

        // 释放第一个 guard
        drop(guard1);
        assert!(!POLL_LOCK.load(Ordering::Relaxed));

        // 再次获取应成功
        let guard3 = acquire_lock();
        assert!(guard3.is_ok(), "acquire should succeed after release");

        // 清理
        drop(guard3);
        POLL_LOCK.store(false, Ordering::Release);
    }

    #[test]
    fn test_next_poll_at_startup_falls_back_when_missing() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "45").unwrap();
        // No saved value → fallback to now → immediate check
        let result = calc_next_poll(&conn);
        assert_eq!(result, now);
    }

    // --- compute_fetch_plan tests ---

    #[test]
    fn test_compute_plan_no_history() {
        // fetch_history=false → third branch (Some(count), 10, false)
        let (max, per_page, paginate) = compute_fetch_plan(false, true, 50);
        assert_eq!(max, Some(50));
        assert_eq!(per_page, 10);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_not_first_query() {
        // !is_first_query → third branch
        let (max, per_page, paginate) = compute_fetch_plan(true, false, 50);
        assert_eq!(max, Some(50));
        assert_eq!(per_page, 10);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_0() {
        // count=0 → first branch: None = no limit, paginate = true
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 0);
        assert_eq!(max, None);
        assert_eq!(per_page, 100);
        assert!(paginate);
    }

    #[test]
    fn test_compute_plan_count_1() {
        // count=1 → second branch: small single-page fetch
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 1);
        assert_eq!(max, Some(1));
        // per_page = clamp(1+5, 10, 100) = 10
        assert_eq!(per_page, 10);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_50() {
        // count=50 → second branch
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 50);
        assert_eq!(max, Some(50));
        // per_page = clamp(50+5, 10, 100) = 55
        assert_eq!(per_page, 55);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_95() {
        // count=95 → second branch, per_page capped at 100
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 95);
        assert_eq!(max, Some(95));
        // per_page = clamp(95+5, 10, 100) = 100
        assert_eq!(per_page, 100);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_100() {
        // count=100 → second branch (since 100 > 100 is false)
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 100);
        assert_eq!(max, Some(100));
        // per_page = clamp(100+5, 10, 100) = 100
        assert_eq!(per_page, 100);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_101() {
        // count=101 → first branch, paginated with limit (safety net)
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 101);
        assert_eq!(max, Some(101));
        assert_eq!(per_page, 100);
        assert!(paginate);
    }
}
