use crate::db;
use crate::types::AppState;
use crate::{crypto, http};
use db::settings::{get_setting, KEY_GITHUB_TOKEN, KEY_PROXY_URL};

#[tauri::command]
pub fn add_source(
    state: tauri::State<AppState>,
    source_type: String,
    owner: String,
    repo: String,
) -> Result<i64, String> {
    {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
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
            .map_err(|e| format!("验证仓库失败: {}", e))?;
        if resp.status() == 404 {
            return Err(format!("GitHub 上不存在仓库 {}/{}", owner, repo));
        }
        if !resp.status().is_success() {
            return Err(format!("GitHub API 返回 {}", resp.status().as_u16()));
        }
    }
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    let id = db::sources::add_source(&conn, &source_type, &owner, &repo)?;
    db::logs::write_log(
        &conn,
        "INFO",
        &format!("添加监控源: {} {}/{}", source_type, owner, repo),
    );
    Ok(id)
}

#[tauri::command]
pub fn remove_source(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    let source = db::sources::get_source(&conn, id)?;
    db::sources::remove_source(&conn, id)?;
    match source {
        Some(s) => db::logs::write_log(&conn, "INFO", &format!("移除监控源 {}/{} id={}", s.owner, s.repo, id)),
        None => db::logs::write_log(&conn, "INFO", &format!("移除监控源 id={}", id)),
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
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    let source = db::sources::get_source(&conn, id)?;
    db::sources::update_source(&conn, id, enabled, poll_interval_minutes)?;
    match source {
        Some(s) => db::logs::write_log(&conn, "INFO", &format!("更新监控源 {}/{} id={}", s.owner, s.repo, id)),
        None => db::logs::write_log(&conn, "INFO", &format!("更新监控源 id={}", id)),
    }
    Ok(())
}

#[tauri::command]
pub fn list_sources(state: tauri::State<AppState>) -> Result<Vec<db::sources::Source>, String> {
    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
    db::sources::list_sources(&conn)
}
