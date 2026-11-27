use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::services::backup_service;

async fn save_file_dialog(app: &tauri::AppHandle) -> Option<tauri_plugin_dialog::FilePath> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Database", &["db"])
        .set_file_name(format!(
            "relwatch-backup.{}.{}.db",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        ))
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.unwrap_or(None)
}

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
#[specta::specta]
pub async fn export_backup(app: tauri::AppHandle) -> Result<String, String> {
    let path = save_file_dialog(&app)
        .await
        .ok_or_else(|| "err.backup_cancelled_export".to_string())?;
    let Some(target) = path.as_path().map(|p| p.to_path_buf()) else {
        return Err("err.backup_export_failed|path is not a filesystem path".to_string());
    };
    let state = app.state::<crate::types::AppState>();
    backup_service::export_backup_to_path(&state.db, target)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn import_backup(app: tauri::AppHandle) -> Result<(), String> {
    let path = open_file_dialog(&app)
        .await
        .ok_or_else(|| "err.backup_cancelled_import".to_string())?;
    let Some(src_path) = path.as_path().map(|p| p.to_path_buf()) else {
        return Err("err.backup_open_failed|path is not a filesystem path".to_string());
    };
    let state = app.state::<crate::types::AppState>();
    backup_service::import_backup_from_path(&state.db, &state.agent_gate, src_path)
        .await
        .map_err(|e| e.to_ipc_string())
}
