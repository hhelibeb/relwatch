use tauri::Manager;

use crate::services::setting_service;
pub use crate::services::setting_service::TestDeepseekPayload;
use crate::types::{AppSettings, AppState};

#[tauri::command]
#[specta::specta]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    setting_service::get_settings(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_settings(
    app: tauri::AppHandle,
    payload: AppSettings,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    setting_service::update_settings(&state.db, &state.next_poll_at, payload)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn set_credential(
    state: tauri::State<'_, AppState>,
    kind: String,
    value: String,
) -> Result<(), String> {
    setting_service::set_credential(&state.db, kind, value)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn test_deepseek_connection(
    state: tauri::State<'_, AppState>,
    payload: Option<TestDeepseekPayload>,
) -> Result<(), String> {
    setting_service::test_deepseek_connection(&state.db, payload)
        .await
        .map_err(|e| e.to_ipc_string())
}
