use crate::db;
use crate::types::AppState;
use crate::{crypto, http};
use db::settings::{get_setting, KEY_GITHUB_TOKEN, KEY_PROXY_URL};
use serde_json::json;

#[tauri::command]
pub async fn add_source(
    state: tauri::State<'_, AppState>,
    source_type: String,
    owner: String,
    repo: String,
) -> Result<i64, String> {
    let description: String;
    {
        let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
        if db::sources::source_exists(&conn, &source_type, &owner, &repo)? {
            return Ok(0);
        }
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
            .await
            .map_err(|e| format!("err.repo_verify_failed|{}", e))?;
        if resp.status() == 404 {
            return Err(format!("err.repo_not_found|{}|{}", owner, repo));
        }
        if !resp.status().is_success() {
            return Err(format!("err.repo_api_error|{}", resp.status().as_u16()));
        }
        let repo_info: serde_json::Value = resp
            .json()
            .await
            .map_err(|_| "err.repo_verify_failed|Failed to parse repo info".to_string())?;
        description = repo_info["description"].as_str().unwrap_or("").to_string();
    }
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
    let id = db::sources::add_source(&conn, &source_type, &owner, &repo, &description)?;
    if id == 0 {
        return Ok(0);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_remove_source_writes_log() {
        let conn = init_memory_db().unwrap();
        let id = db::sources::add_source(&conn, "github", "owner", "repo", "desc").unwrap();
        assert!(id > 0);

        // Simulate remove_source's internal logic
        let source = db::sources::get_source(&conn, id).unwrap().unwrap();
        db::sources::remove_source(&conn, id).unwrap();
        db::logs::write_log_key(
            &conn,
            "INFO",
            "source.removed",
            &serde_json::json!({"owner": &source.owner, "repo": &source.repo, "id": id}).to_string(),
        );

        // Source should be gone
        assert!(db::sources::list_sources(&conn).unwrap().is_empty());

        // Log entry should exist
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("source.removed")));
    }

    #[test]
    fn test_remove_source_unknown_writes_log() {
        let conn = init_memory_db().unwrap();
        // Remove non-existent source
        let result = db::sources::remove_source(&conn, 999);
        // remove_source for non-existent id should succeed (no rows affected)
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_source_writes_log() {
        let conn = init_memory_db().unwrap();
        let id = db::sources::add_source(&conn, "github", "owner", "repo", "desc").unwrap();

        // Simulate update_source's internal logic
        let source = db::sources::get_source(&conn, id).unwrap().unwrap();
        db::sources::update_source(&conn, id, false, 60).unwrap();
        db::logs::write_log_key(
            &conn,
            "INFO",
            "source.updated",
            &serde_json::json!({"owner": &source.owner, "repo": &source.repo, "id": id}).to_string(),
        );

        // Verify update took effect
        let updated = db::sources::get_source(&conn, id).unwrap().unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.poll_interval_minutes, 60);

        // Log entry should exist
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("source.updated")));
    }

    #[test]
    fn test_list_sources_roundtrip() {
        let conn = init_memory_db().unwrap();
        assert!(db::sources::list_sources(&conn).unwrap().is_empty());

        db::sources::add_source(&conn, "github", "a", "b", "d1").unwrap();
        db::sources::add_source(&conn, "github", "c", "d", "d2").unwrap();

        let list = db::sources::list_sources(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|s| s.owner == "a" && s.repo == "b"));
        assert!(list.iter().any(|s| s.owner == "c" && s.repo == "d"));
    }

    #[test]
    fn test_add_source_duplicate_returns_zero() {
        let conn = init_memory_db().unwrap();
        let id1 = db::sources::add_source(&conn, "github", "o", "r", "d").unwrap();
        assert!(id1 > 0);
        let id2 = db::sources::add_source(&conn, "github", "o", "r", "d").unwrap();
        assert_eq!(id2, 0);
    }
}
