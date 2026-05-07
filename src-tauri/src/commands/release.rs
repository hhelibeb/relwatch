use crate::db;
use crate::poll;
use crate::types::{AppState, PollResult};

#[tauri::command]
pub fn get_releases(
    state: tauri::State<AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::releases::get_releases_with_state(&conn)
}

#[tauri::command]
pub fn get_pending_releases(
    state: tauri::State<AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::releases::get_pending_releases(&conn)
}

#[tauri::command]
pub fn set_notification_state(
    state: tauri::State<AppState>,
    release_id: i64,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());

    let snooze_until = snooze_minutes.map(|minutes| {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        until.to_rfc3339()
    });

    let snooze_str = snooze_until.as_deref();
    db::releases::set_notification_state(&conn, release_id, &status, snooze_str)?;

    let rel = db::releases::get_release(&conn, release_id).ok().flatten();
    let action = match status.as_str() {
        "ignored" => "忽略",
        "snoozed" => "推迟",
        "clicked" => "已查看",
        _ => &status,
    };
    match rel {
        Some(r) => db::logs::write_log(&conn, "INFO", &format!("{}/{} {} 已{}(id={})", r.owner, r.repo, r.tag_name, action, release_id)),
        None => db::logs::write_log(&conn, "INFO", &format!("版本 id={} 状态: {}", release_id, action)),
    }
    Ok(())
}

#[tauri::command]
pub fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    poll::check_single_source(app, id)
}
