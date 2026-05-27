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

pub fn search_logs(
    conn: &Connection,
    keyword: &str,
    page: i64,
    page_size: i64,
) -> Result<(Vec<LogEntry>, i64), String> {
    let offset = (page - 1) * page_size;
    let has_keyword = !keyword.is_empty();

    let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if has_keyword {
        let pattern = format!("%{}%", keyword);
        (
            "SELECT id, level, message, message_key, message_args, created_at, COUNT(*) OVER() as total
             FROM logs
             WHERE message LIKE ?1 OR level LIKE ?1 OR message_key LIKE ?1
             ORDER BY id DESC
             LIMIT ?2 OFFSET ?3"
                .to_string(),
            vec![
                Box::new(pattern) as Box<dyn rusqlite::types::ToSql>,
                Box::new(page_size),
                Box::new(offset),
            ],
        )
    } else {
        (
            "SELECT id, level, message, message_key, message_args, created_at, COUNT(*) OVER() as total
             FROM logs
             ORDER BY id DESC
             LIMIT ?1 OFFSET ?2"
                .to_string(),
            vec![
                Box::new(page_size) as Box<dyn rusqlite::types::ToSql>,
                Box::new(offset),
            ],
        )
    };

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let mut total: i64 = 0;
    let logs = stmt
        .query_map(params_refs.as_slice(), |row| {
            total = row.get(6)?;
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

    Ok((logs, total))
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
