use crate::db;
use crate::db::ai_usage::AiUsageStats;
use crate::db::usage::UsageStatRow;
use crate::error::DbResult;
use crate::types::AppState;

fn record_usage_inner(conn: &rusqlite::Connection, events: &[(String, u32)]) -> DbResult<()> {
    if !db::settings::get_setting_bool(conn, db::settings::KEY_ENABLE_USAGE_STATS, true)? {
        return Ok(());
    }
    db::usage::record_usage(conn, events)
}

#[tauri::command]
#[specta::specta]
pub async fn record_usage(
    state: tauri::State<'_, AppState>,
    events: Vec<(String, u32)>,
) -> Result<(), String> {
    db::run_blocking(&state.db, move |conn| Ok(record_usage_inner(conn, &events)?))
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_usage_stats(
    state: tauri::State<'_, AppState>,
    days: Option<u32>,
) -> Result<Vec<UsageStatRow>, String> {
    db::run_blocking(&state.db, move |conn| {
        Ok(db::usage::get_usage_stats(conn, days)?)
    })
    .await
    .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn clear_usage_stats(state: tauri::State<'_, AppState>) -> Result<(), String> {
    db::run_blocking(&state.db, |conn| Ok(db::usage::clear_usage_stats(conn)?))
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_usage_stats(
    state: tauri::State<'_, AppState>,
    source_id: Option<i64>,
    days: Option<u32>,
) -> Result<AiUsageStats, String> {
    db::run_blocking(&state.db, move |conn| {
        Ok(db::ai_usage::get_ai_usage_stats(conn, source_id, days)?)
    })
    .await
    .map_err(|e| e.to_ipc_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_record_usage_respects_enable_usage_stats_switch() {
        let conn = init_memory_db().unwrap();
        record_usage_inner(&conn, &[("source.add".into(), 1)]).unwrap();
        assert_eq!(db::usage::get_usage_stats(&conn, None).unwrap().len(), 1);

        db::settings::set_setting(&conn, db::settings::KEY_ENABLE_USAGE_STATS, "false").unwrap();
        record_usage_inner(&conn, &[("source.add".into(), 5)]).unwrap();
        let rows = db::usage::get_usage_stats(&conn, None).unwrap();
        assert_eq!(rows[0].total_count, 1);

        db::settings::set_setting(&conn, db::settings::KEY_ENABLE_USAGE_STATS, "true").unwrap();
        record_usage_inner(&conn, &[("source.add".into(), 2)]).unwrap();
        let rows = db::usage::get_usage_stats(&conn, None).unwrap();
        assert_eq!(rows[0].total_count, 3);
    }

    #[test]
    fn test_record_usage_empty_events_noop() {
        let conn = init_memory_db().unwrap();
        record_usage_inner(&conn, &[]).unwrap();
        assert!(db::usage::get_usage_stats(&conn, None).unwrap().is_empty());
    }

    #[test]
    fn test_usage_command_flow_roundtrip() {
        let conn = init_memory_db().unwrap();
        record_usage_inner(&conn, &[("source.check".into(), 2)]).unwrap();
        record_usage_inner(&conn, &[("release.open".into(), 1)]).unwrap();

        let all = db::usage::get_usage_stats(&conn, None).unwrap();
        let keys: Vec<&str> = all.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["source.check", "release.open"]);

        db::usage::clear_usage_stats(&conn).unwrap();
        assert!(db::usage::get_usage_stats(&conn, None).unwrap().is_empty());
    }
}
