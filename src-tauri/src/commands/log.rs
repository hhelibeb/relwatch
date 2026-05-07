use crate::db;
use crate::types::AppState;

#[tauri::command]
pub fn get_logs(state: tauri::State<AppState>, limit: i64) -> Result<Vec<db::logs::LogEntry>, String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::logs::get_logs(&conn, limit)
}

#[tauri::command]
pub fn clear_logs(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::logs::clear_logs(&conn)?;
    db::logs::write_log_key(&conn, "INFO", "log.cleared", "{}");
    Ok(())
}
