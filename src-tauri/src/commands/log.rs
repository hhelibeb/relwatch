use crate::db;
use crate::types::{AppState, LogSearchResult};

#[tauri::command]
pub fn get_logs(state: tauri::State<AppState>, limit: i64) -> Result<Vec<db::logs::LogEntry>, String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
    db::logs::get_logs(&conn, limit)
}

#[tauri::command]
pub fn search_logs(
    state: tauri::State<AppState>,
    keyword: String,
    page: i64,
    page_size: i64,
    level: Option<String>,
) -> Result<LogSearchResult, String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
    let (entries, total) = db::logs::search_logs(&conn, &keyword, level.as_deref(), page, page_size)?;
    Ok(LogSearchResult {
        entries,
        total,
        page,
        page_size,
    })
}

#[tauri::command]
pub fn clear_logs(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
    db::logs::clear_logs(&conn)?;
    db::logs::write_log_key(&conn, "INFO", "log.cleared", "{}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_get_logs_returns_written_logs() {
        let conn = init_memory_db().unwrap();
        db::logs::write_log_key(&conn, "INFO", "test.message", "{\"key\":\"val\"}");
        db::logs::write_log_key(&conn, "WARN", "test.warn", "{}");

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("test.message")));
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("test.warn")));
    }

    #[test]
    fn test_get_logs_respects_limit() {
        let conn = init_memory_db().unwrap();
        db::logs::write_log_key(&conn, "INFO", "msg1", "{}");
        db::logs::write_log_key(&conn, "INFO", "msg2", "{}");
        db::logs::write_log_key(&conn, "INFO", "msg3", "{}");

        // Limit 2 should only return 2 newest
        let logs = db::logs::get_logs(&conn, 2).unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn test_clear_logs_empties_and_writes_log() {
        let conn = init_memory_db().unwrap();
        db::logs::write_log_key(&conn, "INFO", "test.message", "{}");
        assert!(!db::logs::get_logs(&conn, 10).unwrap().is_empty());

        // Simulate clear_logs internal logic
        db::logs::clear_logs(&conn).unwrap();
        db::logs::write_log_key(&conn, "INFO", "log.cleared", "{}");

        // After clear, only the "log.cleared" entry should exist
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message_key.as_deref(), Some("log.cleared"));
    }
}
