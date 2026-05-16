use rusqlite::backup::Backup;
use serde_json::json;
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

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

    // 验证文件是有效的 SQLite 数据库
    let header = std::fs::read(&path_str).map_err(|e| format!("无法读取文件: {}", e))?;
    if header.len() < 16 || &header[..16] != b"SQLite format 3\0" {
        return Err("所选文件不是有效的 SQLite 数据库".to_string());
    }

    // 打开备份文件作为源连接（source）
    let src_conn = rusqlite::Connection::open(&path_str)
        .map_err(|e| format!("无法打开备份文件: {}", e))?;

    // 打开目标数据库的独立连接作为恢复目标（Backup API 需要 &mut Connection）
    let db_path = crate::db::init::db_path();
    let mut dst_conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("无法打开目标数据库: {}", e))?;

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
    let backup = Backup::new(&src_conn, &mut dst_conn)
        .map_err(|e| format!("创建备份会话失败: {}", e))?;

    backup
        .run_to_completion(100, Duration::from_millis(250), None)
        .map_err(|e| format!("恢复数据失败: {}", e))?;

    let state = app.state::<crate::types::AppState>();
    if let Ok(conn) = state.db.get() {
        crate::db::logs::write_log_key(&conn, "INFO", "backup.imported", &json!({"path": &path_str}).to_string());
    }

    Ok(())
}
