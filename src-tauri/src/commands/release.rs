use tauri_specta::Event;

use crate::db;
use crate::poll;
use crate::services::release_service;
use crate::types::{AppState, PollResult};

#[tauri::command]
#[specta::specta]
pub async fn get_release_catalog(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    release_service::get_release_catalog(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_release_detail(
    state: tauri::State<'_, AppState>,
    release_id: i64,
) -> Result<db::releases::ReleaseInfo, String> {
    release_service::get_release_detail(&state.db, release_id)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_release_search_bodies(
    state: tauri::State<'_, AppState>,
    before_id: i64,
    max_chars: i64,
) -> Result<Vec<db::releases::ReleaseSearchBody>, String> {
    release_service::get_release_search_bodies(&state.db, before_id, max_chars)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_release_search_bodies_by_ids(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<Vec<db::releases::ReleaseSearchBody>, String> {
    release_service::get_release_search_bodies_by_ids(&state.db, ids)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn set_notification_state(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_id: i64,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<(), String> {
    release_service::set_notification_state(&state.db, release_id, status, snooze_minutes, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_release(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_id: i64,
) -> Result<(), String> {
    release_service::delete_release(&state.db, release_id, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn set_release_flag(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_id: i64,
    flag: i64,
) -> Result<(), String> {
    release_service::set_release_flag(&state.db, release_id, flag, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn batch_update_release_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_ids: Vec<i64>,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<u32, String> {
    release_service::batch_update_release_status(
        &state.db,
        release_ids,
        status,
        snooze_minutes,
        &app,
    )
    .await
    .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn batch_set_release_flag(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_ids: Vec<i64>,
    flag: i64,
) -> Result<u32, String> {
    release_service::batch_set_release_flag(&state.db, release_ids, flag, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn batch_delete_releases(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_ids: Vec<i64>,
) -> Result<u32, String> {
    release_service::batch_delete_releases(&state.db, release_ids, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn mark_source_releases_read(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    source_id: i64,
) -> Result<u32, String> {
    release_service::mark_source_releases_read(&state.db, source_id, &app)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    poll::check_single_source(app, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn translate_release(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_id: i64,
) -> Result<(), String> {
    let on_chunk: crate::deepseek::ChunkSink = {
        let app = app.clone();
        std::sync::Arc::new(move |release_id: i64, delta: &str| {
            let _ = crate::events::ReleaseTranslateChunk {
                release_id,
                delta: delta.to_string(),
            }
            .emit(&app);
        })
    };
    release_service::translate_release(
        &state.db,
        &state.deepseek_semaphore,
        release_id,
        Some(on_chunk),
        &app,
    )
    .await
    .map_err(|e| e.to_ipc_string())
}
