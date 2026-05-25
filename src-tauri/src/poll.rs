use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri::Emitter;

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

struct PollGuard;
impl Drop for PollGuard {
    fn drop(&mut self) {
        POLL_LOCK.store(false, Ordering::Release);
    }
}

fn acquire_lock() -> Result<PollGuard, String> {
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
    crypto::decrypt(&encrypted)
}

async fn fetch_for_source_async(
    client: &reqwest::Client,
    source: &db::sources::Source,
    per_page: usize,
) -> Result<Vec<serde_json::Value>, String> {
    match source.source_type.as_str() {
        "github" => github::fetch_releases(client, &source.owner, &source.repo, per_page).await,
        other => Err(format!("err.unsupported_source|{}", other)),
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

pub async fn trigger_poll(app: tauri::AppHandle) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;
    do_trigger_poll_async(app).await
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
        fetch_history_count = db::settings::get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1).unwrap_or(1).max(1) as usize;
    }

    let (all_new_ids, all_saved) = poll_all_sources_async(&app, &sources, &proxy_url, &proxy_mode, github_token.as_deref(), fetch_history, fetch_history_count).await;

    if !all_saved.is_empty() {
        deepseek::generate_summaries_for_new(&app, &all_saved).await;
    }

    let (_pending, new_releases) = collect_pending_and_notify(&app, &all_new_ids, true);

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
        fetch_history_count = db::settings::get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1).unwrap_or(1).max(1) as usize;
        source_obj = source;
    }

    let is_first_query = source_obj.last_checked_at.is_none()
        || source_obj.last_checked_at.as_deref() == Some("");
    let (max_count, per_page) = if fetch_history && is_first_query {
        (fetch_history_count, std::cmp::max(10, fetch_history_count + 5))
    } else {
        (fetch_history_count, 10)
    };

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
            return Err(e);
        }
    };

    let releases = match fetch_for_source_async(&client, &source_obj, per_page).await {
        Ok(releases) => releases,
        Err(e) => {
            let state = app.state::<AppState>();
            let conn = state.db.get().unwrap();
            let _ = db::sources::record_check_failure(&conn, source_obj.id, &e);
            db::logs::write_log_key(
                &conn,
                "ERROR",
                "check.failed",
                &json!({"owner": &source_obj.owner, "repo": &source_obj.repo, "error": &e}).to_string(),
            );
            return Err(e);
        }
    };
    let saved: Vec<(i64, Option<String>)>;
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        saved = save_for_source(&conn, &source_obj, &releases, max_count);
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
        fetch_history_count = db::settings::get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1).unwrap_or(1).max(1) as usize;
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

    let (all_new_ids, all_saved) = poll_all_sources_async(&app, &enabled, &proxy_url, &proxy_mode, github_token.as_deref(), fetch_history, fetch_history_count).await;

    if !all_saved.is_empty() {
        deepseek::generate_summaries_for_new(&app, &all_saved).await;
    }

    collect_pending_and_notify(&app, &all_new_ids, false);

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
        let (max_count, per_page) = if fetch_history && is_first_query {
            (fetch_history_count, std::cmp::max(10, fetch_history_count + 5))
        } else {
            (fetch_history_count, 10)
        };

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            match fetch_for_source_async(&client, &source, per_page).await {
                Ok(releases) => {
                    let state = app.state::<AppState>();
                    let conn = state.db.get().unwrap();
                    let saved = save_for_source(&conn, &source, &releases, max_count);
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
                Err(e) => {
                    let state = app.state::<AppState>();
                    let conn = state.db.get().unwrap();
                    let _ = db::sources::record_check_failure(&conn, source.id, &e);
                    db::logs::write_log_key(
                        &conn,
                        "ERROR",
                        "check.failed",
                        &json!({"owner": &source.owner, "repo": &source.repo, "error": &e}).to_string(),
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
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        pending = db::releases::get_pending_releases(&conn).unwrap_or_default();
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
        let (enabled, model, base_url, api_key) = deepseek::read_config(&conn);
        assert!(!enabled);
        assert_eq!(model, "deepseek-v4-flash");
        assert_eq!(base_url, "https://api.deepseek.com");
        assert!(api_key.is_none());
    }

    #[test]
    fn test_read_deepseek_config_configured() {
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_MODEL, "deepseek-v4-pro").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, "https://custom.api").unwrap();
        let encrypted = crate::crypto::encrypt("sk-test");
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_API_KEY, &encrypted).unwrap();

        let (enabled, model, base_url, api_key) = deepseek::read_config(&conn);
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
    fn test_next_poll_at_startup_falls_back_when_missing() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "45").unwrap();
        // No saved value → fallback to now → immediate check
        let result = calc_next_poll(&conn);
        assert_eq!(result, now);
    }
}
