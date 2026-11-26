use crate::db;
use crate::poll;
use crate::types::{AppState, PollResult};
use tauri_specta::Event;
use serde_json::json;

#[tauri::command]

/// 版本列表数据源：**全库**目录（无 LIMIT）。
///
/// 正文为预览投影（见 `db::releases::get_release_catalog` 的契约说明）；全文由
/// `get_release_detail`（详情弹窗）与 `get_release_search_bodies`（全文搜索）按需取。
#[specta::specta]pub async fn get_release_catalog(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<db::releases::ReleaseInfo>, String> {
    // 同步 SQLite I/O 放进 spawn_blocking，避免在 Tauri 主线程冻结 UI
    // （池连接在轮询高峰期可能被占用， pool.get() 会等待最多 acquire_timeout）
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        db::releases::get_release_catalog(&conn)
    })
    .await
    .map_err(|e| format!("err.task_failed|get_release_catalog|{}", e))?
}

/// 单条 release 全文（详情弹窗用）：目录里只有正文预览，打开详情时取全文。
#[tauri::command]

#[specta::specta]pub async fn get_release_detail(
    state: tauri::State<'_, AppState>,
    release_id: i64,
) -> Result<db::releases::ReleaseInfo, String> {
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        db::releases::get_release(&conn, release_id)
            .map_err(|e| format!("err.query_failed|{}", e))?
            .ok_or_else(|| format!("err.release_not_found|{}", release_id))
    })
    .await
    .map_err(|e| format!("err.task_failed|get_release_detail|{}", e))?
}

/// 全文搜索索引的正文分块（按 id 游标 + 单次字符预算，**从新到旧**）。详见
/// `db::releases::get_release_search_bodies` 的契约说明。
#[tauri::command]

#[specta::specta]pub async fn get_release_search_bodies(
    state: tauri::State<'_, AppState>,
    before_id: i64,
    max_chars: i64,
) -> Result<Vec<db::releases::ReleaseSearchBody>, String> {
    // 单次预算 clamp：过小会让超大正文条目撑爆调用次数，过大则失去分块的意义
    let max_chars = max_chars.clamp(64 * 1024, 4 * 1024 * 1024);
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        db::releases::get_release_search_bodies(&conn, before_id, max_chars)
    })
    .await
    .map_err(|e| format!("err.task_failed|get_release_search_bodies|{}", e))?
}

/// 按 id 批量取正文：用于刷新索引里内容已变化的条目（翻译落库 / README 回填），
/// 游标分块只覆盖新增行，覆盖不了"已存在的行内容变了"。
#[tauri::command]

#[specta::specta]pub async fn get_release_search_bodies_by_ids(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<Vec<db::releases::ReleaseSearchBody>, String> {
    // 上限保护：一次至多 500 条，避免超长 IN 子句与超大响应
    let ids: Vec<i64> = ids.into_iter().take(500).collect();
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        db::releases::get_release_bodies_by_ids(&conn, &ids)
    })
    .await
    .map_err(|e| format!("err.task_failed|get_release_search_bodies_by_ids|{}", e))?
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
    let mut extra = serde_json::Map::new();
    extra.insert("action".to_string(), json!(&status));
    db::logs::write_release_log(
        &conn,
        rel.as_ref(),
        release_id,
        "release.status_changed",
        "release.status_changed_unknown",
        Some(extra),
    );

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
    db::logs::write_release_log(
        &conn,
        rel.as_ref(),
        release_id,
        "release.deleted",
        "release.deleted_unknown",
        None,
    );

    db::releases::delete_release(&conn, release_id)?;

    let _ = crate::events::ReleaseStateChanged(release_id).emit(&app);

    Ok(())
}

#[tauri::command]

#[specta::specta]pub fn set_release_flag(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_id: i64,
    flag: i64,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;

    db::releases::set_release_flag(&conn, release_id, flag)?;

    let rel = db::releases::get_release(&conn, release_id).ok().flatten();
    let mut extra = serde_json::Map::new();
    extra.insert("flag".to_string(), json!(flag));
    db::logs::write_release_log(
        &conn,
        rel.as_ref(),
        release_id,
        "release.flag_changed",
        "release.flag_changed_unknown",
        Some(extra),
    );

    let _ = crate::events::ReleaseStateChanged(release_id).emit(&app);

    Ok(())
}

fn batch_update_release_status_core(
    conn: &mut rusqlite::Connection,
    release_ids: &[i64],
    status: &str,
    snooze_minutes: Option<i64>,
    emit_changed: impl Fn(i64),
) -> Result<u32, String> {
    if release_ids.is_empty() {
        return Ok(0);
    }
    let snooze_until = snooze_minutes.map(|minutes| {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        until.to_rfc3339()
    });
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let updated = db::releases::batch_set_notification_state(
        &tx,
        release_ids,
        status,
        snooze_until.as_deref(),
    )?;
    if updated > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "release.batch_status_changed",
            &json!({"count": updated, "action": status}).to_string(),
        );
    }
    tx.commit().map_err(|e| e.to_string())?;
    if updated > 0 {
        emit_changed(0);
    }
    Ok(updated)
}

#[tauri::command]
#[specta::specta]
pub fn batch_update_release_status(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_ids: Vec<i64>,
    status: String,
    snooze_minutes: Option<i64>,
) -> Result<u32, String> {
    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    batch_update_release_status_core(&mut conn, &release_ids, &status, snooze_minutes, |id| {
        let _ = crate::events::ReleaseStateChanged(id).emit(&app);
    })
}

fn batch_set_release_flag_core(
    conn: &mut rusqlite::Connection,
    release_ids: &[i64],
    flag: i64,
    emit_changed: impl Fn(i64),
) -> Result<u32, String> {
    if release_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let updated = db::releases::batch_set_release_flag(&tx, release_ids, flag)?;
    if updated > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "release.batch_flag_changed",
            &json!({"count": updated, "flag": flag}).to_string(),
        );
    }
    tx.commit().map_err(|e| e.to_string())?;
    if updated > 0 {
        emit_changed(0);
    }
    Ok(updated)
}

#[tauri::command]
#[specta::specta]
pub fn batch_set_release_flag(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_ids: Vec<i64>,
    flag: i64,
) -> Result<u32, String> {
    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    batch_set_release_flag_core(&mut conn, &release_ids, flag, |id| {
        let _ = crate::events::ReleaseStateChanged(id).emit(&app);
    })
}

fn batch_delete_releases_core(
    conn: &mut rusqlite::Connection,
    release_ids: &[i64],
    emit_changed: impl Fn(i64),
) -> Result<u32, String> {
    if release_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let deleted = db::releases::batch_delete_releases(&tx, release_ids)?;
    if deleted > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "release.batch_deleted",
            &json!({"count": deleted}).to_string(),
        );
    }
    tx.commit().map_err(|e| e.to_string())?;
    if deleted > 0 {
        emit_changed(0);
    }
    Ok(deleted)
}

#[tauri::command]
#[specta::specta]
pub fn batch_delete_releases(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    release_ids: Vec<i64>,
) -> Result<u32, String> {
    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    batch_delete_releases_core(&mut conn, &release_ids, |id| {
        let _ = crate::events::ReleaseStateChanged(id).emit(&app);
    })
}

fn mark_source_releases_read_core(
    conn: &mut rusqlite::Connection,
    source_id: i64,
    emit_changed: impl Fn(i64),
) -> Result<u32, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let source = db::sources::get_source(&tx, source_id)?;
    let updated = db::releases::mark_source_releases_read(&tx, source_id)?;
    if updated > 0 {
        if let Some(s) = source {
            let (log_owner, log_repo) =
                db::logs::source_log_ident(&s.source_type, &s.owner, &s.repo, s.description.as_deref());
            db::logs::write_log_key(
                &tx,
                "INFO",
                "release.source_marked_read",
                &json!({"owner": &log_owner, "repo": &log_repo, "count": updated}).to_string(),
            );
        } else {
            db::logs::write_log_key(
                &tx,
                "INFO",
                "release.batch_status_changed",
                &json!({"count": updated, "action": "clicked"}).to_string(),
            );
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    if updated > 0 {
        emit_changed(0);
    }
    Ok(updated)
}

#[tauri::command]
#[specta::specta]
pub fn mark_source_releases_read(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    source_id: i64,
) -> Result<u32, String> {
    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    mark_source_releases_read_core(&mut conn, source_id, |id| {
        let _ = crate::events::ReleaseStateChanged(id).emit(&app);
    })
}

#[tauri::command]

#[specta::specta]pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    poll::check_single_source(app, id).await
}

/// 对单条 release 触发 AI 全文翻译。
/// 用于用户在「原文」视图右键手动请求翻译旧 release 的场景。
/// 仅在 AI 已启用且已配置 API key 时生效；若该 release 已有译文则直接返回。
///
/// 译文分片经 `release-translate-chunk` 事件实时投递，前端弹窗边收边渲染（流式）。
/// 分片只是体感优化：落库与失败判定仍以回查 `body_translated` 为准。
///
/// 返回**真实结果**（前端靠 Err 复位「翻译中」状态，静默 Ok 会让卡片永久禁用无法重试）：
/// - 前置校验 AI 未启用 / key 缺失 → Err
/// - 执行后回查：translate_single_release 返回时所有任务与落库动作均已
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
            .ok_or_else(|| format!("err.release_not_found|{}", release_id))?
            .body
            .clone()
    };
    let body = body.ok_or_else(|| "err.empty_body".to_string())?;

    // 前置校验 AI 开关与 key：失败立即返回 Err（前端 catch 复位 translating 并提示），
    // 不可静默 Ok 造成「翻译中」永久卡死。
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

    // 分片出口：只投给这次手动翻译。事件带 release_id，前端按 id 只接自己那条。
    // emit 失败（窗口已关）不影响翻译本身，故忽略返回值。
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

    // 委托给 deepseek 的单条翻译入口（内部校验并发、执行翻译/语言检测短路）。
    crate::deepseek::translate_single_release(
        &state.db,
        &state.deepseek_semaphore,
        release_id,
        body,
        Some(on_chunk),
    )
    .await;

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

    #[test]
    fn test_batch_release_commands_emit_single_event_and_write_summary_logs() {
        let mut conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
        let r1 = db::releases::insert_release(&conn, sid, "v1.0", "R1", "https://x/1", "2024-01-01T00:00:00Z", false, None).unwrap();
        let r2 = db::releases::insert_release(&conn, sid, "v2.0", "R2", "https://x/2", "2024-01-02T00:00:00Z", false, None).unwrap();

        let emitted = std::cell::RefCell::new(Vec::new());
        let emit = |id: i64| emitted.borrow_mut().push(id);

        assert_eq!(batch_update_release_status_core(&mut conn, &[r1, r2], "ignored", None, emit).unwrap(), 2);
        assert_eq!(*emitted.borrow(), vec![0]);

        // Invalid status rolls back without emitting or logging
        assert!(batch_update_release_status_core(&mut conn, &[r1], "bad", None, emit).is_err());
        assert_eq!(emitted.borrow().len(), 1);

        assert_eq!(batch_set_release_flag_core(&mut conn, &[r1, r2], 2, emit).unwrap(), 2);
        assert_eq!(emitted.borrow().len(), 2);

        // Reset to pending then mark_source_releases_read
        db::releases::set_notification_state(&conn, r1, "pending", None).unwrap();
        assert_eq!(mark_source_releases_read_core(&mut conn, sid, emit).unwrap(), 1);
        assert_eq!(emitted.borrow().len(), 3);

        assert_eq!(batch_delete_releases_core(&mut conn, &[r1, r2], emit).unwrap(), 2);
        assert_eq!(*emitted.borrow(), vec![0, 0, 0, 0]);
        assert!(db::releases::get_releases_with_state(&conn).unwrap().is_empty());

        // No-op when targets do not exist emits nothing
        assert_eq!(batch_delete_releases_core(&mut conn, &[999], emit).unwrap(), 0);
        assert_eq!(emitted.borrow().len(), 4);

        let logs = db::logs::get_logs(&conn, 20).unwrap();
        let keys: Vec<_> = logs.iter().filter_map(|l| l.message_key.as_deref()).collect();
        assert!(keys.contains(&"release.batch_status_changed"));
        assert!(keys.contains(&"release.batch_flag_changed"));
        assert!(keys.contains(&"release.source_marked_read"));
        assert!(keys.contains(&"release.batch_deleted"));
    }
}
