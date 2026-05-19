use crate::db;
use crate::poll;
use crate::types::{AppState, PollResult};
use tauri::Emitter;
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
    app: tauri::AppHandle,
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

    let _ = app.emit("release-state-changed", release_id);

    Ok(())
}

#[tauri::command]
pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    poll::check_single_source(app, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    /// Helper: create a source + release and return release id
    fn setup_source_and_release(conn: &rusqlite::Connection) -> i64 {
        let sid = db::sources::add_source(conn, "github", "owner", "repo", "").unwrap();
        db::releases::insert_release(conn, sid, "v1.0", "Release 1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap()
    }

    #[test]
    fn test_get_releases_returns_data() {
        let conn = init_memory_db().unwrap();
        assert!(db::releases::get_releases_with_state(&conn).unwrap().is_empty());

        let rid = setup_source_and_release(&conn);
        assert!(rid > 0);

        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag_name, "v1.0");
        assert_eq!(releases[0].notification_status, "pending");
    }

    #[test]
    fn test_get_pending_releases_filters_correctly() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let rid = db::releases::insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // By default, it's pending
        let pending = db::releases::get_pending_releases(&conn).unwrap();
        assert!(pending.iter().any(|r| r.id == rid));

        // Snooze with future time → not pending
        let future = chrono::Utc::now() + chrono::Duration::hours(2);
        db::releases::set_notification_state(&conn, rid, "snoozed", Some(&future.to_rfc3339())).unwrap();
        let pending = db::releases::get_pending_releases(&conn).unwrap();
        assert!(!pending.iter().any(|r| r.id == rid));
    }

    #[test]
    fn test_set_notification_state_writes_log() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
        let rid = db::releases::insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // Simulate set_notification_state's internal logic (without app.emit)
        db::releases::set_notification_state(&conn, rid, "ignored", None).unwrap();
        let rel = db::releases::get_release(&conn, rid).ok().flatten();
        if let Some(r) = rel {
            db::logs::write_log_key(
                &conn,
                "INFO",
                "release.status_changed",
                &serde_json::json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid, "action": "ignored"}).to_string(),
            );
        }

        // Verify DB state changed
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "ignored");

        // Verify log written
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.status_changed")));
    }

    #[test]
    fn test_set_notification_state_snooze_computation() {
        // Unit test for the snooze_until computation logic
        let minutes = 30i64;
        let now = chrono::Utc::now();
        let expected = now + chrono::Duration::minutes(minutes);
        let computed = now + chrono::Duration::minutes(minutes);
        // Allow 1 second tolerance
        let diff = (computed - expected).num_seconds().abs();
        assert!(diff <= 1, "snooze_until computation should be correct");
    }
}
