use crate::db;
use crate::types::AppState;
use crate::{crypto, http};
use db::settings::{get_setting, KEY_GITHUB_TOKEN, KEY_PROXY_URL};
use serde_json::json;

#[tauri::command]
pub fn add_source(
    state: tauri::State<AppState>,
    source_type: String,
    owner: String,
    repo: String,
) -> Result<i64, String> {
    {
        let conn = state.db.get().unwrap();
        let proxy_url = get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        let github_token = get_setting(&conn, KEY_GITHUB_TOKEN)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .and_then(|s| crypto::decrypt(&s));
        let client = http::build_http_client(&proxy_url, github_token.as_deref())?;
        let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
        let resp = client
            .get(&url)
            .send()
            .map_err(|e| format!("err.repo_verify_failed|{}", e))?;
        if resp.status() == 404 {
            return Err(format!("err.repo_not_found|{}|{}", owner, repo));
        }
        if !resp.status().is_success() {
            return Err(format!("err.repo_api_error|{}", resp.status().as_u16()));
        }
    }
    let conn = state.db.get().unwrap();
    let id = db::sources::add_source(&conn, &source_type, &owner, &repo)?;
    db::logs::write_log_key(
        &conn,
        "INFO",
        "source.added",
        &json!({"source_type": &source_type, "owner": &owner, "repo": &repo}).to_string(),
    );
    Ok(id)
}

#[tauri::command]
pub fn remove_source(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.get().unwrap();
    let source = db::sources::get_source(&conn, id)?;
    db::sources::remove_source(&conn, id)?;
    match source {
        Some(s) => db::logs::write_log_key(&conn, "INFO", "source.removed", &json!({"owner": &s.owner, "repo": &s.repo, "id": id}).to_string()),
        None => db::logs::write_log_key(&conn, "INFO", "source.removed_unknown", &json!({"id": id}).to_string()),
    }
    Ok(())
}

#[tauri::command]
pub fn update_source(
    state: tauri::State<AppState>,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
) -> Result<(), String> {
    let conn = state.db.get().unwrap();
    let source = db::sources::get_source(&conn, id)?;
    db::sources::update_source(&conn, id, enabled, poll_interval_minutes)?;
    match source {
        Some(s) => db::logs::write_log_key(&conn, "INFO", "source.updated", &json!({"owner": &s.owner, "repo": &s.repo, "id": id}).to_string()),
        None => db::logs::write_log_key(&conn, "INFO", "source.updated_unknown", &json!({"id": id}).to_string()),
    }
    Ok(())
}

#[tauri::command]
pub fn list_sources(state: tauri::State<AppState>) -> Result<Vec<db::sources::Source>, String> {
    let conn = state.db.get().unwrap();
    db::sources::list_sources(&conn)
}
