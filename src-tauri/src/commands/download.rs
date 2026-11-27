use crate::db;
use crate::db::settings::load_proxy_config;
use crate::http;
use crate::types::AppState;

const MAX_DOWNLOAD_BYTES: usize = 25 * 1024 * 1024;

#[tauri::command]
#[specta::specta]
pub async fn fetch_url_bytes(
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<Vec<u8>, String> {
    let proxy = db::run_blocking(&state.db, |conn| Ok(load_proxy_config(conn)?))
        .await
        .map_err(|e| e.to_ipc_string())?;
    http::fetch_public_bytes(
        &http::HttpClientConfig {
            proxy_url: &proxy.url,
            proxy_mode: &proxy.mode,
            ..Default::default()
        },
        &url,
        MAX_DOWNLOAD_BYTES,
    )
    .await
}
