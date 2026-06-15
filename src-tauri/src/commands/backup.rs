use rusqlite::backup::Backup;
use serde_json::json;
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

/// 验证指定路径的文件是有效的 SQLite 数据库（检查魔数前 16 字节）
pub fn validate_sqlite_file(path: &str) -> Result<(), String> {
    let header = std::fs::read(path).map_err(|e| format!("无法读取文件: {}", e))?;
    if header.len() < 16 || &header[..16] != b"SQLite format 3\0" {
        return Err("所选文件不是有效的 SQLite 数据库".to_string());
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
pub async fn export_backup(app: tauri::AppHandle) -> Result<String, String> {
    let path = save_file_dialog(&app).await;
    let path = match path {
        Some(p) => p,
        None => return Err("备份已取消".to_string()),
    };

    let path_str = path.as_path().unwrap().to_string_lossy().to_string();

    let state = app.state::<crate::types::AppState>();
    let conn = state.db.get().map_err(|e| format!("无法获取数据库连接: {}", e))?;

    // WAL checkpoint 确保数据完整性
    conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| format!("WAL checkpoint 失败: {}", e))?;

    // VACUUM INTO 创建紧凑干净的独立副本
    let escaped = path_str.replace('\'', "''").replace('\\', "\\\\");
    conn
        .execute_batch(&format!("VACUUM INTO '{}';", escaped))
        .map_err(|e| format!("导出备份失败: {}", e))?;

    if let Ok(conn) = state.db.get() {
        crate::db::logs::write_log_key(&conn, "INFO", "backup.exported", &json!({"path": &path_str}).to_string());
    }

    Ok(path_str)
}

#[tauri::command]
pub async fn import_backup(app: tauri::AppHandle) -> Result<(), String> {
    let path = open_file_dialog(&app).await;
    let path = match path {
        Some(p) => p,
        None => return Err("导入已取消".to_string()),
    };

    let path_str = path.as_path().unwrap().to_string_lossy().to_string();

    // 持有 poll 锁，防止导入期间轮询线程并发修改数据库
    let _poll_guard = crate::poll::acquire_lock()
        .map_err(|_| "导入失败：后台轮询正在进行中，请稍后重试".to_string())?;

    // 验证文件是有效的 SQLite 数据库
    validate_sqlite_file(&path_str)?;

    // 打开备份文件作为源连接（source）
    let src_conn = rusqlite::Connection::open(&path_str)
        .map_err(|e| format!("无法打开备份文件: {}", e))?;

    // 从连接池获取目标连接（避免使用独立连接绕过连接池导致并发冲突）
    let state = app.state::<crate::types::AppState>();
    let mut dst_conn = state
        .db
        .get()
        .map_err(|e| format!("无法获取数据库连接: {}", e))?;

    // 先清空现有数据（按外键依赖顺序：子表 → 父表）
    dst_conn
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DELETE FROM notification_state;
             DELETE FROM releases;
             DELETE FROM sources;
             DELETE FROM logs;
             DELETE FROM app_settings;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("清空数据失败: {}", e))?;

    // 使用 rusqlite backup API 将备份文件内容复制到运行中的数据库
    // 通过池连接写入，SQLite 自身的 WAL 锁机制保证并发一致性
    {
        let backup = Backup::new(&src_conn, &mut dst_conn)
            .map_err(|e| format!("创建备份会话失败: {}", e))?;
        backup
            .run_to_completion(100, Duration::from_millis(250), None)
            .map_err(|e| format!("恢复数据失败: {}", e))?;
    } // backup 在此处释放，dst_conn 不再被借用

    let state = app.state::<crate::types::AppState>();
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
        assert!(result.unwrap_err().contains("不是有效的 SQLite"));
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
        assert!(result.unwrap_err().contains("无法读取文件"));
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

        // 模拟 import_backup 的清空步骤
        dst.execute_batch(
            "DELETE FROM t;"
        ).unwrap();
        let remaining: i64 = dst.query_row(
            "SELECT COUNT(*) FROM t", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(remaining, 0, "清空后不应还有数据");

        // 从备份恢复
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
}
