use serde_json::json;

use crate::db::{self, DbPool};
use crate::error::{AppError, AppResult};
use crate::http;
use crate::source::{self, SourceAdapter, SourceCredentials};
use crate::types::Emitter;

async fn write_add_failed_log(
    pool: &DbPool,
    level: &'static str,
    source_type: &str,
    owner: &str,
    repo: &str,
    err: &str,
) {
    let source_type = source_type.to_string();
    let owner = owner.to_string();
    let repo = repo.to_string();
    let err = err.to_string();
    let _ = db::run_blocking(pool, move |conn| {
        db::logs::write_log_key(
            conn,
            level,
            "source.add_failed",
            &json!({
                "source_type": source_type,
                "owner": owner,
                "repo": repo,
                "error": err
            })
            .to_string(),
        );
        Ok(())
    })
    .await;
}

pub async fn add_source(
    pool: &DbPool,
    source_type: String,
    owner: String,
    repo: String,
    config: Option<String>,
) -> AppResult<i64> {
    let adapter = match source::get_adapter(&source_type) {
        Ok(a) => a,
        Err(e) => {
            write_add_failed_log(pool, "ERROR", &source_type, &owner, &repo, &e.to_ipc_string())
                .await;
            return Err(AppError::Source(e));
        }
    };
    add_source_with_adapter(pool, adapter.as_ref(), &source_type, &owner, &repo, config).await
}

async fn add_source_with_adapter(
    pool: &DbPool,
    adapter: &dyn SourceAdapter,
    source_type: &str,
    owner: &str,
    repo: &str,
    config: Option<String>,
) -> AppResult<i64> {
    let auth_kind = adapter.auth_kind();
    let (proxy, token) = db::run_blocking(pool, move |conn| {
        let proxy = db::settings::load_proxy_config(conn)?;
        let token = SourceCredentials::load(conn)
            .get(auth_kind)
            .map(ToOwned::to_owned);
        Ok((proxy, token))
    })
    .await?;

    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy.url,
        proxy_mode: &proxy.mode,
        bearer_token: None,
        ..Default::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            write_add_failed_log(pool, "ERROR", source_type, owner, repo, &e).await;
            return Err(AppError::raw(e));
        }
    };

    let resolved_owner = match adapter
        .resolve_owner(&client, owner, token.as_deref())
        .await
    {
        Ok(o) => o,
        Err(e) => {
            let level = db::logs::failure_log_level(e.status());
            write_add_failed_log(pool, level, source_type, owner, repo, &e.to_ipc_string()).await;
            return Err(AppError::Source(e));
        }
    };

    let exists = {
        let st = source_type.to_string();
        let ro = resolved_owner.clone();
        let rp = repo.to_string();
        db::run_blocking(pool, move |conn| {
            Ok(db::sources::source_exists(conn, &st, &ro, &rp)?)
        })
        .await?
    };
    if exists {
        return Ok(0);
    }

    let description = match adapter
        .verify_and_describe(&client, &resolved_owner, repo, token.as_deref())
        .await
    {
        Ok(d) => d,
        Err(e) => {
            let level = db::logs::failure_log_level(e.status());
            write_add_failed_log(pool, level, source_type, owner, repo, &e.to_ipc_string()).await;
            return Err(AppError::Source(e));
        }
    };

    let st = source_type.to_string();
    let ro = resolved_owner;
    let rp = repo.to_string();
    db::run_blocking(pool, move |conn| {
        let id = db::sources::add_source_with_config(
            conn,
            &st,
            &ro,
            &rp,
            &description,
            config.as_deref(),
        )?;
        if id == 0 {
            return Ok(0);
        }
        let (log_owner, log_repo) = source::source_log_ident(&st, &ro, &rp, Some(&description));
        db::logs::write_log_key(
            conn,
            "INFO",
            "source.added",
            &json!({"source_type": &st, "owner": &log_owner, "repo": &log_repo}).to_string(),
        );
        Ok(id)
    })
    .await
}

pub async fn remove_source(pool: &DbPool, id: i64, emitter: &dyn Emitter) -> AppResult<()> {
    db::run_blocking(pool, move |conn| {
        let source = db::sources::get_source(conn, id)?;
        db::sources::remove_source(conn, id)?;
        match source {
            Some(s) => {
                let (log_owner, log_repo) = source::source_log_ident(
                    &s.source_type,
                    &s.owner,
                    &s.repo,
                    s.description.as_deref(),
                );
                db::logs::write_log_key(
                    conn,
                    "INFO",
                    "source.removed",
                    &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string(),
                );
            }
            None => db::logs::write_log_key(
                conn,
                "INFO",
                "source.removed_unknown",
                &json!({"id": id}).to_string(),
            ),
        }
        Ok(())
    })
    .await?;
    emitter.emit_release_state_changed(id);
    Ok(())
}

fn update_source_conn(
    conn: &mut rusqlite::Connection,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
    muted: Option<bool>,
    config: Option<String>,
) -> AppResult<bool> {
    let source = db::sources::get_source(conn, id)?;
    let old_enabled = source.as_ref().map(|s| s.enabled);
    let old_muted = source.as_ref().map(|s| s.muted);
    let old_config = source.as_ref().and_then(|s| s.config.clone());
    let mut muted_changed = false;

    // poll_interval_minutes 为历史透传列（0 = 未设置，调度只读全局周期），禁止在此 clamp
    let tx = conn.transaction()?;

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
            let (log_owner, log_repo) = source::source_log_ident(
                &s.source_type,
                &s.owner,
                &s.repo,
                s.description.as_deref(),
            );

            if old_enabled != Some(enabled) {
                logged = true;
                let key = if enabled {
                    "source.log_resumed"
                } else {
                    "source.log_paused"
                };
                db::logs::write_log_key(
                    &tx,
                    "INFO",
                    key,
                    &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string(),
                );
            }

            if let Some(m) = muted {
                if old_muted != Some(m) {
                    logged = true;
                    muted_changed = true;
                    let key = if m {
                        "source.log_muted"
                    } else {
                        "source.log_unmuted"
                    };
                    db::logs::write_log_key(
                        &tx,
                        "INFO",
                        key,
                        &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string(),
                    );
                }
            }

            if let Some(c) = &config {
                if old_config.as_deref() != Some(c.as_str()) {
                    logged = true;
                    db::logs::write_log_key(
                        &tx,
                        "INFO",
                        "source.log_config_updated",
                        &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string(),
                    );
                }
            }

            if !logged {
                db::logs::write_log_key(
                    &tx,
                    "INFO",
                    "source.updated",
                    &json!({"owner": &log_owner, "repo": &log_repo, "id": id}).to_string(),
                );
            }
        }
        None => {
            db::logs::write_log_key(
                &tx,
                "INFO",
                "source.updated_unknown",
                &json!({"id": id}).to_string(),
            );
        }
    }
    tx.commit()?;
    Ok(muted_changed)
}

pub async fn update_source(
    pool: &DbPool,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
    muted: Option<bool>,
    config: Option<String>,
    emitter: &dyn Emitter,
) -> AppResult<()> {
    let muted_changed = db::run_blocking_mut(pool, move |conn| {
        update_source_conn(conn, id, enabled, poll_interval_minutes, muted, config)
    })
    .await?;
    if muted_changed {
        emitter.emit_release_state_changed(id);
    }
    Ok(())
}

pub async fn list_sources(pool: &DbPool) -> AppResult<Vec<db::sources::Source>> {
    db::run_blocking(pool, |conn| Ok(db::sources::list_sources(conn)?)).await
}

fn batch_update_sources_conn(
    conn: &mut rusqlite::Connection,
    ids: &[i64],
    enabled: Option<bool>,
    muted: Option<bool>,
) -> AppResult<(u32, bool)> {
    if ids.is_empty() || (enabled.is_none() && muted.is_none()) {
        return Ok((0, false));
    }
    let tx = conn.transaction()?;
    let mut updated = 0u32;
    let mut muted_changed = false;

    if let Some(en) = enabled {
        let count = db::sources::batch_set_sources_enabled(&tx, ids, en)?;
        updated += count;
        if count > 0 {
            let key = if en {
                "source.batch_resumed"
            } else {
                "source.batch_paused"
            };
            db::logs::write_log_key(&tx, "INFO", key, &json!({"count": count}).to_string());
        }
    }

    if let Some(m) = muted {
        let count = db::sources::batch_set_sources_muted(&tx, ids, m)?;
        updated += count;
        if count > 0 {
            muted_changed = true;
            let key = if m {
                "source.batch_muted"
            } else {
                "source.batch_unmuted"
            };
            db::logs::write_log_key(&tx, "INFO", key, &json!({"count": count}).to_string());
        }
    }

    tx.commit()?;
    Ok((updated, muted_changed))
}

pub async fn batch_update_sources(
    pool: &DbPool,
    ids: Vec<i64>,
    enabled: Option<bool>,
    muted: Option<bool>,
    emitter: &dyn Emitter,
) -> AppResult<u32> {
    let (updated, muted_changed) = db::run_blocking_mut(pool, move |conn| {
        batch_update_sources_conn(conn, &ids, enabled, muted)
    })
    .await?;
    if muted_changed {
        emitter.emit_release_state_changed(0);
    }
    Ok(updated)
}

fn batch_remove_sources_conn(conn: &mut rusqlite::Connection, ids: &[i64]) -> AppResult<u32> {
    if ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let removed = db::sources::batch_remove_sources(&tx, ids)?;
    if removed > 0 {
        db::logs::write_log_key(
            &tx,
            "INFO",
            "source.batch_removed",
            &json!({"count": removed}).to_string(),
        );
    }
    tx.commit()?;
    Ok(removed)
}

pub async fn batch_remove_sources(
    pool: &DbPool,
    ids: Vec<i64>,
    emitter: &dyn Emitter,
) -> AppResult<u32> {
    let removed =
        db::run_blocking_mut(pool, move |conn| batch_remove_sources_conn(conn, &ids)).await?;
    if removed > 0 {
        emitter.emit_release_state_changed(0);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_pool;
    use crate::error::SourceError;
    use crate::source::FetchedRelease;
    use crate::types::NoopEmitter;
    use async_trait::async_trait;

    struct MockAdapter {
        resolved_owner: Result<String, SourceError>,
        description: Result<String, SourceError>,
    }

    #[async_trait]
    impl SourceAdapter for MockAdapter {
        fn source_type(&self) -> &'static str {
            "github"
        }

        async fn fetch(
            &self,
            _client: &reqwest::Client,
            _source: &db::sources::Source,
            _per_page: usize,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedRelease>, SourceError> {
            Ok(Vec::new())
        }

        async fn fetch_all(
            &self,
            _client: &reqwest::Client,
            _source: &db::sources::Source,
            _max_count: Option<usize>,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedRelease>, SourceError> {
            Ok(Vec::new())
        }

        async fn save(
            &self,
            _db: &DbPool,
            _source: &db::sources::Source,
            _data: &[FetchedRelease],
            _max_count: usize,
            _client: &reqwest::Client,
        ) -> Vec<(i64, Option<String>)> {
            Vec::new()
        }

        async fn resolve_owner(
            &self,
            _client: &reqwest::Client,
            _owner: &str,
            _token: Option<&str>,
        ) -> Result<String, SourceError> {
            self.resolved_owner.clone()
        }

        async fn verify_and_describe(
            &self,
            _client: &reqwest::Client,
            _owner: &str,
            _repo: &str,
            _token: Option<&str>,
        ) -> Result<String, SourceError> {
            self.description.clone()
        }
    }

    #[tokio::test]
    async fn add_source_end_to_end_persists_dedups_and_logs_failures() {
        let pool = init_memory_pool().unwrap();
        let ok_adapter = MockAdapter {
            resolved_owner: Ok("resolved-owner".into()),
            description: Ok("demo repo".into()),
        };

        let id = add_source_with_adapter(&pool, &ok_adapter, "github", "@handle", "repo", None)
            .await
            .unwrap();
        assert!(id > 0);

        let dup = add_source_with_adapter(&pool, &ok_adapter, "github", "@handle", "repo", None)
            .await
            .unwrap();
        assert_eq!(dup, 0);

        let list = list_sources(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].owner, "resolved-owner");
        assert_eq!(list[0].description.as_deref(), Some("demo repo"));

        let fail_adapter = MockAdapter {
            resolved_owner: Ok("other".into()),
            description: Err(SourceError::api_error(404, "Not Found")),
        };
        let err = add_source_with_adapter(&pool, &fail_adapter, "github", "other", "missing", None)
            .await
            .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.api_error|404|Not Found");

        let conn = pool.get().unwrap();
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs
            .iter()
            .any(|l| l.message_key.as_deref() == Some("source.added")));
        assert!(logs
            .iter()
            .any(|l| l.message_key.as_deref() == Some("source.add_failed") && l.level == "ERROR"));
    }

    #[tokio::test]
    async fn update_and_remove_source_emit_badge_events_and_logs() {
        let pool = init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            db::sources::add_source(&conn, "github", "o", "r", "desc").unwrap()
        };
        let emitter = NoopEmitter::new();

        update_source(&pool, id, true, 0, Some(true), None, &emitter)
            .await
            .unwrap();
        assert_eq!(emitter.state_changed_ids(), vec![id]);

        update_source(&pool, id, true, 0, Some(false), None, &emitter)
            .await
            .unwrap();
        assert_eq!(emitter.state_changed_ids(), vec![id, id]);

        // 非 muted 变更或相同 muted 值不触发角标刷新
        update_source(&pool, id, false, 60, None, None, &emitter)
            .await
            .unwrap();
        update_source(&pool, id, true, 0, Some(false), Some(r#"{"a":1}"#.into()), &emitter)
            .await
            .unwrap();
        assert_eq!(emitter.state_changed_ids(), vec![id, id]);

        // 不存在的 id 走 updated_unknown 与 removed_unknown 日志
        update_source(&pool, 999, true, 0, None, None, &emitter)
            .await
            .unwrap();
        remove_source(&pool, 999, &emitter).await.unwrap();

        remove_source(&pool, id, &emitter).await.unwrap();
        assert_eq!(emitter.state_changed_ids(), vec![id, id, 999, id]);
        assert!(list_sources(&pool).await.unwrap().is_empty());

        let conn = pool.get().unwrap();
        let keys: Vec<_> = db::logs::get_logs(&conn, 20)
            .unwrap()
            .into_iter()
            .filter_map(|l| l.message_key)
            .collect();
        assert!(keys.contains(&"source.log_muted".to_string()));
        assert!(keys.contains(&"source.log_unmuted".to_string()));
        assert!(keys.contains(&"source.log_paused".to_string()));
        assert!(keys.contains(&"source.log_config_updated".to_string()));
        assert!(keys.contains(&"source.updated_unknown".to_string()));
        assert!(keys.contains(&"source.removed_unknown".to_string()));
        assert!(keys.contains(&"source.removed".to_string()));
    }

    #[tokio::test]
    async fn batch_update_and_remove_sources_emit_single_event_and_log() {
        let pool = init_memory_pool().unwrap();
        let (s1, s2) = {
            let conn = pool.get().unwrap();
            let s1 = db::sources::add_source(&conn, "github", "a", "b", "").unwrap();
            let s2 = db::sources::add_source(&conn, "github", "c", "d", "").unwrap();
            db::sources::record_check_failure(&conn, s2, "err").unwrap();
            (s1, s2)
        };
        let emitter = NoopEmitter::new();

        let paused = batch_update_sources(&pool, vec![s2], Some(false), None, &emitter)
            .await
            .unwrap();
        assert_eq!(paused, 1);
        assert!(emitter.state_changed_ids().is_empty());

        let muted = batch_update_sources(&pool, vec![s1, s2], None, Some(true), &emitter)
            .await
            .unwrap();
        assert_eq!(muted, 1);
        assert_eq!(emitter.state_changed_ids(), vec![0]);

        let resumed = batch_update_sources(&pool, vec![s1, s2], Some(true), None, &emitter)
            .await
            .unwrap();
        assert_eq!(resumed, 1);
        assert_eq!(emitter.state_changed_ids(), vec![0]);
        {
            let conn = pool.get().unwrap();
            let s2_after = db::sources::get_source(&conn, s2).unwrap().unwrap();
            assert!(s2_after.enabled);
            assert_eq!(s2_after.consecutive_failures, 0);
        }

        let removed = batch_remove_sources(&pool, vec![s1, s2], &emitter)
            .await
            .unwrap();
        assert_eq!(removed, 2);
        assert_eq!(emitter.state_changed_ids(), vec![0, 0]);
        assert!(list_sources(&pool).await.unwrap().is_empty());

        // 空目标删除不触发事件
        assert_eq!(batch_remove_sources(&pool, vec![999], &emitter).await.unwrap(), 0);
        assert_eq!(emitter.state_changed_ids(), vec![0, 0]);

        let conn = pool.get().unwrap();
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs
            .iter()
            .any(|l| l.message_key.as_deref() == Some("source.batch_muted")));
        assert!(logs
            .iter()
            .any(|l| l.message_key.as_deref() == Some("source.batch_removed")));
    }
}
