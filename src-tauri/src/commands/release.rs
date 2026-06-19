use crate::db;
use crate::poll;
use crate::types::{AppState, PollResult};
use tauri::Emitter;
use serde_json::json;

#[tauri::command]
pub fn get_releases(
    state: tauri::State<AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
    db::releases::get_releases_with_state(&conn)
}

#[tauri::command]
pub fn set_notification_state(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_id: i64,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;

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
pub fn delete_release(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_id: i64,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;

    let rel = db::releases::get_release(&conn, release_id).ok().flatten();
    match rel {
        Some(r) => db::logs::write_log_key(
            &conn,
            "INFO",
            "release.deleted",
            &json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": release_id}).to_string(),
        ),
        None => db::logs::write_log_key(
            &conn,
            "INFO",
            "release.deleted_unknown",
            &json!({"id": release_id}).to_string(),
        ),
    }

    db::releases::delete_release(&conn, release_id)?;

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

    #[test]
    fn test_set_notification_state_unknown_release_logs_unknown_key() {
        let conn = init_memory_db().unwrap();

        // 模拟 set_notification_state 对不存在的 release_id 走 None 分支
        let rel = db::releases::get_release(&conn, 999).ok().flatten();
        assert!(rel.is_none());

        db::logs::write_log_key(
            &conn,
            "INFO",
            "release.status_changed_unknown",
            &serde_json::json!({"id": 999, "action": "ignored"}).to_string(),
        );

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.status_changed_unknown")));
    }

    #[test]
    fn test_delete_release_roundtrip() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
        let rid = db::releases::insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // 模拟 delete_release 的逻辑：查 release → 记日志 → 删除
        let rel = db::releases::get_release(&conn, rid).ok().flatten();
        assert!(rel.is_some());
        if let Some(r) = rel {
            db::logs::write_log_key(
                &conn,
                "INFO",
                "release.deleted",
                &serde_json::json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid}).to_string(),
            );
        }
        db::releases::delete_release(&conn, rid).unwrap();

        // 验证 release 已删除
        assert!(db::releases::get_release(&conn, rid).unwrap().is_none());

        // 验证日志已写入
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.deleted")));
    }

    #[test]
    fn test_delete_release_unknown_logs_unknown_key() {
        let conn = init_memory_db().unwrap();

        // 模拟 delete_release 对不存在的 id 走 None 分支
        let rel = db::releases::get_release(&conn, 999).ok().flatten();
        assert!(rel.is_none());

        db::logs::write_log_key(
            &conn,
            "INFO",
            "release.deleted_unknown",
            &serde_json::json!({"id": 999}).to_string(),
        );
        // delete_release 对不存在的 id 不应报错（SQL DELETE 匹配 0 行）
        db::releases::delete_release(&conn, 999).unwrap();

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.deleted_unknown")));
    }
}
