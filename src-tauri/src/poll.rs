use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri::Emitter;

use crate::db;
use crate::db::settings::{KEY_POLL_INTERVAL, KEY_PROXY_URL, KEY_LOG_RETENTION, KEY_GITHUB_TOKEN, KEY_NEXT_POLL_AT};
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
) -> Result<Vec<serde_json::Value>, String> {
    match source.source_type.as_str() {
        "github" => github::fetch_releases(client, &source.owner, &source.repo).await,
        other => Err(format!("err.unsupported_source|{}", other)),
    }
}

fn save_for_source(
    conn: &rusqlite::Connection,
    source: &db::sources::Source,
    data: &[serde_json::Value],
) -> Vec<(i64, Option<String>)> {
    match source.source_type.as_str() {
        "github" => github::save_releases(conn, source.id, data),
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
    let (sources, proxy_url, github_token);

    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        sources = db::sources::list_sources(&conn)?
            .into_iter()
            .filter(|s| s.enabled)
            .collect::<Vec<_>>();
        proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        github_token = get_github_token(&conn);
    }

    let (all_new_ids, all_saved) = poll_all_sources_async(&app, &sources, &proxy_url, github_token.as_deref()).await;

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

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}

pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;

    let (client, source_obj);
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        let sources = db::sources::list_sources(&conn)?;
        let source = sources
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("err.source_not_found")?;
        let proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        let github_token = get_github_token(&conn);
        drop(conn);
        client = http::build_http_client(&proxy_url, github_token.as_deref())?;
        source_obj = source;
    }

    let releases = fetch_for_source_async(&client, &source_obj).await?;
    let saved: Vec<(i64, Option<String>)>;
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        saved = save_for_source(&conn, &source_obj, &releases);
        db::logs::write_log_key(
            &conn,
            "INFO",
            "check.manual",
            &json!({"owner": &source_obj.owner, "repo": &source_obj.repo, "count": saved.len()}).to_string(),
        );
    }

    let new_ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();

    if !saved.is_empty() {
        deepseek::generate_summaries_for_new(&app, &saved).await;
    }

    let (_, new_releases) = collect_pending_and_notify(&app, &new_ids, false);

    Ok(PollResult { new_releases })
}

#[allow(dead_code)]
pub fn do_poll(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        do_poll_async(app).await;
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

    let (sources, proxy_url, retention_days, github_token);
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        sources = db::sources::list_sources(&conn).unwrap_or_default();
        proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)
            .ok()
            .flatten()
            .unwrap_or_default();
        retention_days = db::settings::get_setting(&conn, KEY_LOG_RETENTION)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if retention_days > 0 {
            db::logs::delete_old_logs(&conn, retention_days);
        }
        github_token = get_github_token(&conn);
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

    let (all_new_ids, all_saved) = poll_all_sources_async(&app, &enabled, &proxy_url, github_token.as_deref()).await;

    if !all_saved.is_empty() {
        deepseek::generate_summaries_for_new(&app, &all_saved).await;
    }

    if !all_new_ids.is_empty() {
        collect_pending_and_notify(&app, &all_new_ids, false);
    }

    let _ = app.emit("poll-completed", ());
}

async fn poll_all_sources_async(
    app: &tauri::AppHandle,
    sources: &[db::sources::Source],
    proxy_url: &str,
    github_token: Option<&str>,
) -> (Vec<i64>, Vec<(i64, Option<String>)>) {
    let client = match http::build_http_client(proxy_url, github_token) {
        Ok(c) => c,
        Err(e) => {
            let state = app.state::<AppState>();
            if let Ok(conn) = state.db.get() {
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

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            match fetch_for_source_async(&client, &source).await {
                Ok(releases) => {
                    let state = app.state::<AppState>();
                    let conn = state.db.get().unwrap();
                    let saved = save_for_source(&conn, &source, &releases);
                    let ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();
                    let new_count = ids.len();
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

    for release in &pending {
        if !new_ids.contains(&release.id) {
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
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_MODEL, "deepseek-v3").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, "https://custom.api").unwrap();
        let encrypted = crate::crypto::encrypt("sk-test");
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_API_KEY, &encrypted).unwrap();

        let (enabled, model, base_url, api_key) = deepseek::read_config(&conn);
        assert!(enabled);
        assert_eq!(model, "deepseek-v3");
        assert_eq!(base_url, "https://custom.api");
        assert_eq!(api_key.unwrap(), "sk-test");
    }

    use crate::http;

    #[test]
    fn test_build_http_client_empty() {
        let result = http::build_http_client("", None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_invalid() {
        let result = http::build_http_client("://invalid", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid proxy URL"));
    }

    #[test]
    fn test_build_http_client_valid() {
        let result = http::build_http_client("http://127.0.0.1:1080", None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_success() {
        let result = deepseek::build_client("sk-test", "");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_with_proxy() {
        let result = deepseek::build_client("sk-test", "http://127.0.0.1:1080");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_invalid_key() {
        let result = deepseek::build_client("key\nwith\nnewlines", "");
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
            .unwrap_or_else(|| {
                let interval = db::settings::get_setting(conn, KEY_POLL_INTERVAL)
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(30);
                now + interval * 60
            })
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
        // Saved next_poll_at is 10 min in the past → expired, fallback to now+30m
        db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now - 10 * 60).to_string()).unwrap();
        let result = calc_next_poll(&conn);
        assert_eq!(result, now + 30 * 60);
    }

    #[test]
    fn test_next_poll_at_startup_falls_back_when_missing() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "45").unwrap();
        // No saved value → fallback to now + interval
        let result = calc_next_poll(&conn);
        assert_eq!(result, now + 45 * 60);
    }
}
