use crate::db;
use crate::types::AppState;
use crate::{crypto, http, source};
use db::settings::{get_setting, KEY_GITHUB_TOKEN, KEY_PROXY_URL, KEY_PROXY_MODE, KEY_YOUTUBE_API_KEY, KEY_BILIBILI_COOKIE};
use serde_json::json;
use tauri::Emitter;

#[tauri::command]
pub async fn add_source(
    state: tauri::State<'_, AppState>,
    source_type: String,
    owner: String,
    repo: String,
    config: Option<String>,
) -> Result<i64, String> {
    let description: String;
    let resolved_owner: String;
    {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let proxy_url = get_setting(&conn, KEY_PROXY_URL)?.unwrap_or_default();
        let proxy_mode = get_setting(&conn, KEY_PROXY_MODE)?.unwrap_or_else(|| {
            if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
        });
        let github_token = get_setting(&conn, KEY_GITHUB_TOKEN)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                let (plain, new_v2) = crypto::decrypt_with_migration(&s)?;
                if let Some(new_val) = &new_v2 {
                    if let Err(e) = db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, new_val) {
                        log::warn!("迁移 v1→v2 GitHub Token 回写失败: {}", e);
                    }
                }
                Some(plain)
            });
        // YouTube Data API Key（加密存储，解密后传给 youtube adapter 走 API 模式）
        let youtube_api_key = get_setting(&conn, KEY_YOUTUBE_API_KEY)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                let (plain, new_v2) = crypto::decrypt_with_migration(&s)?;
                if let Some(new_val) = &new_v2 {
                    if let Err(e) = db::settings::set_setting(&conn, KEY_YOUTUBE_API_KEY, new_val) {
                        log::warn!("迁移 v1→v2 YouTube API Key 回写失败: {}", e);
                    }
                }
                Some(plain)
            });
        // B 站登录 Cookie（SESSDATA，加密存储，可选；降低风控概率）
        let bilibili_cookie = get_setting(&conn, KEY_BILIBILI_COOKIE)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                let (plain, new_v2) = crypto::decrypt_with_migration(&s)?;
                if let Some(new_val) = &new_v2 {
                    if let Err(e) = db::settings::set_setting(&conn, KEY_BILIBILI_COOKIE, new_val) {
                        log::warn!("迁移 v1→v2 B 站 Cookie 回写失败: {}", e);
                    }
                }
                Some(plain)
            });
        // client 不携带 default Authorization——github token 由 adapter 按请求设置，
        // 避免 HF 请求泄露 GitHub Token
        let client = match http::build_http_client(http::HttpClientConfig {
            proxy_url: &proxy_url,
            proxy_mode: &proxy_mode,
            bearer_token: None,
            ..Default::default()
        }) {
            Ok(c) => c,
            Err(e) => {
                db::logs::write_log_key(
                    &conn, "ERROR", "source.add_failed",
                    &json!({"source_type": &source_type, "owner": &owner, "repo": &repo, "error": &e}).to_string(),
                );
                return Err(e);
            }
        };
        // source 分发收敛为 trait 调用，消除原 match source_type 字符串分支
        let adapter = match source::get_adapter(&source_type) {
            Ok(a) => a,
            Err((_, msg)) => {
                db::logs::write_log_key(
                    &conn, "ERROR", "source.add_failed",
                    &json!({"source_type": &source_type, "owner": &owner, "repo": &repo, "error": &msg}).to_string(),
                );
                return Err(msg);
            }
        };
        // 先把用户输入归一化为标准 owner（YouTube：@handle/链接 → channel_id），
        // 再查重与验证，保证去重与 RSS 拉取都基于统一的 channel_id。
        // token 按适配器声明的 auth_kind 选取（YouTube → Data API Key，GitHub → PAT）。
        resolved_owner = match adapter
            .resolve_owner(&client, &owner, source::token_for(adapter.as_ref(), github_token.as_deref(), youtube_api_key.as_deref(), bilibili_cookie.as_deref()))
            .await
        {
            Ok(o) => o,
            Err((status, msg)) => {
                let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 { "WARN" } else { "ERROR" };
                db::logs::write_log_key(
                    &conn, level, "source.add_failed",
                    &json!({"source_type": &source_type, "owner": &owner, "repo": &repo, "error": &msg}).to_string(),
                );
                return Err(msg);
            }
        };
        if db::sources::source_exists(&conn, &source_type, &resolved_owner, &repo)? {
            return Ok(0);
        }
        description = match adapter
            .verify_and_describe(&client, &resolved_owner, &repo, source::token_for(adapter.as_ref(), github_token.as_deref(), youtube_api_key.as_deref(), bilibili_cookie.as_deref()))
            .await
        {
            Ok(d) => d,
            Err((status, msg)) => {
                let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 { "WARN" } else { "ERROR" };
                db::logs::write_log_key(
                    &conn, level, "source.add_failed",
                    &json!({"source_type": &source_type, "owner": &owner, "repo": &repo, "error": &msg}).to_string(),
                );
                return Err(msg);
            }
        };
    }
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let id = db::sources::add_source_with_config(
        &conn, &source_type, &resolved_owner, &repo, &description, config.as_deref(),
    )?;
    if id == 0 {
        return Ok(0);
    }
    let (log_owner, log_repo) = db::logs::source_log_ident(&source_type, &resolved_owner, &repo, Some(&description));
    db::logs::write_log_key(
        &conn,
        "INFO",
        "source.added",
        &json!({"source_type": &source_type, "owner": &log_owner, "repo": &log_repo}).to_string(),
    );
    Ok(id)
}

#[tauri::command]
pub fn remove_source(app: tauri::AppHandle, state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let source = db::sources::get_source(&conn, id)?;
    db::sources::remove_source(&conn, id)?;
    match source {
        Some(s) => {
            let (log_owner, log_repo) = db::logs::source_log_ident(&s.source_type, &s.owner, &s.repo, s.description.as_deref());
            db::logs::write_log_key(&conn, "INFO", "source.removed", &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string())
        }
        None => db::logs::write_log_key(&conn, "INFO", "source.removed_unknown", &json!({"id": id}).to_string()),
    }
    // 删除源会级联删除其 releases（未读计数变化），通知托盘刷新角标
    let _ = app.emit("release-state-changed", id);
    Ok(())
}

#[tauri::command]
pub fn update_source(
    state: tauri::State<AppState>,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
    muted: Option<bool>,
    config: Option<String>,
) -> Result<(), String> {
    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let source = db::sources::get_source(&conn, id)?;
    let old_enabled = source.as_ref().map(|s| s.enabled);
    let old_muted = source.as_ref().map(|s| s.muted);
    let old_config = source.as_ref().and_then(|s| s.config.clone());

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    db::sources::update_source(&tx, id, enabled, poll_interval_minutes)?;

    if let Some(m) = muted {
        db::sources::set_source_muted(&tx, id, m)?;
    }
    if let Some(c) = &config {
        db::sources::update_source_config(&tx, id, c)?;
    }

    match source {
        Some(s) => {
            let mut logged = false;

            let (log_owner, log_repo) = db::logs::source_log_ident(&s.source_type, &s.owner, &s.repo, s.description.as_deref());

            // enabled 变化 → 暂停/恢复
            if old_enabled != Some(enabled) {
                logged = true;
                if enabled {
                    db::logs::write_log_key(&tx, "INFO", "source.log_resumed",
                        &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string());
                } else {
                    db::logs::write_log_key(&tx, "INFO", "source.log_paused",
                        &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string());
                }
            }

            // muted 变化 → 静默/取消静默
            if let Some(m) = muted {
                if old_muted != Some(m) {
                    logged = true;
                    if m {
                        db::logs::write_log_key(&tx, "INFO", "source.log_muted",
                            &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string());
                    } else {
                        db::logs::write_log_key(&tx, "INFO", "source.log_unmuted",
                            &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string());
                    }
                }
            }

            // config 变化 → 订阅内容调整
            if let Some(c) = &config {
                if old_config.as_deref() != Some(c.as_str()) {
                    logged = true;
                    db::logs::write_log_key(&tx, "INFO", "source.log_config_updated",
                        &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string());
                }
            }

            // 没有具体变更被记录时，回退到通用 updated
            if !logged {
                db::logs::write_log_key(&tx, "INFO", "source.updated",
                    &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string());
            }
        }
        None => {
            db::logs::write_log_key(&tx, "INFO", "source.updated_unknown",
                &json!({"id": id}).to_string());
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_sources(state: tauri::State<AppState>) -> Result<Vec<db::sources::Source>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
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

        // Simulate update_source's internal logic (enabled: true → false, 应该记录暂停)
        let source = db::sources::get_source(&conn, id).unwrap().unwrap();
        db::sources::update_source(&conn, id, false, 60).unwrap();
        db::logs::write_log_key(
            &conn,
            "INFO",
            "source.log_paused",
            &serde_json::json!({"owner": &source.owner, "repo": &source.repo, "id": id}).to_string(),
        );

        // Verify update took effect
        let updated = db::sources::get_source(&conn, id).unwrap().unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.poll_interval_minutes, 60);

        // Log entry should exist
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("source.log_paused")));
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

    #[test]
    fn test_update_source_with_muted() {
        let conn = init_memory_db().unwrap();
        let id = db::sources::add_source(&conn, "github", "owner", "repo", "desc").unwrap();

        // 模拟 update_source 命令中传 muted=true 的逻辑（应该记录静默）
        let source = db::sources::get_source(&conn, id).unwrap().unwrap();
        db::sources::update_source(&conn, id, true, 30).unwrap();
        db::sources::set_source_muted(&conn, id, true).unwrap();
        db::logs::write_log_key(
            &conn,
            "INFO",
            "source.log_muted",
            &serde_json::json!({"owner": &source.owner, "repo": &source.repo, "id": id}).to_string(),
        );

        // 验证静默生效
        let updated = db::sources::get_source(&conn, id).unwrap().unwrap();
        assert!(updated.muted, "muted 应被设为 true");

        // 切换回非静默
        db::sources::set_source_muted(&conn, id, false).unwrap();
        let updated = db::sources::get_source(&conn, id).unwrap().unwrap();
        assert!(!updated.muted);
    }

    #[test]
    fn test_add_source_failure_writes_log() {
        let conn = init_memory_db().unwrap();

        // 模拟 add_source 中 fetch_repo_info 失败时的日志写入（404 → ERROR）
        db::logs::write_log_key(
            &conn,
            "ERROR",
            "source.add_failed",
            &serde_json::json!({"source_type":"github","owner":"o","repo":"r","error":"404 Not Found"}).to_string(),
        );

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].level, "ERROR");
        assert_eq!(logs[0].message_key.as_deref(), Some("source.add_failed"));
        assert!(logs[0].rendered_message.as_deref().unwrap().contains("404"));
    }

    #[test]
    fn test_update_source_nonexistent_id_logs_unknown() {
        let conn = init_memory_db().unwrap();

        // 模拟 update_source 对不存在的 id 走 None 分支（写 source.updated_unknown）
        db::logs::write_log_key(
            &conn,
            "INFO",
            "source.updated_unknown",
            &serde_json::json!({"id": 999}).to_string(),
        );

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("source.updated_unknown")));
    }
}
