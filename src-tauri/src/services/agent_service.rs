use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::agent::{
    agent_process_level_changed, format_session_command, resolve_session_binary, spawn_terminal,
    validate_attached_files,
};
use crate::agent_gate::ConcurrencyGate;
use crate::agent_rpc::{self, RpcAvailableModel, RpcManager};
use crate::agent_session::{
    self, agent_sessions_dir, align_run_ids, discover_disk_sessions, is_safe_session_path,
    is_valid_session_key, partition_session_keys, prune_session_files_and_db,
    remove_session_file_in, session_path_for_key, AgentChatMessage, AgentSessionInfo,
    AgentSessionUsage, PruneAgentSessionsResult,
};
use crate::db::agent::{self as db_agent, AgentConfig, AgentEntityRef, AgentModelRef, NewRun};
use crate::db::logs;
use crate::db::{self, DbPool};
use crate::error::{AgentRunError, AppError, AppResult, DbError};

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentJobInput {
    pub session_key: String,
    pub entities: Vec<AgentEntityRef>,
    pub skill_path: Option<String>,
    pub instruction: String,
    pub model: Option<AgentModelRef>,
    #[serde(default)]
    pub files: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentModelsInfo {
    pub models: Vec<RpcAvailableModel>,
    pub current: Option<RpcAvailableModel>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentRpcStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub process_count: u32,
    pub restart_pending: bool,
}

pub async fn get_agent_config(pool: &DbPool) -> AppResult<AgentConfig> {
    db::run_blocking(pool, |conn| Ok(db_agent::load_agent_config(conn)?)).await
}

pub async fn save_agent_config(
    pool: &DbPool,
    agent_gate: &Arc<ConcurrencyGate>,
    agent_rpc: &RpcManager,
    config: AgentConfig,
) -> AppResult<()> {
    let wd_check = config
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    if let Some(wd) = wd_check {
        let is_dir = tokio::task::spawn_blocking(move || Path::new(&wd).is_dir())
            .await
            .map_err(|e| AppError::Db(DbError::TaskJoin(e.to_string())))?;
        if !is_dir {
            return Err(AppError::validation("err.agent.working_dir_not_found"));
        }
    }

    let cfg_for_db = config.clone();
    let old = db::run_blocking_mut(pool, move |conn| {
        let old = db_agent::load_agent_config(conn)?;
        db_agent::save_agent_config(conn, &cfg_for_db)?;
        logs::write_log_key(
            conn,
            "INFO",
            "agent.config_saved",
            &serde_json::json!({
                "enabled": cfg_for_db.enabled,
                "skills": cfg_for_db.skills.len()
            })
            .to_string(),
        );
        Ok(old)
    })
    .await?;

    let old_max = old.max_concurrency;
    let new_max = agent_gate.set_limit(config.max_concurrency);
    if new_max != old_max {
        log::info!("agent concurrency limit changed: {} -> {}", old_max, new_max);
    }
    if agent_process_level_changed(&old, &config) {
        agent_rpc.request_restart().await;
    }
    Ok(())
}

pub async fn get_agent_ws_width(pool: &DbPool) -> AppResult<i64> {
    db::run_blocking(pool, |conn| Ok(db_agent::load_agent_ws_width(conn)?)).await
}

pub async fn save_agent_ws_width(pool: &DbPool, width: i64) -> AppResult<()> {
    if !(1..=2000).contains(&width) {
        return Err(AppError::validation("err.agent.ws_width_range"));
    }
    db::run_blocking(pool, move |conn| Ok(db_agent::save_agent_ws_width(conn, width)?)).await
}

pub async fn enqueue_agent_job(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    input: AgentJobInput,
) -> AppResult<i64> {
    enqueue_agent_job_in(pool, agent_rpc, input, None).await
}

async fn enqueue_agent_job_in(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    input: AgentJobInput,
    sessions_dir_override: Option<PathBuf>,
) -> AppResult<i64> {
    let session_key = input.session_key.trim().to_string();
    if !is_valid_session_key(&session_key) {
        return Err(AppError::validation("err.agent.invalid_session"));
    }
    let instruction = input.instruction.trim().to_string();
    let input_entities = input.entities;
    let input_skill_path = input.skill_path;
    let sk_for_check = session_key.clone();

    let (entities, skill_path, existing_session_path) = db::run_blocking(pool, move |conn| {
        let config = db_agent::load_agent_config(conn)?;
        if !config.enabled {
            return Err(AppError::validation("err.agent.disabled"));
        }
        let mut entities: Vec<AgentEntityRef> = Vec::new();
        for r in &input_entities {
            if entities.iter().any(|e| e.kind == r.kind && e.id == r.id) {
                continue;
            }
            let exists = match r.kind.as_str() {
                "source" => db::sources::get_source(conn, r.id)?.is_some(),
                "release" => db::releases::get_release(conn, r.id)?.is_some(),
                _ => false,
            };
            if !exists {
                return Err(AppError::validation("err.agent.entity_missing"));
            }
            entities.push(r.clone());
        }
        let skill_path = match input_skill_path.as_deref().map(|s| s.trim().to_string()) {
            Some(p) if !p.is_empty() => {
                if !config.skills.contains(&p) {
                    return Err(AppError::validation("err.agent.skill_not_configured"));
                }
                Some(p)
            }
            _ => None,
        };
        let existing_session_path = match db_agent::get_session_path(conn, &sk_for_check)? {
            Some(p) if is_safe_session_path(&p) => Some(p),
            _ => None,
        };
        Ok((entities, skill_path, existing_session_path))
    })
    .await?;

    let input_files = input.files;
    let files = tokio::task::spawn_blocking(move || {
        validate_attached_files(input_files.as_ref()).map_err(AppError::raw)
    })
    .await
    .map_err(|e| AppError::Db(DbError::TaskJoin(e.to_string())))??;

    if instruction.is_empty() && entities.is_empty() && files.is_empty() {
        return Err(AppError::validation("err.agent.empty_job"));
    }

    let model_json: Option<String> = input
        .model
        .as_ref()
        .map(|m| serde_json::to_string(m).map_err(DbError::from).map_err(AppError::from))
        .transpose()?;

    if let Some(m) = &input.model {
        verify_model_available(agent_rpc, m).await?;
    }

    let session_path = match existing_session_path {
        Some(p) => Some(p),
        None => {
            let path = match sessions_dir_override {
                Some(dir) => dir.join(format!("ws-{}.jsonl", session_key)),
                None => session_path_for_key(&session_key),
            };
            let p_str = tokio::task::spawn_blocking(move || {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                path.to_string_lossy().to_string()
            })
            .await
            .map_err(|e| AppError::Db(DbError::TaskJoin(e.to_string())))?;
            Some(p_str)
        }
    };

    let files_json: Option<String> = if files.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&files).map_err(DbError::from)?)
    };

    // BEGIN IMMEDIATE 串行化同会话并发提交，防止 check 与 insert 间的 TOCTOU
    db::run_blocking_mut(pool, move |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if db_agent::active_run_count_for_session(&tx, &session_key)? > 0 {
            return Err(AppError::validation("err.agent.session_busy"));
        }
        let run_id = db_agent::create_run(
            &tx,
            &NewRun {
                session_key: &session_key,
                skill_path: skill_path.as_deref(),
                entities: &entities,
                instruction: &instruction,
                model: model_json.as_deref(),
                session_path: session_path.as_deref(),
                files: files_json.as_deref(),
            },
        )?;
        tx.commit()?;
        Ok(run_id)
    })
    .await
}

async fn verify_model_available(agent_rpc: &RpcManager, m: &AgentModelRef) -> AppResult<()> {
    let models = agent_rpc
        .get_available_models()
        .await
        .map_err(AppError::raw)?;
    if models
        .iter()
        .any(|x| x.provider == m.provider && x.id == m.model_id)
    {
        return Ok(());
    }
    if models.is_empty() {
        log::warn!("agent model precheck skipped: pi returned an empty model list");
        return Ok(());
    }
    Err(AppError::validation_args(
        "err.agent.model_unavailable",
        [format!("{}/{}", m.provider, m.model_id)],
    ))
}

pub async fn get_available_models(
    pool: &DbPool,
    agent_rpc: &RpcManager,
) -> AppResult<AgentModelsInfo> {
    let config = db::run_blocking(pool, |conn| Ok(db_agent::load_agent_config(conn)?)).await?;
    agent_rpc::ensure_supported_type(&config).map_err(AppError::raw)?;
    if !config.enabled {
        return Ok(AgentModelsInfo {
            models: Vec::new(),
            current: None,
        });
    }
    let models = agent_rpc
        .get_available_models()
        .await
        .map_err(AppError::raw)?;
    let scoped = tokio::task::spawn_blocking(agent_rpc::read_scoped_model_patterns)
        .await
        .map_err(|e| AppError::Db(DbError::TaskJoin(e.to_string())))?;
    let models = agent_rpc::filter_scoped_models(models, &scoped);
    let mut current = agent_rpc.get_current_model().await.map_err(AppError::raw)?;

    if let Some(cfg_m) = config
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let matches = current
            .as_ref()
            .map(|c| cfg_m == c.id || cfg_m == format!("{}/{}", c.provider, c.id))
            .unwrap_or(false);
        if !matches {
            if let Some((provider, model_id)) = cfg_m.split_once('/') {
                if !agent_rpc.has_busy_process().await {
                    agent_rpc
                        .set_model(provider, model_id)
                        .await
                        .map_err(AppError::raw)?;
                    current = agent_rpc.get_current_model().await.map_err(AppError::raw)?;
                }
            }
        }
    }
    Ok(AgentModelsInfo { models, current })
}

pub async fn list_agent_runs(
    pool: &DbPool,
    session_key: String,
    limit: Option<i64>,
) -> AppResult<Vec<db_agent::AgentRunSummary>> {
    if !is_valid_session_key(&session_key) {
        return Err(AppError::validation("err.agent.invalid_session"));
    }
    let limit = limit.unwrap_or(20).clamp(1, 100);
    db::run_blocking(pool, move |conn| {
        Ok(db_agent::list_run_summaries(conn, &session_key, limit)?)
    })
    .await
}

pub async fn get_agent_queue_status(
    pool: &DbPool,
    session_key: String,
) -> AppResult<db_agent::AgentQueueStatus> {
    if !is_valid_session_key(&session_key) {
        return Err(AppError::validation("err.agent.invalid_session"));
    }
    db::run_blocking(pool, move |conn| {
        Ok(db_agent::agent_queue_status(conn, &session_key)?)
    })
    .await
}

pub async fn get_agent_queue(pool: &DbPool) -> AppResult<Vec<db_agent::AgentQueueItem>> {
    db::run_blocking(pool, |conn| Ok(db_agent::agent_queue(conn)?)).await
}

pub async fn get_agent_session_usage(session_key: String) -> AppResult<AgentSessionUsage> {
    if !is_valid_session_key(&session_key) {
        return Err(AppError::validation("err.agent.invalid_session"));
    }
    tokio::task::spawn_blocking(move || {
        let path = session_path_for_key(&session_key);
        if !path.exists() {
            return Ok(AgentSessionUsage::empty());
        }
        agent_session::session_usage(&path)
            .ok_or_else(|| AppError::validation("err.agent.read_session"))
    })
    .await
    .map_err(|e| AppError::Db(DbError::TaskJoin(e.to_string())))?
}

pub async fn list_agent_messages(
    pool: &DbPool,
    session_key: String,
) -> AppResult<Vec<AgentChatMessage>> {
    if !is_valid_session_key(&session_key) {
        return Err(AppError::validation("err.agent.invalid_session"));
    }
    db::run_blocking(pool, move |conn| {
        let path = session_path_for_key(&session_key);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut messages = agent_session::parse_session_file(&path).map_err(AppError::raw)?;
        let mut runs_desc = db_agent::list_run_summaries(conn, &session_key, 1000)?;
        runs_desc.reverse();
        align_run_ids(&mut messages, &runs_desc);
        Ok(messages)
    })
    .await
}

pub async fn list_agent_sessions(pool: &DbPool) -> AppResult<Vec<AgentSessionInfo>> {
    db::run_blocking(pool, |conn| {
        let digests = db_agent::latest_run_digests(conn)?;
        Ok(discover_disk_sessions(&agent_sessions_dir(), &digests))
    })
    .await
}

pub async fn cancel_run(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    agent_cancelled: &Mutex<HashSet<i64>>,
    run_id: i64,
) -> AppResult<()> {
    let run = db::run_blocking(pool, move |conn| Ok(db_agent::get_run(conn, run_id)?))
        .await?
        .ok_or(AgentRunError::RunNotFound)?;

    if run.status != "pending" && run.status != "running" {
        return Ok(());
    }
    let was_running = run.status == "running";

    // 必须先插取消标记再 abort：pi 的 abort 响应晚于 agent_end，避免 dispatch 误记 failed
    agent_cancelled.lock().unwrap().insert(run_id);

    if was_running {
        let still_running = db::run_blocking(pool, move |conn| {
            Ok(db_agent::get_run(conn, run_id)
                .ok()
                .flatten()
                .map(|r| r.status == "running")
                .unwrap_or(false))
        })
        .await
        .unwrap_or(false);
        if still_running {
            agent_rpc.abort_run(run_id).await;
        }
    }

    let finished = db::run_blocking(pool, move |conn| {
        Ok(db_agent::get_run(conn, run_id)
            .ok()
            .flatten()
            .map(|r| r.status != "pending" && r.status != "running")
            .unwrap_or(false))
    })
    .await
    .unwrap_or(false);
    if finished {
        agent_cancelled.lock().unwrap().remove(&run_id);
    }
    Ok(())
}

pub async fn cancel_runs_best_effort(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    agent_cancelled: &Mutex<HashSet<i64>>,
    run_ids: Vec<i64>,
) -> AppResult<()> {
    for run_id in run_ids {
        if let Err(e) = cancel_run(pool, agent_rpc, agent_cancelled, run_id).await {
            if !e.is_run_not_found() {
                return Err(e);
            }
        }
    }
    Ok(())
}

pub async fn delete_agent_session(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    agent_cancelled: &Mutex<HashSet<i64>>,
    session_key: String,
) -> AppResult<()> {
    delete_agent_session_in(pool, agent_rpc, agent_cancelled, session_key, agent_sessions_dir())
        .await
}

async fn delete_agent_session_in(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    agent_cancelled: &Mutex<HashSet<i64>>,
    session_key: String,
    sessions_dir: PathBuf,
) -> AppResult<()> {
    if !is_valid_session_key(&session_key) {
        return Err(AppError::validation("err.agent.invalid_session"));
    }
    let sk = session_key.clone();
    let active_ids =
        db::run_blocking(pool, move |conn| Ok(db_agent::active_runs_for_session(conn, &sk)?))
            .await?;
    cancel_runs_best_effort(pool, agent_rpc, agent_cancelled, active_ids).await?;

    db::run_blocking(pool, move |conn| {
        remove_session_file_in(&sessions_dir, &session_key)
            .map_err(|e| AppError::validation_args("err.agent.delete_session", [e.to_string()]))?;
        Ok(db_agent::delete_runs_for_session(conn, &session_key)?)
    })
    .await
}

pub async fn prune_agent_sessions(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    agent_cancelled: &Mutex<HashSet<i64>>,
    session_keys: Vec<String>,
) -> AppResult<PruneAgentSessionsResult> {
    prune_agent_sessions_in(
        pool,
        agent_rpc,
        agent_cancelled,
        session_keys,
        agent_sessions_dir(),
    )
    .await
}

async fn prune_agent_sessions_in(
    pool: &DbPool,
    agent_rpc: &RpcManager,
    agent_cancelled: &Mutex<HashSet<i64>>,
    session_keys: Vec<String>,
    sessions_dir: PathBuf,
) -> AppResult<PruneAgentSessionsResult> {
    let (valid_keys, invalid_keys) = partition_session_keys(session_keys);
    if valid_keys.is_empty() {
        return Ok(PruneAgentSessionsResult {
            deleted: 0,
            failed: invalid_keys.len() as u32,
            failed_keys: invalid_keys,
        });
    }

    let vk = valid_keys.clone();
    let active_run_ids = db::run_blocking(pool, move |conn| {
        Ok(db_agent::batch_active_runs_for_sessions(conn, &vk)?)
    })
    .await?;
    cancel_runs_best_effort(pool, agent_rpc, agent_cancelled, active_run_ids).await?;

    db::run_blocking_mut(pool, move |conn| {
        prune_session_files_and_db(conn, &sessions_dir, valid_keys, invalid_keys)
            .map_err(AppError::raw)
    })
    .await
}

pub async fn export_agent_session_to_path(
    session_path: PathBuf,
    title: String,
    is_md: bool,
    target: String,
) -> AppResult<String> {
    tokio::task::spawn_blocking(move || {
        let messages = agent_session::parse_session_file(&session_path).map_err(AppError::raw)?;
        let usage = agent_session::session_usage(&session_path);
        let content = if is_md {
            agent_session::render_markdown(
                &title,
                &agent_session::export_messages(&messages),
                usage.as_ref(),
            )
        } else {
            serde_json::to_string_pretty(&agent_session::export_messages(&messages)).map_err(
                |e| AppError::validation_args("err.agent.export_serialize", [e.to_string()]),
            )?
        };
        std::fs::write(&target, content)
            .map_err(|e| AppError::validation_args("err.agent.export_write", [e.to_string()]))?;
        Ok(target)
    })
    .await
    .map_err(|e| AppError::Db(DbError::TaskJoin(e.to_string())))?
}

fn resolve_session_path_conn(conn: &rusqlite::Connection, run_id: i64) -> AppResult<String> {
    let run = db_agent::get_run(conn, run_id)?.ok_or(AgentRunError::RunNotFound)?;
    let path = run
        .session_path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::validation("err.agent.no_session"))?;
    // 库里的绝对路径可能来自导入的外部备份，越界路径会让 pi 在会话目录之外读写，
    // 故与写入侧保持一致，不信任未校验的值。
    if !is_safe_session_path(&path) {
        log::warn!("拒绝越界的会话路径: {:?}", path);
        return Err(AppError::validation("err.agent.session_unsafe"));
    }
    Ok(path)
}

pub async fn get_agent_session_command(pool: &DbPool, run_id: i64) -> AppResult<String> {
    db::run_blocking(pool, move |conn| {
        let path = resolve_session_path_conn(conn, run_id)?;
        let (binary, working_dir) = resolve_session_binary(conn).map_err(AppError::raw)?;
        Ok(format_session_command(&binary, &path, working_dir.as_deref()))
    })
    .await
}

pub async fn open_agent_session(pool: &DbPool, run_id: i64) -> AppResult<()> {
    db::run_blocking(pool, move |conn| {
        let path = resolve_session_path_conn(conn, run_id)?;
        if !Path::new(&path).exists() {
            return Err(AppError::validation("err.agent.session_missing"));
        }
        let (binary, working_dir) = resolve_session_binary(conn).map_err(AppError::raw)?;
        spawn_terminal(&binary, &path, working_dir.as_deref()).map_err(AppError::raw)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_pool;

    fn enable_agent(pool: &DbPool) {
        let mut conn = pool.get().unwrap();
        let mut cfg = db_agent::load_agent_config(&conn).unwrap();
        cfg.enabled = true;
        cfg.skills = vec!["skills/demo.md".into()];
        db_agent::save_agent_config(&mut conn, &cfg).unwrap();
    }

    #[test]
    fn resolve_session_path_rejects_out_of_dir_paths() {
        let pool = init_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        let run_id = db_agent::create_run(
            &conn,
            &NewRun {
                session_key: "abc",
                skill_path: None,
                entities: &[],
                instruction: "x",
                model: None,
                session_path: None,
                files: None,
            },
        )
        .unwrap();

        let ok_path = session_path_for_key("abc").to_string_lossy().to_string();
        db_agent::set_run_session_path(&conn, run_id, &ok_path).unwrap();
        assert_eq!(resolve_session_path_conn(&conn, run_id).unwrap(), ok_path);

        // 导入的外部备份可以写入任意 session_path，越界值不得拿来启 pi
        let escape = agent_sessions_dir()
            .join("..")
            .join("..")
            .join("evil.jsonl");
        db_agent::set_run_session_path(&conn, run_id, &escape.to_string_lossy()).unwrap();
        let err = resolve_session_path_conn(&conn, run_id).unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.agent.session_unsafe");
    }

    #[tokio::test]
    async fn save_agent_config_validates_working_dir_and_updates_gate() {
        let pool = init_memory_pool().unwrap();
        let gate = ConcurrencyGate::new(1);
        let rpc = RpcManager::new(pool.clone());

        let bad_cfg = AgentConfig {
            working_dir: Some("/nonexistent_dir_relwatch_test_xyz".into()),
            ..AgentConfig::default()
        };
        let err = save_agent_config(&pool, &gate, &rpc, bad_cfg)
            .await
            .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.agent.working_dir_not_found");

        let ok_cfg = AgentConfig {
            enabled: true,
            max_concurrency: 5,
            skills: vec!["a.md".into()],
            ..AgentConfig::default()
        };
        save_agent_config(&pool, &gate, &rpc, ok_cfg).await.unwrap();
        assert_eq!(gate.limit(), 5);
        assert!(get_agent_config(&pool).await.unwrap().enabled);

        assert!(save_agent_ws_width(&pool, 0).await.is_err());
        save_agent_ws_width(&pool, 520).await.unwrap();
        assert_eq!(get_agent_ws_width(&pool).await.unwrap(), 520);
    }

    #[tokio::test]
    async fn enqueue_agent_job_validates_and_blocks_busy_session() {
        let pool = init_memory_pool().unwrap();
        let rpc = RpcManager::new(pool.clone());
        let tmp_dir =
            std::env::temp_dir().join(format!("relwatch-agent-svc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let err = enqueue_agent_job_in(
            &pool,
            &rpc,
            AgentJobInput {
                session_key: "sess-1".into(),
                entities: vec![],
                skill_path: None,
                instruction: "hi".into(),
                model: None,
                files: None,
            },
            Some(tmp_dir.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.agent.disabled");

        enable_agent(&pool);

        let err = enqueue_agent_job_in(
            &pool,
            &rpc,
            AgentJobInput {
                session_key: "sess-1".into(),
                entities: vec![AgentEntityRef {
                    kind: "release".into(),
                    id: 999,
                }],
                skill_path: None,
                instruction: "hi".into(),
                model: None,
                files: None,
            },
            Some(tmp_dir.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.agent.entity_missing");

        let run_id = enqueue_agent_job_in(
            &pool,
            &rpc,
            AgentJobInput {
                session_key: "sess-1".into(),
                entities: vec![],
                skill_path: Some("skills/demo.md".into()),
                instruction: "analyze".into(),
                model: None,
                files: None,
            },
            Some(tmp_dir.clone()),
        )
        .await
        .unwrap();
        assert!(run_id > 0);

        let busy = enqueue_agent_job_in(
            &pool,
            &rpc,
            AgentJobInput {
                session_key: "sess-1".into(),
                entities: vec![],
                skill_path: None,
                instruction: "second".into(),
                model: None,
                files: None,
            },
            Some(tmp_dir.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(busy.to_ipc_string(), "err.agent.session_busy");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn cancel_delete_and_prune_sessions_mark_active_runs_and_remove_files() {
        let pool = init_memory_pool().unwrap();
        let rpc = RpcManager::new(pool.clone());
        let cancelled = Mutex::new(HashSet::new());
        let tmp_dir =
            std::env::temp_dir().join(format!("relwatch-agent-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let err = cancel_run(&pool, &rpc, &cancelled, 99999).await.unwrap_err();
        assert!(err.is_run_not_found());
        assert_eq!(err.to_ipc_string(), "err.agent.run_not_found");

        let (r1, r2) = {
            let conn = pool.get().unwrap();
            let r1 = db_agent::create_run(
                &conn,
                &NewRun {
                    session_key: "sess-del",
                    skill_path: None,
                    entities: &[],
                    instruction: "hi",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap();
            let r2 = db_agent::create_run(
                &conn,
                &NewRun {
                    session_key: "sess-prune",
                    skill_path: None,
                    entities: &[],
                    instruction: "hi",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap();
            (r1, r2)
        };
        let f1 = tmp_dir.join("ws-sess-del.jsonl");
        let f2 = tmp_dir.join("ws-sess-prune.jsonl");
        std::fs::write(&f1, "{}\n").unwrap();
        std::fs::write(&f2, "{}\n").unwrap();

        cancel_runs_best_effort(&pool, &rpc, &cancelled, vec![99999, r1])
            .await
            .unwrap();
        assert!(cancelled.lock().unwrap().contains(&r1));

        delete_agent_session_in(&pool, &rpc, &cancelled, "sess-del".into(), tmp_dir.clone())
            .await
            .unwrap();
        assert!(!f1.exists());
        assert!(list_agent_runs(&pool, "sess-del".into(), None)
            .await
            .unwrap()
            .is_empty());

        let pruned = prune_agent_sessions_in(
            &pool,
            &rpc,
            &cancelled,
            vec!["sess-prune".into(), "../bad".into()],
            tmp_dir.clone(),
        )
        .await
        .unwrap();
        assert_eq!(pruned.deleted, 1);
        assert_eq!(pruned.failed, 1);
        assert!(cancelled.lock().unwrap().contains(&r2));
        assert!(!f2.exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
