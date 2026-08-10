use rusqlite::backup::Backup;
use serde_json::json;
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

/// 验证指定路径的文件是有效的 SQLite 数据库（检查魔数前 16 字节）
pub fn validate_sqlite_file(path: &str) -> Result<(), String> {
    let header = std::fs::read(path).map_err(|e| format!("err.backup_read_failed|{}", e))?;
    if header.len() < 16 || &header[..16] != b"SQLite format 3\0" {
        return Err("err.backup_invalid_file".to_string());
    }
    Ok(())
}

/// 将回调式 dialog 转换为 async
async fn save_file_dialog(app: &tauri::AppHandle) -> Option<tauri_plugin_dialog::FilePath> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Database", &["db"])
        .set_file_name(format!(
            "relwatch-backup.{}.{}.db",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".to_string())
        ))
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.unwrap_or(None)
}

/// 将回调式 open dialog 转换为 async
async fn open_file_dialog(app: &tauri::AppHandle) -> Option<tauri_plugin_dialog::FilePath> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Database", &["db"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.unwrap_or(None)
}

#[tauri::command]

#[specta::specta]pub async fn export_backup(app: tauri::AppHandle) -> Result<String, String> {
    let path = save_file_dialog(&app).await;
    let path = match path {
        Some(p) => p,
        None => return Err("err.backup_cancelled_export".to_string()),
    };

    let path_str = path.as_path().unwrap().to_string_lossy().to_string();

    let state = app.state::<crate::types::AppState>();
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;

    // WAL checkpoint 确保数据完整性
    conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| format!("err.backup_wal_checkpoint_failed|{}", e))?;

    // VACUUM INTO 创建紧凑干净的独立副本
    let escaped = path_str.replace('\'', "''").replace('\\', "\\\\");
    conn
        .execute_batch(&format!("VACUUM INTO '{}';", escaped))
        .map_err(|e| format!("err.backup_export_failed|{}", e))?;

    if let Ok(conn) = state.db.get() {
        crate::db::logs::write_log_key(&conn, "INFO", "backup.exported", &json!({"path": &path_str}).to_string());
    }

    Ok(path_str)
}

#[tauri::command]

#[specta::specta]pub async fn import_backup(app: tauri::AppHandle) -> Result<(), String> {
    let path = open_file_dialog(&app).await;
    let path = match path {
        Some(p) => p,
        None => return Err("err.backup_cancelled_import".to_string()),
    };

    let path_str = path.as_path().unwrap().to_string_lossy().to_string();

    // 持有 poll 锁，防止导入期间轮询线程并发修改数据库
    let _poll_guard = crate::poll::acquire_lock()
        .map_err(|_| "err.backup_import_busy".to_string())?;

    // 验证文件是有效的 SQLite 数据库
    validate_sqlite_file(&path_str)?;

    // 打开备份文件作为源连接（source）
    let src_conn = rusqlite::Connection::open(&path_str)
        .map_err(|e| format!("err.backup_open_failed|{}", e))?;

    // 从连接池获取目标连接（避免使用独立连接绕过连接池导致并发冲突）
    let state = app.state::<crate::types::AppState>();
    let mut dst_conn = state
        .db
        .get()
        .map_err(|e| format!("err.db_connect|{}", e))?;

    // 不预先 DELETE：SQLite Backup API 是页级整库覆盖拷贝，目标库的内容会被
    // 源库**完整替换**（含 schema），预先 DELETE 对结果无影响、反而会在恢复失败
    // 时造成用户数据被清空且无法回滚的数据丢失窗口。
    //
    // 使用 rusqlite backup API 将备份文件内容复制到运行中的数据库。
    // 通过池连接写入，SQLite 自身的 WAL 锁机制保证并发一致性。
    {
        let backup = Backup::new(&src_conn, &mut dst_conn)
            .map_err(|e| format!("err.backup_session_failed|{}", e))?;
        backup
            .run_to_completion(100, Duration::from_millis(250), None)
            .map_err(|e| format!("err.backup_restore_failed|{}", e))?;
    } // backup 在此处释放，dst_conn 不再被借用
    drop(dst_conn);

    // 恢复后跑迁移： imported 备份可能来自旧版本应用（缺 ai_summary/muted/
    // body_translated/rendered_message 等列），不补列会让引用这些列的查询立即报错，
    // 应用处于半瘫痪直到下次重启。此处主动补齐，避免该断裂。
    // 同时检查 master key 一致性： 备份里的加密设置（github_token/deepseek_api_key）
    // 用的是导出机器的 master key 加密，本机 master key 无法解密，自动清空避免静默失效。
    let cleared_keys: Vec<&'static str> = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        // Backup 整库覆盖后，目标库 schema 被备份的 schema 完全替换。旧版本备份可能
        // 缺 logs 等基础表，也可能缺 ai_summary 等 ALTER 后新增的列。先 apply_schema
        // （CREATE TABLE IF NOT EXISTS 补齐缺失基础表），再 migrate（ALTER 补齐新增列），
        // 确保运行中的应用查询新列/新表不会报错。
        if let Err(e) = crate::db::init::apply_schema(&conn) {
            return Err(format!("err.backup_reinit_failed|{}", e));
        }
        if let Err(e) = crate::db::init::migrate(&conn) {
            return Err(format!("err.backup_migrate_failed|{}", e));
        }
        crate::crypto::verify_master_key_consistency(&conn)
    };
    if !cleared_keys.is_empty() {
        eprintln!(
            "WARNING: 导入的备份中以下加密设置无法用本机 master key 解密，已自动清空，请重新配置: {}",
            cleared_keys.join(", ")
        );
    }

    if let Ok(conn) = state.db.get() {
        crate::db::logs::write_log_key(&conn, "INFO", "backup.imported", &json!({"path": &path_str}).to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_validate_sqlite_file_valid() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_valid_{}.db", std::process::id()));
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE t (x);").unwrap();
        }
        let result = validate_sqlite_file(path.to_str().unwrap());
        assert!(result.is_ok(), "合法 SQLite 文件应通过验证: {:?}", result);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_validate_sqlite_file_invalid() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_invalid_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"not a sqlite database file").unwrap();
        }
        let result = validate_sqlite_file(path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("err.backup_invalid_file"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_validate_sqlite_file_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_empty_{}.tmp", std::process::id()));
        {
            std::fs::File::create(&path).unwrap();
        }
        let result = validate_sqlite_file(path.to_str().unwrap());
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_validate_sqlite_file_nonexistent() {
        let result = validate_sqlite_file("/tmp/nonexistent_file_that_does_not_exist.db");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("err.backup_read_failed"));
    }

    #[test]
    fn test_export_vacuum_into_creates_valid_sqlite_file() {
        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_export_{}.db", std::process::id()));

        // 创建源内存数据库并写入数据
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
             INSERT INTO t VALUES (1, 'hello');"
        ).unwrap();

        // VACUUM INTO —— export_backup 的核心操作
        let escaped = backup_path.to_str().unwrap().replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{}';", escaped)).unwrap();

        // 验证文件已创建且是有效的 SQLite
        assert!(backup_path.exists(), "VACUUM INTO 应创建备份文件");
        assert!(validate_sqlite_file(backup_path.to_str().unwrap()).is_ok(),
                "导出的文件应为有效的 SQLite 数据库");

        // 验证数据往返
        let restored = rusqlite::Connection::open(&backup_path).unwrap();
        let val: String = restored.query_row(
            "SELECT val FROM t WHERE id = 1", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(val, "hello");

        let _ = std::fs::remove_file(&backup_path);
    }

    #[test]
    fn test_import_backup_restores_data() {
        use std::time::Duration;

        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_restore_{}.db", std::process::id()));

        // 创建备份文件
        {
            let src = rusqlite::Connection::open(&backup_path).unwrap();
            src.execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
                 INSERT INTO t VALUES (1, 'restored-data');"
            ).unwrap();
        }

        // 创建内存目标数据库（模拟运行中的 DB）
        let mut dst = rusqlite::Connection::open_in_memory().unwrap();
        dst.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);"
        ).unwrap();

        // rusqlite Backup API —— import_backup 的核心操作
        {
            let src_conn = rusqlite::Connection::open(&backup_path).unwrap();
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst).unwrap();
            backup.run_to_completion(100, Duration::from_millis(250), None).unwrap();
        }

        // 验证数据已恢复
        let val: String = dst.query_row(
            "SELECT val FROM t WHERE id = 1", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(val, "restored-data");

        let _ = std::fs::remove_file(&backup_path);
    }

    #[test]
    fn test_import_clears_existing_data_before_restore() {
        use std::time::Duration;

        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_clear_{}.db", std::process::id()));

        // 创建备份（一条记录）
        {
            let src = rusqlite::Connection::open(&backup_path).unwrap();
            src.execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
                 INSERT INTO t VALUES (1, 'fresh');"
            ).unwrap();
        }

        // 创建目标数据库，包含脏数据
        let mut dst = rusqlite::Connection::open_in_memory().unwrap();
        dst.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
             INSERT INTO t VALUES (1, 'stale');
             INSERT INTO t VALUES (2, 'extra');"
        ).unwrap();

        // 注意：生产代码 import_backup 不再预先 DELETE（Backup API 页级整库覆盖，
        // DELETE 是死代码且制造数据丢失窗口）。这里仅验证 Backup 覆盖后脏数据被替换。
        // 从备份恢复（整库覆盖）
        {
            let src_conn = rusqlite::Connection::open(&backup_path).unwrap();
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst).unwrap();
            backup.run_to_completion(100, Duration::from_millis(250), None).unwrap();
        }

        // 验证只有备份中的记录存在
        let count: i64 = dst.query_row(
            "SELECT COUNT(*) FROM t", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(count, 1, "只应存在恢复的一条记录");
        let val: String = dst.query_row(
            "SELECT val FROM t", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(val, "fresh", "脏数据应被备份数据覆盖");

        let _ = std::fs::remove_file(&backup_path);
    }

    /// 问题2 回归测试：导入旧版本 schema 的备份（缺 ai_summary 等 ALTER 后新增的列）
    /// 后，Backup API 整库覆盖会把目标库 schema 退回旧版；不补列会让引用新列的
    /// 查询立即报错。此处验证 import_backup 修复后的链路：覆盖 → apply_schema → migrate → 查询新列成功。
    #[test]
    fn test_import_old_schema_backup_then_migrate_restores_columns() {
        use crate::db::init::{apply_schema, migrate};
        use std::time::Duration;

        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_old_schema_{}.db", std::process::id()));

        // 构造一个旧版本 schema 的备份：只有基础 releases 表，无 ai_summary/ai_importance 列
        {
            let src = rusqlite::Connection::open(&backup_path).unwrap();
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
                 INSERT INTO sources VALUES (1,'github','o','r',30,1,NULL,'unknown',NULL,0,0,0,'2024','2024');"
            ).unwrap();
        }

        // 目标库：先用完整 init 建 schema，再被旧备份覆盖（模拟运行中应用导入旧备份）
        let mut dst = crate::db::init::init_memory_db().unwrap();
        {
            let src_conn = rusqlite::Connection::open(&backup_path).unwrap();
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst).unwrap();
            backup.run_to_completion(100, Duration::from_millis(250), None).unwrap();
        }

        // 覆盖后查询新列应失败（证明 schema 已退回旧版）
        let pre = dst.prepare("SELECT r.ai_summary FROM releases r");
        assert!(pre.is_err(), "覆盖后旧 schema 应缺 ai_summary 列");
        drop(pre);

        // 跑恢复后链路：先补基础表，再 migrate 补 ALTER 列（与 import_backup 一致）
        apply_schema(&dst).expect("apply_schema 应补齐缺失的基础表");
        migrate(&dst).expect("migrate 应成功补齐缺失列");

        // 现在引用新列的查询应成功
        let ok = dst.prepare("SELECT r.ai_summary, r.ai_importance, r.body_translated FROM releases r");
        assert!(ok.is_ok(), "migrate 后应补齐 ai_summary/ai_importance/body_translated 列");

        let _ = std::fs::remove_file(&backup_path);
    }
}
