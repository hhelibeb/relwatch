use std::sync::Arc;

use serde_json::json;

use crate::db::releases::{ReleaseInfo, ReleaseSearchBody};
use crate::db::{self, DbPool};
use crate::deepseek;
use crate::error::{AppError, AppResult, DbError};
use crate::source;
use crate::types::Emitter;

pub async fn get_release_catalog(pool: &DbPool) -> AppResult<Vec<ReleaseInfo>> {
    let tier1 = source::tier1_body_source_types();
    db::run_blocking(pool, move |conn| {
        Ok(db::releases::get_release_catalog(conn, &tier1)?)
    })
    .await
}

pub async fn get_release_detail(pool: &DbPool, release_id: i64) -> AppResult<ReleaseInfo> {
    db::run_blocking(pool, move |conn| {
        db::releases::get_release(conn, release_id)
            .map_err(|e| AppError::Db(DbError::Query(e.to_string())))?
            .ok_or_else(|| {
                AppError::validation_args("err.release_not_found", [release_id.to_string()])
            })
    })
    .await
}

pub async fn get_release_search_bodies(
    pool: &DbPool,
    before_id: i64,
    max_chars: i64,
) -> AppResult<Vec<ReleaseSearchBody>> {
    let max_chars = max_chars.clamp(64 * 1024, 4 * 1024 * 1024);
    let tier1 = source::tier1_body_source_types();
    db::run_blocking(pool, move |conn| {
        Ok(db::releases::get_release_search_bodies(
            conn, before_id, max_chars, &tier1,
        )?)
    })
    .await
}

pub async fn get_release_search_bodies_by_ids(
    pool: &DbPool,
    ids: Vec<i64>,
) -> AppResult<Vec<ReleaseSearchBody>> {
    let ids: Vec<i64> = ids.into_iter().take(500).collect();
    let tier1 = source::tier1_body_source_types();
    db::run_blocking(pool, move |conn| {
        Ok(db::releases::get_release_bodies_by_ids(conn, &ids, &tier1)?)
    })
    .await
}

pub async fn set_notification_state(
    pool: &DbPool,
    release_id: i64,
    status: String,
    snooze_minutes: Option<i64>,
    emitter: &dyn Emitter,
) -> AppResult<()> {
    let snooze_until = snooze_minutes.map(|minutes| {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        until.to_rfc3339()
    });
    db::run_blocking(pool, move |conn| {
        db::releases::set_notification_state(conn, release_id, &status, snooze_until.as_deref())?;
        let rel = db::releases::get_release(conn, release_id).ok().flatten();
        let mut extra = serde_json::Map::new();
        extra.insert("action".to_string(), json!(&status));
        source::write_release_log(
            conn,
            rel.as_ref(),
            release_id,
            "release.status_changed",
            "release.status_changed_unknown",
            Some(extra),
        );
        Ok(())
    })
    .await?;
    emitter.emit_release_state_changed(release_id);
    Ok(())
}

pub async fn delete_release(
    pool: &DbPool,
    release_id: i64,
    emitter: &dyn Emitter,
) -> AppResult<()> {
    db::run_blocking(pool, move |conn| {
        let rel = db::releases::get_release(conn, release_id).ok().flatten();
        source::write_release_log(
            conn,
            rel.as_ref(),
            release_id,
            "release.deleted",
            "release.deleted_unknown",
            None,
        );
        db::releases::delete_release(conn, release_id)?;
        Ok(())
    })
    .await?;
    emitter.emit_release_state_changed(release_id);
    Ok(())
}

pub async fn set_release_flag(
    pool: &DbPool,
    release_id: i64,
    flag: i64,
    emitter: &dyn Emitter,
) -> AppResult<()> {
    db::run_blocking(pool, move |conn| {
        db::releases::set_release_flag(conn, release_id, flag)?;
        let rel = db::releases::get_release(conn, release_id).ok().flatten();
        let mut extra = serde_json::Map::new();
        extra.insert("flag".to_string(), json!(flag));
        source::write_release_log(
            conn,
            rel.as_ref(),
            release_id,
            "release.flag_changed",
            "release.flag_changed_unknown",
            Some(extra),
        );
        Ok(())
    })
    .await?;
    emitter.emit_release_state_changed(release_id);
    Ok(())
}

fn batch_update_release_status_conn(
    conn: &mut rusqlite::Connection,
    release_ids: &[i64],
    status: &str,
    snooze_minutes: Option<i64>,
) -> AppResult<u32> {
    if release_ids.is_empty() {
        return Ok(0);
    }
    let snooze_until = snooze_minutes.map(|minutes| {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        until.to_rfc3339()
    });
    let tx = conn.transaction()?;
    let updated =
        db::releases::batch_set_notification_state(&tx, release_ids, status, snooze_until.as_deref())?;
    if updated > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "release.batch_status_changed",
            &json!({"count": updated, "action": status}).to_string(),
        );
    }
    tx.commit()?;
    Ok(updated)
}

pub async fn batch_update_release_status(
    pool: &DbPool,
    release_ids: Vec<i64>,
    status: String,
    snooze_minutes: Option<i64>,
    emitter: &dyn Emitter,
) -> AppResult<u32> {
    let updated = db::run_blocking_mut(pool, move |conn| {
        batch_update_release_status_conn(conn, &release_ids, &status, snooze_minutes)
    })
    .await?;
    if updated > 0 {
        emitter.emit_release_state_changed(0);
    }
    Ok(updated)
}

fn batch_set_release_flag_conn(
    conn: &mut rusqlite::Connection,
    release_ids: &[i64],
    flag: i64,
) -> AppResult<u32> {
    if release_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let updated = db::releases::batch_set_release_flag(&tx, release_ids, flag)?;
    if updated > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "release.batch_flag_changed",
            &json!({"count": updated, "flag": flag}).to_string(),
        );
    }
    tx.commit()?;
    Ok(updated)
}

pub async fn batch_set_release_flag(
    pool: &DbPool,
    release_ids: Vec<i64>,
    flag: i64,
    emitter: &dyn Emitter,
) -> AppResult<u32> {
    let updated = db::run_blocking_mut(pool, move |conn| {
        batch_set_release_flag_conn(conn, &release_ids, flag)
    })
    .await?;
    if updated > 0 {
        emitter.emit_release_state_changed(0);
    }
    Ok(updated)
}

fn batch_delete_releases_conn(
    conn: &mut rusqlite::Connection,
    release_ids: &[i64],
) -> AppResult<u32> {
    if release_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let deleted = db::releases::batch_delete_releases(&tx, release_ids)?;
    if deleted > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "release.batch_deleted",
            &json!({"count": deleted}).to_string(),
        );
    }
    tx.commit()?;
    Ok(deleted)
}

pub async fn batch_delete_releases(
    pool: &DbPool,
    release_ids: Vec<i64>,
    emitter: &dyn Emitter,
) -> AppResult<u32> {
    let deleted =
        db::run_blocking_mut(pool, move |conn| batch_delete_releases_conn(conn, &release_ids))
            .await?;
    if deleted > 0 {
        emitter.emit_release_state_changed(0);
    }
    Ok(deleted)
}

fn mark_source_releases_read_conn(
    conn: &mut rusqlite::Connection,
    source_id: i64,
) -> AppResult<u32> {
    let tx = conn.transaction()?;
    let src = db::sources::get_source(&tx, source_id)?;
    let updated = db::releases::mark_source_releases_read(&tx, source_id)?;
    if updated > 0 {
        if let Some(s) = src {
            let (log_owner, log_repo) = source::source_log_ident(
                &s.source_type,
                &s.owner,
                &s.repo,
                s.description.as_deref(),
            );
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
    tx.commit()?;
    Ok(updated)
}

pub async fn mark_source_releases_read(
    pool: &DbPool,
    source_id: i64,
    emitter: &dyn Emitter,
) -> AppResult<u32> {
    let updated = db::run_blocking_mut(pool, move |conn| {
        mark_source_releases_read_conn(conn, source_id)
    })
    .await?;
    if updated > 0 {
        emitter.emit_release_state_changed(0);
    }
    Ok(updated)
}

pub async fn translate_release(
    pool: &DbPool,
    semaphore: &Arc<tokio::sync::Semaphore>,
    release_id: i64,
    on_chunk: Option<deepseek::ChunkSink>,
    emitter: &dyn Emitter,
) -> AppResult<()> {
    enum Precheck {
        AlreadyTranslated,
        Proceed(String),
    }

    let precheck = db::run_blocking(pool, move |conn| {
        let existing = db::releases::get_release(conn, release_id)
            .map_err(|e| AppError::Db(DbError::Query(e.to_string())))?;
        let Some(r) = existing else {
            return Err(AppError::validation_args(
                "err.release_not_found",
                [release_id.to_string()],
            ));
        };
        if r.body_translated.is_some() {
            return Ok(Precheck::AlreadyTranslated);
        }
        if let Ok(adapter) = source::get_adapter(&r.source_type) {
            if !adapter.ai_eligible() {
                return Err(AppError::validation_args("err.source_no_ai", [r.source_type]));
            }
        }
        let body = r.body.ok_or_else(|| AppError::validation("err.empty_body"))?;

        let cfg = deepseek::read_config(conn);
        if !cfg.enabled {
            return Err(AppError::validation("err.ai_disabled"));
        }
        if cfg.api_key.is_none() {
            return Err(AppError::validation("err.ai_key_missing"));
        }
        Ok(Precheck::Proceed(body))
    })
    .await?;

    let body = match precheck {
        Precheck::AlreadyTranslated => return Ok(()),
        Precheck::Proceed(body) => body,
    };

    deepseek::translate_single_release(pool, semaphore, release_id, body, on_chunk).await;

    let translated = db::run_blocking(pool, move |conn| {
        let r = db::releases::get_release(conn, release_id)
            .map_err(|e| AppError::Db(DbError::Query(e.to_string())))?;
        Ok(r.map(|r| r.body_translated.is_some()).unwrap_or(false))
    })
    .await?;

    if translated {
        emitter.emit_release_state_changed(release_id);
        Ok(())
    } else {
        Err(AppError::validation("err.translate_failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_pool;
    use crate::types::NoopEmitter;

    #[tokio::test]
    async fn single_release_state_flag_and_delete_emit_and_log() {
        let pool = init_memory_pool().unwrap();
        let rid = {
            let conn = pool.get().unwrap();
            let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
            db::releases::insert_release(
                &conn,
                sid,
                "v1.0",
                "R1",
                "https://x/1",
                "2024-01-01T00:00:00Z",
                false,
                Some("hello"),
            )
            .unwrap()
        };
        let emitter = NoopEmitter::new();

        set_notification_state(&pool, rid, "snoozed".into(), Some(60), &emitter)
            .await
            .unwrap();
        set_release_flag(&pool, rid, 3, &emitter).await.unwrap();
        assert_eq!(emitter.state_changed_ids(), vec![rid, rid]);

        let detail = get_release_detail(&pool, rid).await.unwrap();
        assert_eq!(detail.notification_status, "snoozed");
        assert!(detail.snooze_until.is_some());
        assert_eq!(detail.flag, 3);

        // 非法 flag 保留结构化 DbError::Validation，不发事件
        let err = set_release_flag(&pool, rid, 99, &emitter).await.unwrap_err();
        assert!(matches!(err, AppError::Db(DbError::Validation { .. })));
        assert_eq!(err.to_ipc_string(), "err.release_flag_invalid|99");
        assert_eq!(emitter.state_changed_ids(), vec![rid, rid]);

        delete_release(&pool, rid, &emitter).await.unwrap();
        delete_release(&pool, 999, &emitter).await.unwrap();
        assert_eq!(emitter.state_changed_ids(), vec![rid, rid, rid, 999]);
        assert!(get_release_detail(&pool, rid).await.is_err());

        let conn = pool.get().unwrap();
        let keys: Vec<_> = db::logs::get_logs(&conn, 20)
            .unwrap()
            .into_iter()
            .filter_map(|l| l.message_key)
            .collect();
        assert!(keys.contains(&"release.status_changed".to_string()));
        assert!(keys.contains(&"release.flag_changed".to_string()));
        assert!(keys.contains(&"release.deleted".to_string()));
        assert!(keys.contains(&"release.deleted_unknown".to_string()));
    }

    #[tokio::test]
    async fn batch_release_operations_emit_single_event_and_rollback_on_invalid_status() {
        let pool = init_memory_pool().unwrap();
        let (sid, r1, r2) = {
            let conn = pool.get().unwrap();
            let sid = db::sources::add_source(&conn, "github", "owner", "repo", "").unwrap();
            let r1 = db::releases::insert_release(
                &conn,
                sid,
                "v1.0",
                "R1",
                "https://x/1",
                "2024-01-01T00:00:00Z",
                false,
                None,
            )
            .unwrap();
            let r2 = db::releases::insert_release(
                &conn,
                sid,
                "v2.0",
                "R2",
                "https://x/2",
                "2024-01-02T00:00:00Z",
                false,
                None,
            )
            .unwrap();
            (sid, r1, r2)
        };
        let emitter = NoopEmitter::new();

        assert_eq!(
            batch_update_release_status(&pool, vec![r1, r2], "ignored".into(), None, &emitter)
                .await
                .unwrap(),
            2
        );
        assert_eq!(emitter.state_changed_ids(), vec![0]);

        let err = batch_update_release_status(&pool, vec![r1], "bad".into(), None, &emitter)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Db(DbError::Validation { .. })));
        assert_eq!(err.to_ipc_string(), "err.invalid_notification_status|bad");
        assert_eq!(emitter.state_changed_ids(), vec![0]);

        assert_eq!(
            batch_set_release_flag(&pool, vec![r1, r2], 2, &emitter)
                .await
                .unwrap(),
            2
        );
        assert_eq!(emitter.state_changed_ids(), vec![0, 0]);

        {
            let conn = pool.get().unwrap();
            db::releases::set_notification_state(&conn, r1, "pending", None).unwrap();
        }
        assert_eq!(
            mark_source_releases_read(&pool, sid, &emitter)
                .await
                .unwrap(),
            1
        );
        assert_eq!(emitter.state_changed_ids(), vec![0, 0, 0]);

        assert_eq!(
            batch_delete_releases(&pool, vec![r1, r2], &emitter)
                .await
                .unwrap(),
            2
        );
        assert_eq!(emitter.state_changed_ids(), vec![0, 0, 0, 0]);
        assert!(get_release_catalog(&pool).await.unwrap().is_empty());

        assert_eq!(
            batch_delete_releases(&pool, vec![999], &emitter)
                .await
                .unwrap(),
            0
        );
        assert_eq!(emitter.state_changed_ids(), vec![0, 0, 0, 0]);

        let conn = pool.get().unwrap();
        let keys: Vec<_> = db::logs::get_logs(&conn, 20)
            .unwrap()
            .into_iter()
            .filter_map(|l| l.message_key)
            .collect();
        assert!(keys.contains(&"release.batch_status_changed".to_string()));
        assert!(keys.contains(&"release.batch_flag_changed".to_string()));
        assert!(keys.contains(&"release.source_marked_read".to_string()));
        assert!(keys.contains(&"release.batch_deleted".to_string()));
    }

    #[tokio::test]
    async fn translate_release_validates_eligibility_body_and_ai_config() {
        let pool = init_memory_pool().unwrap();
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let emitter = NoopEmitter::new();

        let (yt_rid, empty_rid, gh_rid) = {
            let conn = pool.get().unwrap();
            let yt_sid = db::sources::add_source(&conn, "youtube", "UC123", "", "").unwrap();
            let yt_rid = db::releases::insert_release(
                &conn,
                yt_sid,
                "v1",
                "vid",
                "https://youtu.be/1",
                "2024-01-01T00:00:00Z",
                false,
                Some("video desc"),
            )
            .unwrap();
            let gh_sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
            let empty_rid = db::releases::insert_release(
                &conn,
                gh_sid,
                "v0",
                "R0",
                "https://x/0",
                "2024-01-01T00:00:00Z",
                false,
                None,
            )
            .unwrap();
            let gh_rid = db::releases::insert_release(
                &conn,
                gh_sid,
                "v1",
                "R1",
                "https://x/1",
                "2024-01-02T00:00:00Z",
                false,
                Some("Fix critical bug in parser"),
            )
            .unwrap();
            (yt_rid, empty_rid, gh_rid)
        };

        let err = translate_release(&pool, &sem, yt_rid, None, &emitter)
            .await
            .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.source_no_ai|youtube");

        let err = translate_release(&pool, &sem, empty_rid, None, &emitter)
            .await
            .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.empty_body");

        let err = translate_release(&pool, &sem, gh_rid, None, &emitter)
            .await
            .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.ai_disabled");
    }

    #[tokio::test]
    async fn translate_release_succeeds_and_emits_state_changed() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "修复了解析器关键缺陷" } }]
            })))
            .mount(&mock)
            .await;

        crate::crypto::set_test_master_key();
        let pool = init_memory_pool().unwrap();
        let rid = {
            let conn = pool.get().unwrap();
            db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
            db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, &mock.uri())
                .unwrap();
            db::settings::set_setting(
                &conn,
                db::settings::KEY_DEEPSEEK_API_KEY,
                &crate::crypto::encrypt("sk-test"),
            )
            .unwrap();
            let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
            db::releases::insert_release(
                &conn,
                sid,
                "v1.0.0",
                "v1.0.0",
                "https://github.com/o/r/releases/tag/v1.0.0",
                "2024-01-01T00:00:00Z",
                false,
                Some("Fixed a critical bug in the parser"),
            )
            .unwrap()
        };

        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let emitter = NoopEmitter::new();
        translate_release(&pool, &sem, rid, None, &emitter)
            .await
            .unwrap();

        assert_eq!(emitter.state_changed_ids(), vec![rid]);
        let detail = get_release_detail(&pool, rid).await.unwrap();
        assert_eq!(
            detail.body_translated.as_deref(),
            Some("修复了解析器关键缺陷")
        );
    }
}
