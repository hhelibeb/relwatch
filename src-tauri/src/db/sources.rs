use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Source {
    pub id: i64,
    pub source_type: String,
    pub owner: String,
    pub repo: String,
    pub poll_interval_minutes: i64,
    pub enabled: bool,
    pub last_checked_at: Option<String>,
    pub last_check_status: String,
    pub last_check_message: Option<String>,
    pub consecutive_failures: i64,
    pub last_new_count: i64,
    pub muted: bool,
    pub created_at: String,
    pub updated_at: String,
    pub description: Option<String>,
}

pub fn add_source(
    conn: &Connection,
    source_type: &str,
    owner: &str,
    repo: &str,
    description: &str,
) -> Result<i64, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let desc = if description.is_empty() { None } else { Some(description) };
    conn.execute(
        "INSERT OR IGNORE INTO sources (source_type, owner, repo, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![source_type, owner, repo, desc, now, now],
    )
    .map_err(|e| e.to_string())?;
    if conn.changes() == 0 {
        return Ok(0);
    }
    Ok(conn.last_insert_rowid())
}

pub fn source_exists(
    conn: &Connection,
    source_type: &str,
    owner: &str,
    repo: &str,
) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(
            "SELECT 1 FROM sources
             WHERE source_type = ?1 AND lower(owner) = lower(?2) AND lower(repo) = lower(?3)
             LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    stmt.exists(params![source_type, owner, repo])
        .map_err(|e| e.to_string())
}

pub fn remove_source(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM sources WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_source(conn: &Connection, id: i64) -> Result<Option<Source>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source_type, owner, repo, poll_interval_minutes, enabled,
                    last_checked_at, last_check_status, last_check_message,
                    consecutive_failures, last_new_count, muted, created_at, updated_at,
                    description
             FROM sources WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(Source {
                id: row.get(0)?,
                source_type: row.get(1)?,
                owner: row.get(2)?,
                repo: row.get(3)?,
                poll_interval_minutes: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_checked_at: row.get(6)?,
                last_check_status: row.get(7)?,
                last_check_message: row.get(8)?,
                consecutive_failures: row.get(9)?,
                last_new_count: row.get(10)?,
                muted: row.get::<_, i64>(11)? != 0,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                description: row.get(14)?,
            })
        })
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok(source)) => Ok(Some(source)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

pub fn update_source(
    conn: &Connection,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE sources SET enabled = ?1, poll_interval_minutes = ?2, updated_at = ?3 WHERE id = ?4",
        params![enabled as i64, poll_interval_minutes, now, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_source_muted(
    conn: &Connection,
    id: i64,
    muted: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE sources SET muted = ?1, updated_at = ?2 WHERE id = ?3",
        params![muted as i64, chrono::Utc::now().to_rfc3339(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_muted_source_ids(conn: &Connection) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM sources WHERE muted = 1")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(ids)
}

pub fn record_check_success(conn: &Connection, id: i64, new_count: usize) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE sources
         SET last_checked_at = ?1,
             last_check_status = 'ok',
             last_check_message = NULL,
             consecutive_failures = 0,
             last_new_count = ?2,
             updated_at = ?1
         WHERE id = ?3",
        params![now, new_count as i64, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_check_failure(conn: &Connection, id: i64, message: &str) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let trimmed: String = message.chars().take(500).collect();
    conn.execute(
        "UPDATE sources
         SET last_checked_at = ?1,
             last_check_status = 'error',
             last_check_message = ?2,
             consecutive_failures = consecutive_failures + 1,
             last_new_count = 0,
             updated_at = ?1
         WHERE id = ?3",
        params![now, trimmed, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_source_description(conn: &Connection, id: i64, description: &str) -> Result<(), String> {
    let desc = if description.is_empty() { None } else { Some(description) };
    conn.execute(
        "UPDATE sources SET description = ?1, updated_at = ?2 WHERE id = ?3",
        params![desc, chrono::Utc::now().to_rfc3339(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source_type, owner, repo, poll_interval_minutes, enabled,
                    last_checked_at, last_check_status, last_check_message,
                    consecutive_failures, last_new_count, muted, created_at, updated_at,
                    description
             FROM sources ORDER BY id DESC",
        )
        .map_err(|e| e.to_string())?;

    let sources = stmt
        .query_map([], |row| {
            Ok(Source {
                id: row.get(0)?,
                source_type: row.get(1)?,
                owner: row.get(2)?,
                repo: row.get(3)?,
                poll_interval_minutes: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_checked_at: row.get(6)?,
                last_check_status: row.get(7)?,
                last_check_message: row.get(8)?,
                consecutive_failures: row.get(9)?,
                last_new_count: row.get(10)?,
                muted: row.get::<_, i64>(11)? != 0,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                description: row.get(14)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_remove_nonexistent_source() {
        let conn = init_memory_db().unwrap();
        // 删除不存在的 id 不应报错
        let result = remove_source(&conn, 999);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_nonexistent_source() {
        let conn = init_memory_db().unwrap();
        let result = get_source(&conn, 999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_nonexistent_source() {
        let conn = init_memory_db().unwrap();
        // 更新不存在的 id 不应报错（0 rows affected）
        let result = update_source(&conn, 999, false, 60);
        assert!(result.is_ok());
    }

    #[test]
    fn test_record_check_failure_truncates_long_message() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        // 构造超过 500 字符的错误消息
        let long_msg = "x".repeat(1000);
        record_check_failure(&conn, id, &long_msg).unwrap();

        let s = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(s.last_check_status, "error");
        assert!(s.last_check_message.is_some());
        // 应被截断到 500 字符
        assert!(s.last_check_message.as_ref().unwrap().len() <= 500);
        assert_eq!(s.consecutive_failures, 1);
    }

    #[test]
    fn test_source_add_and_list() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "microsoft", "vscode", "Code editor").unwrap();
        assert!(id > 0);
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].owner, "microsoft");
        assert!(sources[0].enabled);
        assert_eq!(sources[0].last_check_status, "unknown");
        assert_eq!(sources[0].consecutive_failures, 0);
    }

    #[test]
    fn test_source_remove() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "a", "b", "").unwrap();
        remove_source(&conn, id).unwrap();
        assert_eq!(list_sources(&conn).unwrap().len(), 0);
    }

    #[test]
    fn test_source_update() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();
        update_source(&conn, id, false, 60).unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert!(!s.enabled);
        assert_eq!(s.poll_interval_minutes, 60);
    }

    #[test]
    fn test_source_health_success_and_failure() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        record_check_failure(&conn, id, "network failed").unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert_eq!(s.last_check_status, "error");
        assert_eq!(s.last_check_message.as_deref(), Some("network failed"));
        assert_eq!(s.consecutive_failures, 1);
        assert_eq!(s.last_new_count, 0);
        assert!(s.last_checked_at.is_some());

        record_check_success(&conn, id, 3).unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert_eq!(s.last_check_status, "ok");
        assert!(s.last_check_message.is_none());
        assert_eq!(s.consecutive_failures, 0);
        assert_eq!(s.last_new_count, 3);
    }

    #[test]
    fn test_source_muted_default_false() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.muted, "新建源的 muted 应默认为 false");
    }

    #[test]
    fn test_source_set_muted_toggle() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "a", "b", "").unwrap();

        // 切换为静默
        set_source_muted(&conn, id, true).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.muted);

        // 取消静默
        set_source_muted(&conn, id, false).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.muted);
    }

    #[test]
    fn test_source_muted_independent_of_enabled() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        // 先设为静默
        set_source_muted(&conn, id, true).unwrap();

        // 暂停（enabled=false）
        update_source(&conn, id, false, 30).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.enabled);
        assert!(s.muted, "muted 应跨暂停保留");

        // 恢复（enabled=true）后仍为静默
        update_source(&conn, id, true, 30).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.enabled);
        assert!(s.muted, "muted 应在恢复后仍然保持");
    }

    #[test]
    fn test_add_source_duplicate() {
        let conn = init_memory_db().unwrap();
        let id1 = add_source(&conn, "github", "microsoft", "vscode", "Code editor").unwrap();
        assert!(id1 > 0);
        let id2 = add_source(&conn, "github", "microsoft", "vscode", "Code editor").unwrap();
        assert_eq!(id2, 0);
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
    }
}
