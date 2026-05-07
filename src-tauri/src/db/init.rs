use rusqlite::{Connection, Result};
use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .expect("Failed to get app data dir")
        .join("RelWatch")
}

pub fn db_path() -> PathBuf {
    app_data_dir().join("database.db")
}

pub fn init_memory_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_type TEXT NOT NULL,
            owner TEXT NOT NULL,
            repo TEXT NOT NULL,
            poll_interval_minutes INTEGER NOT NULL DEFAULT 30,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(source_type, owner, repo)
        );

        CREATE TABLE IF NOT EXISTS releases (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id INTEGER NOT NULL,
            tag_name TEXT NOT NULL,
            release_name TEXT NOT NULL,
            html_url TEXT NOT NULL,
            published_at TEXT NOT NULL,
            prerelease INTEGER NOT NULL DEFAULT 0,
            body TEXT,
            detected_at TEXT NOT NULL,
            UNIQUE(source_id, tag_name),
            FOREIGN KEY (source_id) REFERENCES sources(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS notification_state (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            release_id INTEGER NOT NULL UNIQUE,
            status TEXT NOT NULL DEFAULT 'pending',
            snooze_until TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            level TEXT NOT NULL,
            message TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

pub fn init_db() -> Result<Connection> {
    let dir = app_data_dir();
    std::fs::create_dir_all(&dir).expect("Failed to create app data dir");

    let conn = Connection::open(db_path())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA wal_autocheckpoint=1000;
         PRAGMA foreign_keys=ON;",
    )?;
    apply_schema(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let has_summary: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('releases') WHERE name='ai_summary'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_summary {
        conn.execute_batch(
            "ALTER TABLE releases ADD COLUMN ai_summary TEXT;
             ALTER TABLE releases ADD COLUMN ai_importance TEXT;",
        )?;
    }
    let has_msg_key: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('logs') WHERE name='message_key'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_msg_key {
        conn.execute_batch(
            "ALTER TABLE logs ADD COLUMN message_key TEXT;
             ALTER TABLE logs ADD COLUMN message_args TEXT;",
        )?;
    }
    Ok(())
}
