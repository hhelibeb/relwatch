use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::backup::Backup;
use rusqlite::Connection;
use serde_json::json;

use crate::agent_gate::ConcurrencyGate;
use crate::db::DbPool;
use crate::error::{AppError, AppResult, DbError};

fn sibling_temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    target.with_file_name(name)
}

/// 先导出到同目录临时文件再改名覆盖：VACUUM INTO 拒绝非空目标文件，
/// 同卷 rename 保证覆盖语义且失败时保留旧备份。
fn vacuum_into(conn: &Connection, target: &Path) -> AppResult<()> {
    let tmp = sibling_temp_path(target);
    let _ = std::fs::remove_file(&tmp);

    let tmp_str = tmp.to_string_lossy();
    if let Err(e) = conn.execute("VACUUM INTO ?1", rusqlite::params![tmp_str.as_ref()]) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::validation_args(
            "err.backup_export_failed",
            [e.to_string()],
        ));
    }

    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::validation_args("err.backup_export_failed", [e.to_string()])
    })
}

pub fn validate_sqlite_file(path: &str) -> AppResult<()> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AppError::validation_args("err.backup_read_failed", [e.to_string()]))?;
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(AppError::validation("err.backup_invalid_file"));
        }
        Err(e) => {
            return Err(AppError::validation_args(
                "err.backup_read_failed",
                [e.to_string()],
            ));
        }
    }
    if &header != b"SQLite format 3\0" {
        return Err(AppError::validation("err.backup_invalid_file"));
    }
    Ok(())
}

pub async fn export_backup_to_path(pool: &DbPool, target: PathBuf) -> AppResult<String> {
    let path_str = target.to_string_lossy().to_string();
    let log_path = path_str.clone();
    let pool = pool.clone();

    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let conn = pool.get().map_err(DbError::Connect)?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| {
                AppError::validation_args("err.backup_wal_checkpoint_failed", [e.to_string()])
            })?;
        vacuum_into(&conn, &target)?;
        crate::db::logs::write_log_key(
            &conn,
            "INFO",
            "backup.exported",
            &json!({"path": &log_path}).to_string(),
        );
        Ok(())
    })
    .await
    .map_err(|e| AppError::validation_args("err.backup_export_failed", [e.to_string()]))??;

    Ok(path_str)
}

pub async fn import_backup_from_path(
    pool: &DbPool,
    agent_gate: &Arc<ConcurrencyGate>,
    src_path: PathBuf,
) -> AppResult<()> {
    let path_str = src_path.to_string_lossy().to_string();
    let pool = pool.clone();
    let agent_gate = agent_gate.clone();

    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let _poll_guard = crate::poll::acquire_lock()
            .map_err(|_| AppError::validation("err.backup_import_busy"))?;

        validate_sqlite_file(&path_str)?;

        let src_conn = Connection::open(&path_str)
            .map_err(|e| AppError::validation_args("err.backup_open_failed", [e.to_string()]))?;

        let mut dst_conn = pool.get().map_err(DbError::Connect)?;

        // Backup API 为页级整库替换，不预删目标表以免中途失败丢数据
        {
            let backup = Backup::new(&src_conn, &mut dst_conn).map_err(|e| {
                AppError::validation_args("err.backup_session_failed", [e.to_string()])
            })?;
            backup
                .run_to_completion(100, Duration::from_millis(250), None)
                .map_err(|e| {
                    AppError::validation_args("err.backup_restore_failed", [e.to_string()])
                })?;
        }
        drop(src_conn);

        crate::db::init::apply_schema(&dst_conn)
            .map_err(|e| AppError::validation_args("err.backup_reinit_failed", [e.to_string()]))?;
        crate::db::init::migrate(&dst_conn).map_err(|e| {
            AppError::validation_args("err.backup_migrate_failed", [e.to_string()])
        })?;
        let cleared_keys = crate::crypto::verify_master_key_consistency(&dst_conn);
        if !cleared_keys.is_empty() {
            eprintln!(
                "WARNING: 导入的备份中以下加密设置无法用本机 master key 解密，已自动清空，请重新配置: {}",
                cleared_keys.join(", ")
            );
        }

        agent_gate.sync_from_db(&dst_conn);
        crate::db::logs::write_log_key(
            &dst_conn,
            "INFO",
            "backup.imported",
            &json!({"path": &path_str}).to_string(),
        );
        Ok(())
    })
    .await
    .map_err(|e| AppError::validation_args("err.backup_restore_failed", [e.to_string()]))??;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_pool;
    use std::io::Write;

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("relwatch-backup-test-{}-{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn seed_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
             INSERT INTO t VALUES (1, 'hello');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn validate_sqlite_file_checks_magic_header_and_io_errors() {
        let dir = TempDirGuard::new("validate");

        let valid = dir.path().join("valid.db");
        {
            let conn = Connection::open(&valid).unwrap();
            conn.execute_batch("CREATE TABLE t (x);").unwrap();
        }
        assert!(validate_sqlite_file(valid.to_str().unwrap()).is_ok());

        let invalid = dir.path().join("invalid.tmp");
        std::fs::File::create(&invalid)
            .unwrap()
            .write_all(b"not a sqlite database file")
            .unwrap();
        assert_eq!(
            validate_sqlite_file(invalid.to_str().unwrap())
                .unwrap_err()
                .to_ipc_string(),
            "err.backup_invalid_file"
        );

        let empty = dir.path().join("empty.tmp");
        std::fs::File::create(&empty).unwrap();
        assert_eq!(
            validate_sqlite_file(empty.to_str().unwrap())
                .unwrap_err()
                .to_ipc_string(),
            "err.backup_invalid_file"
        );

        let missing_err = validate_sqlite_file("/tmp/nonexistent_relwatch_backup_xyz.db")
            .unwrap_err()
            .to_ipc_string();
        assert!(missing_err.starts_with("err.backup_read_failed|"));
    }

    #[test]
    fn vacuum_into_handles_overwrite_quotes_and_clean_failure() {
        let dir = TempDirGuard::new("vacuum");
        let backup_path = dir.path().join("backup.db");
        std::fs::write(&backup_path, b"stale backup content".repeat(64)).unwrap();

        let conn = seed_db();
        vacuum_into(&conn, &backup_path).unwrap();
        assert!(validate_sqlite_file(backup_path.to_str().unwrap()).is_ok());
        assert!(!sibling_temp_path(&backup_path).exists());

        let quote_dir = dir.path().join("it's here");
        std::fs::create_dir_all(&quote_dir).unwrap();
        let quoted = quote_dir.join("back'up.db");
        vacuum_into(&conn, &quoted).unwrap();
        assert!(validate_sqlite_file(quoted.to_str().unwrap()).is_ok());

        let missing = dir.path().join("no_such_subdir").join("backup.db");
        let err = vacuum_into(&conn, &missing).unwrap_err().to_ipc_string();
        assert!(err.starts_with("err.backup_export_failed|"), "{err}");
        assert!(!missing.exists());
        assert!(!sibling_temp_path(&missing).exists());
    }

    #[tokio::test]
    async fn export_then_import_roundtrip_restores_sources_and_syncs_gate() {
        let dir = TempDirGuard::new("roundtrip");
        let backup_file = dir.path().join("roundtrip.db");

        let src_pool = init_memory_pool().unwrap();
        {
            let mut conn = src_pool.get().unwrap();
            crate::db::sources::add_source(&conn, "github", "owner-a", "repo-a", "desc").unwrap();
            let mut cfg = crate::db::agent::load_agent_config(&conn).unwrap();
            cfg.max_concurrency = 4;
            crate::db::agent::save_agent_config(&mut conn, &cfg).unwrap();
        }

        let exported = export_backup_to_path(&src_pool, backup_file.clone())
            .await
            .unwrap();
        assert_eq!(PathBuf::from(&exported), backup_file);

        let dst_pool = init_memory_pool().unwrap();
        {
            let conn = dst_pool.get().unwrap();
            crate::db::sources::add_source(&conn, "github", "stale", "stale", "").unwrap();
        }
        let gate = ConcurrencyGate::new(1);
        import_backup_from_path(&dst_pool, &gate, backup_file)
            .await
            .unwrap();

        let conn = dst_pool.get().unwrap();
        let sources = crate::db::sources::list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].owner, "owner-a");
        assert_eq!(gate.limit(), 4);
        let logs = crate::db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs
            .iter()
            .any(|l| l.message_key.as_deref() == Some("backup.imported")));
    }

    #[tokio::test]
    async fn import_old_schema_backup_migrates_missing_columns() {
        let dir = TempDirGuard::new("old_schema");
        let backup_path = dir.path().join("old_schema.db");

        {
            let src = Connection::open(&backup_path).unwrap();
            src.execute_batch(
                "CREATE TABLE releases (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_id INTEGER NOT NULL,
                    tag_name TEXT NOT NULL,
                    release_name TEXT NOT NULL,
                    html_url TEXT NOT NULL,
                    published_at TEXT NOT NULL,
                    prerelease INTEGER NOT NULL DEFAULT 0,
                    body TEXT,
                    detected_at TEXT NOT NULL,
                    retry_count INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(source_id, tag_name)
                );
                 INSERT INTO releases VALUES (1, 1, 'v1', 'R', 'u', '2024-01-01', 0, 'b', '2024-01-01', 0);
                 CREATE TABLE sources (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_type TEXT NOT NULL, owner TEXT NOT NULL, repo TEXT NOT NULL,
                    poll_interval_minutes INTEGER NOT NULL DEFAULT 30,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    last_checked_at TEXT, last_check_status TEXT NOT NULL DEFAULT 'unknown',
                    last_check_message TEXT, consecutive_failures INTEGER NOT NULL DEFAULT 0,
                    last_new_count INTEGER NOT NULL DEFAULT 0, muted INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                    UNIQUE(source_type, owner, repo)
                );
                 INSERT INTO sources VALUES (1,'github','o','r',30,1,NULL,'unknown',NULL,0,0,0,'2024','2024');",
            )
            .unwrap();
        }

        let dst_pool = init_memory_pool().unwrap();
        let gate = ConcurrencyGate::new(1);
        import_backup_from_path(&dst_pool, &gate, backup_path)
            .await
            .unwrap();

        let conn = dst_pool.get().unwrap();
        assert!(conn
            .prepare("SELECT r.ai_summary, r.ai_importance, r.body_translated FROM releases r")
            .is_ok());
    }
}
