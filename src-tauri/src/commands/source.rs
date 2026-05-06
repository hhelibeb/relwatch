use crate::db;
use crate::types::AppState;

#[tauri::command]
pub fn add_source(
    state: tauri::State<AppState>,
    source_type: String,
    owner: String,
    repo: String,
) -> Result<i64, String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    let id = db::sources::add_source(&conn, &source_type, &owner, &repo)?;
    db::logs::write_log(
        &conn,
        "INFO",
        &format!("添加监控源: {} {}/{}", source_type, owner, repo),
    );
    Ok(id)
}

#[tauri::command]
pub fn remove_source(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::sources::remove_source(&conn, id)?;
    db::logs::write_log(&conn, "INFO", &format!("移除监控源 id={}", id));
    Ok(())
}

#[tauri::command]
pub fn update_source(
    state: tauri::State<AppState>,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::sources::update_source(&conn, id, enabled, poll_interval_minutes)?;
    db::logs::write_log(&conn, "INFO", &format!("更新监控源 id={}", id));
    Ok(())
}

#[tauri::command]
pub fn list_sources(state: tauri::State<AppState>) -> Result<Vec<db::sources::Source>, String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::sources::list_sources(&conn)
}
