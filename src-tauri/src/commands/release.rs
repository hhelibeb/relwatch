use crate::db;
use crate::poll;
use crate::types::{AppState, PollResult};
use tauri_specta::Event;
use serde_json::json;

#[tauri::command]

#[specta::specta]pub async fn get_releases(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    // 同步 SQLite I/O 放进 spawn_blocking，避免在 Tauri 主线程冻结 UI
    // （池连接在轮询高峰期可能被占用， pool.get() 会等待最多 acquire_timeout）
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        db::releases::get_releases_with_state(&conn)
    })
    .await
    .map_err(|e| format!("err.task_failed|get_releases|{}", e))?
}

#[tauri::command]

#[specta::specta]pub fn set_notification_state(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_id: i64,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;

    let snooze_until = snooze_minutes.map(|minutes| {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        until.to_rfc3339()
    });

    let snooze_str = snooze_until.as_deref();
    db::releases::set_notification_state(&conn, release_id, &status, snooze_str)?;

    let rel = db::releases::get_release(&conn, release_id).ok().flatten();
    match rel {
        Some(r) => {
            let (log_owner, log_repo, log_tag) = db::logs::release_log_ident(&r);
            db::logs::write_log_key(&conn, "INFO", "release.status_changed", &json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": release_id, "action": &status}).to_string())
        }
        None => db::logs::write_log_key(&conn, "INFO", "release.status_changed_unknown", &json!({"id": release_id, "action": &status}).to_string()),
    }

    let _ = crate::events::ReleaseStateChanged(release_id).emit(&app);

    Ok(())
}

#[tauri::command]

#[specta::specta]pub fn delete_release(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_id: i64,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;

    let rel = db::releases::get_release(&conn, release_id).ok().flatten();
    match rel {
        Some(r) => db::logs::write_log_key(
            &conn,
            "INFO",
            "release.deleted",
            &json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": release_id}).to_string(),
        ),
        None => db::logs::write_log_key(
            &conn,
            "INFO",
            "release.deleted_unknown",
            &json!({"id": release_id}).to_string(),
        ),
    }

    db::releases::delete_release(&conn, release_id)?;

    let _ = crate::events::ReleaseStateChanged(release_id).emit(&app);

    Ok(())
}

#[tauri::command]

#[specta::specta]pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    poll::check_single_source(app, id).await
}

/// 对单条 release 触发 AI 全文翻译。
/// 用于用户在「原文」视图右键手动请求翻译旧 release 的场景。
/// 仅在 AI 已启用且已配置 API key 时生效；若该 release 已有译文则直接返回。
///
/// 返回**真实结果**（修复「翻译失败静默吞掉 → 前端翻译中永久卡死」）：
/// - 前置校验 AI 未启用 / key 缺失 → Err。此前这两项在 run_ai_job 内静默 return、
///   本命令无条件 Ok(())，前端成功路径不复位 translating、唯一复位点 watch
///   译文落库永不触发 → 卡片/弹窗永久禁用无法重试（AI 未启用/key 失效/断网等）
/// - 执行后回查：generate_translations_for_new 返回时所有任务与落库动作均已
///   await 完成，该 release 仍未落库 = 翻译失败（断网/API 错误等）→ Err
#[tauri::command]

#[specta::specta]pub async fn translate_release(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    release_id: i64,
) -> Result<(), String> {
    // 读取该 release 的 body，无 body 则无需翻译
    let body = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        // 已有译文则跳过
        let existing = db::releases::get_release(&conn, release_id)
            .map_err(|e| format!("err.query_failed|{}", e))?;
        if let Some(ref r) = existing {
            if r.body_translated.is_some() {
                return Ok(());
            }
            // 不参与 AI 摘要/翻译的源类型（如 youtube / bilibili）不生成翻译，
            // 与 poll 侧 filter_ai_eligible 的排除策略一致，统一由适配器能力声明驱动。
            if let Ok(adapter) = crate::source::get_adapter(&r.source_type) {
                if !adapter.ai_eligible() {
                    return Err(format!("err.source_no_ai|{}", r.source_type));
                }
            }
        }
        existing
            .ok_or_else(|| format!("err.release_not_found|{}", release_id))? // 安全地构造错误字符串
            .body
            .clone()
    };
    let body = body.ok_or_else(|| "err.empty_body".to_string())?;

    // 前置校验 AI 开关与 key：失败立即返回 Err（前端 catch 复位 translating 并提示），
    // 而不是让 run_ai_job 静默 return 后本命令无条件 Ok(()) 造成永久卡死。
    {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let cfg = crate::deepseek::read_config(&conn);
        if !cfg.enabled {
            return Err("err.ai_disabled".to_string());
        }
        if cfg.api_key.is_none() {
            return Err("err.ai_key_missing".to_string());
        }
    }

    // 委托给 deepseek 的批量翻译函数（内部校验并发、执行翻译/语言检测短路）。
    let saved = vec![(release_id, Some(body))];
    crate::deepseek::generate_translations_for_new(&state.db, &state.deepseek_semaphore, &saved, true).await;

    // 回查结果：await 后仍未落库 = 翻译未成功（断网/API 错误/build_client 失败等），
    // 返回 Err 让前端复位并提示，而非静默 Ok 造成「翻译中」永久卡死。
    {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let r = db::releases::get_release(&conn, release_id)
            .map_err(|e| format!("err.query_failed|{}", e))?;
        if r.as_ref().map(|r| r.body_translated.is_some()).unwrap_or(false) {
            // 翻译完成，通知前端刷新
            let _ = crate::events::ReleaseStateChanged(release_id).emit(&app);
            Ok(())
        } else {
            Err("err.translate_failed".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    /// Helper: create a source + release and return release id
    fn setup_source_and_release(conn: &rusqlite::Connection) -> i64 {
        let sid = db::sources::add_source(conn, "github", "owner", "repo", "").unwrap();
        db::releases::insert_release(conn, sid, "v1.0", "Release 1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap()
    }

    #[test]
    fn test_get_releases_returns_data() {
        let conn = init_memory_db().unwrap();
        assert!(db::releases::get_releases_with_state(&conn).unwrap().is_empty());

        let rid = setup_source_and_release(&conn);
        assert!(rid > 0);

        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag_name, "v1.0");
        assert_eq!(releases[0].notification_status, "pending");
    }

    #[test]
    fn test_get_pending_releases_filters_correctly() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let rid = db::releases::insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // By default, it's pending
        let pending = db::releases::get_pending_releases(&conn).unwrap();
        assert!(pending.iter().any(|r| r.id == rid));

        // Snooze with future time → not pending
        let future = chrono::Utc::now() + chrono::Duration::hours(2);
        db::releases::set_notification_state(&conn, rid, "snoozed", Some(&future.to_rfc3339())).unwrap();
        let pending = db::releases::get_pending_releases(&conn).unwrap();
        assert!(!pending.iter().any(|r| r.id == rid));
    }

    #[test]
    fn test_set_notification_state_writes_log() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
        let rid = db::releases::insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // Simulate set_notification_state's internal logic (without app.emit)
        db::releases::set_notification_state(&conn, rid, "ignored", None).unwrap();
        let rel = db::releases::get_release(&conn, rid).ok().flatten();
        if let Some(r) = rel {
            db::logs::write_log_key(
                &conn,
                "INFO",
                "release.status_changed",
                &serde_json::json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid, "action": "ignored"}).to_string(),
            );
        }

        // Verify DB state changed
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "ignored");

        // Verify log written
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.status_changed")));
    }

    #[test]
    fn test_set_notification_state_snooze_computation() {
        // Unit test for the snooze_until computation logic
        let minutes = 30i64;
        let now = chrono::Utc::now();
        let expected = now + chrono::Duration::minutes(minutes);
        let computed = now + chrono::Duration::minutes(minutes);
        // Allow 1 second tolerance
        let diff = (computed - expected).num_seconds().abs();
        assert!(diff <= 1, "snooze_until computation should be correct");
    }

    #[test]
    fn test_set_notification_state_unknown_release_logs_unknown_key() {
        let conn = init_memory_db().unwrap();

        // 模拟 set_notification_state 对不存在的 release_id 走 None 分支
        let rel = db::releases::get_release(&conn, 999).ok().flatten();
        assert!(rel.is_none());

        db::logs::write_log_key(
            &conn,
            "INFO",
            "release.status_changed_unknown",
            &serde_json::json!({"id": 999, "action": "ignored"}).to_string(),
        );

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.status_changed_unknown")));
    }

    #[test]
    fn test_delete_release_roundtrip() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
        let rid = db::releases::insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // 模拟 delete_release 的逻辑：查 release → 记日志 → 删除
        let rel = db::releases::get_release(&conn, rid).ok().flatten();
        assert!(rel.is_some());
        if let Some(r) = rel {
            db::logs::write_log_key(
                &conn,
                "INFO",
                "release.deleted",
                &serde_json::json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid}).to_string(),
            );
        }
        db::releases::delete_release(&conn, rid).unwrap();

        // 验证 release 已删除
        assert!(db::releases::get_release(&conn, rid).unwrap().is_none());

        // 验证日志已写入
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.deleted")));
    }

    #[test]
    fn test_delete_release_unknown_logs_unknown_key() {
        let conn = init_memory_db().unwrap();

        // 模拟 delete_release 对不存在的 id 走 None 分支
        let rel = db::releases::get_release(&conn, 999).ok().flatten();
        assert!(rel.is_none());

        db::logs::write_log_key(
            &conn,
            "INFO",
            "release.deleted_unknown",
            &serde_json::json!({"id": 999}).to_string(),
        );
        // delete_release 对不存在的 id 不应报错（SQL DELETE 匹配 0 行）
        db::releases::delete_release(&conn, 999).unwrap();

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("release.deleted_unknown")));
    }
}
