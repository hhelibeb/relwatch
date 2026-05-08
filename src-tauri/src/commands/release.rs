use crate::db;
use crate::poll;
use crate::types::{AppState, PollResult};
use serde_json::json;

#[tauri::command]
pub fn get_releases(
    state: tauri::State<AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    let conn = state.db.get().unwrap();
    db::releases::get_releases_with_state(&conn)
}

#[tauri::command]
pub fn get_pending_releases(
    state: tauri::State<AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    let conn = state.db.get().unwrap();
    db::releases::get_pending_releases(&conn)
}

#[tauri::command]
pub fn set_notification_state(
    state: tauri::State<AppState>,
    release_id: i64,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<(), String> {
    let conn = state.db.get().unwrap();

    let snooze_until = snooze_minutes.map(|minutes| {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        until.to_rfc3339()
    });

    let snooze_str = snooze_until.as_deref();
    db::releases::set_notification_state(&conn, release_id, &status, snooze_str)?;

    let rel = db::releases::get_release(&conn, release_id).ok().flatten();
    match rel {
        Some(r) => db::logs::write_log_key(&conn, "INFO", "release.status_changed", &json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": release_id, "action": &status}).to_string()),
        None => db::logs::write_log_key(&conn, "INFO", "release.status_changed_unknown", &json!({"id": release_id, "action": &status}).to_string()),
    }
    Ok(())
}

#[tauri::command]
pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    poll::check_single_source(app, id).await
}
