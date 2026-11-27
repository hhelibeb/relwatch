use crate::db;
use crate::services::source_service;
use crate::source;
use crate::types::AppState;

#[derive(serde::Serialize, specta::Type)]
pub struct SourceTypeInfo {
    pub source_type: String,
    pub auth_kind: String,
    pub ai_eligible: bool,
    pub always_fetch_history: bool,
    pub refresh_description_after_check: bool,
}

#[tauri::command]
#[specta::specta]
pub fn list_source_types() -> Vec<SourceTypeInfo> {
    source::list_adapters()
        .into_iter()
        .map(|a| SourceTypeInfo {
            source_type: a.source_type().to_string(),
            auth_kind: a.auth_kind().as_str().to_string(),
            ai_eligible: a.ai_eligible(),
            always_fetch_history: a.always_fetch_history(),
            refresh_description_after_check: a.refresh_description_after_check(),
        })
        .collect()
}

#[tauri::command]
#[specta::specta]
pub async fn add_source(
    state: tauri::State<'_, AppState>,
    source_type: String,
    owner: String,
    repo: String,
    config: Option<String>,
) -> Result<i64, String> {
    source_service::add_source(&state.db, source_type, owner, repo, config)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn remove_source(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    source_service::remove_source(&state.db, id, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_source(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
    muted: Option<bool>,
    config: Option<String>,
) -> Result<(), String> {
    source_service::update_source(&state.db, id, enabled, poll_interval_minutes, muted, config, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_sources(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<db::sources::Source>, String> {
    source_service::list_sources(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn batch_update_sources(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
    enabled: Option<bool>,
    muted: Option<bool>,
) -> Result<u32, String> {
    source_service::batch_update_sources(&state.db, ids, enabled, muted, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn batch_remove_sources(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<u32, String> {
    source_service::batch_remove_sources(&state.db, ids, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}
