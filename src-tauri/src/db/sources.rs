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
    pub created_at: String,
    pub updated_at: String,
}

pub fn add_source(
    conn: &Connection,
    source_type: &str,
    owner: &str,
    repo: &str,
) -> Result<i64, String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO sources (source_type, owner, repo, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![source_type, owner, repo, now, now],
    )
    .map_err(|e| e.to_string())?;
    if conn.changes() == 0 {
        return Ok(0);
    }
    Ok(conn.last_insert_rowid())
}

pub fn remove_source(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM sources WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
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

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source_type, owner, repo, poll_interval_minutes, enabled, created_at, updated_at
             FROM sources ORDER BY id",
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
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
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
    fn test_source_add_and_list() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "microsoft", "vscode").unwrap();
        assert!(id > 0);
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].owner, "microsoft");
        assert!(sources[0].enabled);
    }

    #[test]
    fn test_source_remove() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "a", "b").unwrap();
        remove_source(&conn, id).unwrap();
        assert_eq!(list_sources(&conn).unwrap().len(), 0);
    }

    #[test]
    fn test_source_update() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y").unwrap();
        update_source(&conn, id, false, 60).unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert!(!s.enabled);
        assert_eq!(s.poll_interval_minutes, 60);
    }

    #[test]
    fn test_add_source_duplicate() {
        let conn = init_memory_db().unwrap();
        let id1 = add_source(&conn, "github", "microsoft", "vscode").unwrap();
        assert!(id1 > 0);
        let id2 = add_source(&conn, "github", "microsoft", "vscode").unwrap();
        assert_eq!(id2, 0);
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
    }
}
