use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    pub id: i64,
    pub level: String,
    pub message: String,
    pub created_at: String,
    pub message_key: Option<String>,
    pub message_args: Option<String>,
}

pub fn write_log(conn: &Connection, level: &str, message: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = conn.execute(
        "INSERT INTO logs (level, message, created_at) VALUES (?1, ?2, ?3)",
        params![level, message, now],
    );
}

pub fn write_log_key(conn: &Connection, level: &str, key: &str, args: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = conn.execute(
        "INSERT INTO logs (level, message, message_key, message_args, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![level, key, key, args, now],
    );
}

pub fn get_logs(conn: &Connection, limit: i64) -> Result<Vec<LogEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, level, message, message_key, message_args, created_at FROM logs ORDER BY id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;

    let logs = stmt
        .query_map(params![limit], |row| {
            Ok(LogEntry {
                id: row.get(0)?,
                level: row.get(1)?,
                message: row.get(2)?,
                message_key: row.get(3)?,
                message_args: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(logs)
}

pub fn delete_old_logs(conn: &Connection, days: i64) {
    if days <= 0 {
        return;
    }
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let _ = conn.execute("DELETE FROM logs WHERE created_at < ?1", rusqlite::params![cutoff]);
}

pub fn clear_logs(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM logs", rusqlite::params![])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_logs_write_and_read() {
        let conn = init_memory_db().unwrap();
        write_log(&conn, "INFO", "msg1");
        write_log(&conn, "ERROR", "msg2");
        write_log_key(&conn, "INFO", "test.key", r#"{"a":"1"}"#);
        let logs = get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].message, "test.key");
        assert_eq!(logs[0].message_key.as_deref(), Some("test.key"));
        assert_eq!(logs[0].message_args.as_deref(), Some(r#"{"a":"1"}"#));
    }

    #[test]
    fn test_logs_boundary() {
        let conn = init_memory_db().unwrap();
        write_log(&conn, "INFO", "msg1");
        write_log(&conn, "ERROR", "msg2");

        let logs = get_logs(&conn, 0).unwrap();
        assert_eq!(logs.len(), 0);

        let logs = get_logs(&conn, 100).unwrap();
        assert_eq!(logs.len(), 2);
    }
}
