use std::sync::Arc;

use tauri::Manager;
use tauri_specta::Event;

use crate::agent_gate::ConcurrencyGate;
use crate::db::agent::{self, AgentConfig, AgentRun};
use crate::events::AgentRunFinished;
use crate::types::AppState;
use super::prompt::render_run_entities;
use super::{
    executor_for, AgentContext, AgentExecutor, STATUS_CANCELLED, STATUS_FAILED, STATUS_SUCCESS,
};

#[derive(Clone)]
pub struct AgentDispatchCtx {
    pub db_pool: r2d2::Pool<crate::db::SqliteConnectionManager>,
    pub gate: Arc<ConcurrencyGate>,
    pub executor_override: Option<Arc<dyn AgentExecutor>>,
    pub app: Option<tauri::AppHandle>,
    pub rpc: Arc<crate::agent_rpc::RpcManager>,
    pub cancelled: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
}

fn clear_cancel_marker(ctx: &AgentDispatchCtx, run_id: i64) {
    ctx.cancelled.lock().unwrap().remove(&run_id);
}

pub async fn dispatch_run(ctx: &AgentDispatchCtx, run_id: i64) {
    let (config, run, entity_texts, is_first_turn): (
        Option<AgentConfig>,
        Option<AgentRun>,
        Vec<String>,
        bool,
    ) = {
        let pool = ctx.db_pool.clone();
        match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("agent dispatch db_lock: {}", e);
                    return (None, None, vec![], true);
                }
            };
            let run = match agent::get_run(&conn, run_id) {
                Ok(Some(r)) => r,
                Ok(None) => return (None, None, vec![], true),
                Err(e) => {
                    log::error!("agent get_run: {}", e);
                    return (None, None, vec![], true);
                }
            };
            let config = match agent::load_agent_config(&conn) {
                Ok(c) => Some(c),
                Err(e) => {
                    log::error!("agent load_config: {}", e);
                    None
                }
            };
            let texts = render_run_entities(&conn, &run);
            let is_first_turn = run
                .session_path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| std::fs::metadata(p).map(|m| m.len() == 0).unwrap_or(true))
                .unwrap_or(true);
            (config, Some(run), texts, is_first_turn)
        })
        .await
        {
            Ok(v) => v,
            Err(e) => {
                log::error!("agent dispatch load panic: {}", e);
                clear_cancel_marker(ctx, run_id);
                return;
            }
        }
    };

    let (config, run) = match (config, run) {
        (Some(c), Some(r)) => (c, r),
        (None, Some(r)) => {
            log::error!("agent config load failed for run {}", r.id);
            clear_cancel_marker(ctx, run_id);
            mark_run_failed(&ctx.db_pool, run_id, "err.agent.config_load").await;
            return;
        }
        _ => {
            clear_cancel_marker(ctx, run_id);
            return;
        }
    };
    if !config.enabled {
        log::warn!("agent run {} aborted: agent disabled", run_id);
        clear_cancel_marker(ctx, run_id);
        mark_run_failed(&ctx.db_pool, run_id, "err.agent.disabled").await;
        return;
    }

    let skill_path: Option<String> = run
        .skill_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string());

    let model_override: Option<crate::db::agent::AgentModelRef> = run
        .model
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let files: Vec<String> = run
        .files
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();

    let executor = match &ctx.executor_override {
        Some(e) => e.clone(),
        None => match executor_for(&config.agent_type, &config, ctx.rpc.clone()) {
            Ok(e) => e,
            Err(e) => {
                log::error!("agent executor_for {}: {}", config.agent_type, e);
                clear_cancel_marker(ctx, run_id);
                mark_run_failed(&ctx.db_pool, run_id, &e).await;
                return;
            }
        },
    };

    let _permit = match ctx.gate.acquire().await {
        Ok(p) => p,
        Err(e) => {
            log::error!("agent gate closed: {}", e);
            clear_cancel_marker(ctx, run_id);
            mark_run_failed(&ctx.db_pool, run_id, "err.agent.semaphore_closed").await;
            return;
        }
    };

    if ctx.cancelled.lock().unwrap().remove(&run_id) {
        mark_run_cancelled(&ctx.db_pool, run_id).await;
        emit_run_finished(ctx, &run, STATUS_CANCELLED, None).await;
        return;
    }

    {
        let pool = ctx.db_pool.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = pool.get() {
                let _ = agent::mark_run_started(&conn, run_id);
            }
        })
        .await;
    }

    let stream_target = ctx.app.as_ref().map(|app| {
        let app = app.clone();
        let session_key = run.session_key.clone();
        move |event: &serde_json::Value| {
            let _ = crate::events::AgentRpcStream {
                session_key: session_key.clone(),
                run_id,
                event: serde_json::to_string(event).unwrap_or_default(),
            }
            .emit(&app);
        }
    });

    let cancel_probe = {
        let cancelled = ctx.cancelled.clone();
        move || cancelled.lock().unwrap().contains(&run_id)
    };

    let outcome = executor
        .execute(&AgentContext {
            run_id,
            skill_path: skill_path.as_deref(),
            model: model_override.as_ref(),
            session_path: run.session_path.as_deref().filter(|p| !p.is_empty()),
            entity_texts: &entity_texts,
            files: &files,
            instruction: &run.instruction,
            prompt_suffix: config.prompt_suffix.as_deref(),
            first_turn: is_first_turn,
            timeout_seconds: config.timeout_seconds as u64,
            on_stream: stream_target
                .as_ref()
                .map(|f| f as &(dyn Fn(&serde_json::Value) + Send + Sync)),
            is_cancelled: Some(&cancel_probe),
        })
        .await;

    let (status, exit_code, stdout, stderr, error) = match outcome {
        Ok(o) => {
            let st = if o.exit_code == Some(0) {
                STATUS_SUCCESS
            } else {
                STATUS_FAILED
            };
            (st, o.exit_code, Some(o.stdout), Some(o.stderr), None)
        }
        Err(e) => {
            let st = e.terminal_status();
            let code = e.to_ipc_string();
            let partial = e.into_partial_stdout();
            let out = if partial.is_empty() { None } else { Some(partial) };
            (st, None, out, None, Some(code))
        }
    };
    let cancelled = ctx.cancelled.lock().unwrap().remove(&run_id);
    let (status, error) = if cancelled {
        (STATUS_CANCELLED, None)
    } else {
        (status, error)
    };

    {
        let pool = ctx.db_pool.clone();
        let err_clone = error.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = pool.get() {
                let _ = agent::finish_run(
                    &conn,
                    run_id,
                    status,
                    exit_code,
                    stdout.as_deref(),
                    stderr.as_deref(),
                    err_clone.as_deref(),
                );
            }
        })
        .await;
    }
    log::info!(
        "agent run {} finished: {} (exit={:?})",
        run_id,
        status,
        exit_code
    );

    ctx.rpc.restart_if_pending().await;

    emit_run_finished(ctx, &run, status, error).await;
}

async fn emit_run_finished(
    ctx: &AgentDispatchCtx,
    run: &AgentRun,
    status: &str,
    error: Option<String>,
) {
    if let Some(app) = &ctx.app {
        let _ = AgentRunFinished {
            run_id: run.id,
            session_key: run.session_key.clone(),
            status: status.to_string(),
            message: error,
        }
        .emit(app);
    }
}

async fn mark_run_failed(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    run_id: i64,
    error: &str,
) {
    let pool = db_pool.clone();
    let error = error.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(conn) = pool.get() {
            let _ = agent::finish_run(&conn, run_id, STATUS_FAILED, None, None, None, Some(&error));
        }
    })
    .await;
}

async fn mark_run_cancelled(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    run_id: i64,
) {
    let pool = db_pool.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(conn) = pool.get() {
            let _ = agent::finish_run(&conn, run_id, STATUS_CANCELLED, None, None, None, None);
        }
    })
    .await;
}

pub fn dispatch_ctx_from_app(app: &tauri::AppHandle) -> AgentDispatchCtx {
    let state = app.state::<AppState>();
    AgentDispatchCtx {
        db_pool: state.db.clone(),
        gate: state.agent_gate.clone(),
        executor_override: None,
        app: Some(app.clone()),
        rpc: state.agent_rpc.clone(),
        cancelled: state.agent_cancelled.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agent::{create_run, get_run, save_agent_config, AgentEntityRef, NewRun};
    use crate::db::init::init_memory_pool;
    use crate::error::AgentRunError;
    use super::super::{AgentError, AgentOutcome};
    use async_trait::async_trait;

    fn sample_config() -> AgentConfig {
        AgentConfig {
            enabled: true,
            agent_type: "pi".to_string(),
            binary: None,
            model: None,
            working_dir: None,
            prompt_suffix: None,
            timeout_seconds: 300,
            max_concurrency: 1,
            skills: vec!["/tmp/skill".to_string()],
        }
    }

    struct FakeExecutor(
        Arc<std::sync::Mutex<Vec<String>>>,
        Arc<std::sync::Mutex<Vec<Option<String>>>>,
        Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    );

    #[async_trait]
    impl AgentExecutor for FakeExecutor {
        fn agent_type(&self) -> &'static str {
            "fake"
        }
        async fn execute(&self, ctx: &AgentContext<'_>) -> Result<AgentOutcome, AgentError> {
            self.0.lock().unwrap().extend(ctx.entity_texts.iter().cloned());
            self.1
                .lock()
                .unwrap()
                .push(ctx.skill_path.map(|s| s.to_string()));
            self.2.lock().unwrap().push(ctx.files.to_vec());
            if ctx.skill_path == Some("/fail") {
                return Err("boom".into());
            }
            if ctx.skill_path == Some("/partial-fail") {
                return Err(AgentRunError::timeout_secs(300, "部分产物".to_string()));
            }
            if ctx.skill_path == Some("/end-lost") {
                return Err(AgentRunError::end_lost("已生成内容".to_string()));
            }
            Ok(AgentOutcome {
                exit_code: Some(0),
                stdout: "done".into(),
                stderr: String::new(),
            })
        }
    }

    fn fake_executor() -> Arc<dyn AgentExecutor> {
        Arc::new(FakeExecutor(
            Arc::new(std::sync::Mutex::new(Vec::new())),
            Arc::new(std::sync::Mutex::new(Vec::new())),
            Arc::new(std::sync::Mutex::new(Vec::new())),
        ))
    }

    fn observing_executor(
        entities: Arc<std::sync::Mutex<Vec<String>>>,
        skills: Arc<std::sync::Mutex<Vec<Option<String>>>>,
        files: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    ) -> Arc<dyn AgentExecutor> {
        Arc::new(FakeExecutor(entities, skills, files))
    }

    fn dispatch_ctx_with(
        pool: r2d2::Pool<crate::db::SqliteConnectionManager>,
        executor: Arc<dyn AgentExecutor>,
    ) -> AgentDispatchCtx {
        AgentDispatchCtx {
            db_pool: pool.clone(),
            gate: crate::agent_gate::ConcurrencyGate::new(1),
            executor_override: Some(executor),
            app: None,
            rpc: Arc::new(crate::agent_rpc::RpcManager::new(pool)),
            cancelled: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    #[tokio::test]
    async fn dispatch_run_transitions_to_success_with_entities() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
            crate::db::sources::add_source(&conn, "github", "owner1", "repo", "desc").unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            let entities = vec![AgentEntityRef { kind: "source".into(), id: 1 }];
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/tmp/skill"),
                    entities: &entities,
                    instruction: "总结",
                    model: Some("C:/s/ws-1.jsonl"),
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                seen.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ),
        );
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "success");
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.stdout.as_deref(), Some("done"));
        assert!(run.started_at.is_some());
        assert!(run.finished_at.is_some());
        assert!(seen.lock().unwrap().iter().any(|t| t.contains("owner1/repo")));
    }

    #[tokio::test]
    async fn dispatch_run_marks_failure_with_error() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/fail"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let ctx = dispatch_ctx_with(pool.clone(), fake_executor());
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "failed");
        assert_eq!(run.error.as_deref(), Some("boom"));
        assert!(run.finished_at.is_some());
        assert_eq!(run.stdout, None);
    }

    #[tokio::test]
    async fn dispatch_run_keeps_partial_output_on_failure() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/partial-fail"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let ctx = dispatch_ctx_with(pool.clone(), fake_executor());
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "timeout");
        assert_eq!(run.error.as_deref(), Some("err.agent.timeout|300"));
        assert_eq!(run.stdout.as_deref(), Some("部分产物"));
    }

    #[tokio::test]
    async fn dispatch_run_marks_end_lost_as_unknown_not_failed() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/end-lost"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let ctx = dispatch_ctx_with(pool.clone(), fake_executor());
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "unknown");
        assert_eq!(run.error.as_deref(), Some("err.agent.end_lost"));
        assert_eq!(run.stdout.as_deref(), Some("已生成内容"));
    }

    #[tokio::test]
    async fn dispatch_run_passes_local_files_to_executor() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            let files = serde_json::to_string(&vec!["C:/logs/app.log", "C:/img/shot.png"]).unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: None,
                    entities: &[],
                    instruction: "看看日志",
                    model: None,
                    session_path: None,
                    files: Some(&files),
                },
            )
            .unwrap()
        };
        let seen_files = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                seen_files.clone(),
            ),
        );
        dispatch_run(&ctx, run_id).await;

        assert_eq!(
            seen_files.lock().unwrap().as_slice(),
            &[vec![
                "C:/logs/app.log".to_string(),
                "C:/img/shot.png".to_string()
            ]]
        );
        let conn = pool.get().unwrap();
        assert_eq!(get_run(&conn, run_id).unwrap().unwrap().status, "success");
    }

    #[tokio::test]
    async fn dispatch_run_tolerates_corrupt_files_json() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: None,
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: Some("{not json"),
                },
            )
            .unwrap()
        };
        let seen_files = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                seen_files.clone(),
            ),
        );
        dispatch_run(&ctx, run_id).await;

        assert_eq!(seen_files.lock().unwrap().as_slice(), &[Vec::<String>::new()]);
        let conn = pool.get().unwrap();
        assert_eq!(get_run(&conn, run_id).unwrap().unwrap().status, "success");
    }

    #[tokio::test]
    async fn dispatch_run_aborts_when_agent_disabled() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &AgentConfig { enabled: false, ..sample_config() }).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/tmp/skill"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                seen.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ),
        );
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "failed");
        assert_eq!(run.error.as_deref(), Some("err.agent.disabled"));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatch_run_without_skill_runs_without_skill() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &AgentConfig { skills: vec![], ..sample_config() }).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: None,
                    entities: &[],
                    instruction: "你好",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let seen_skills = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                Arc::new(std::sync::Mutex::new(Vec::new())),
                seen_skills.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ),
        );
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "success");
        assert_eq!(seen_skills.lock().unwrap().as_slice(), &[None]);
    }

    #[tokio::test]
    async fn dispatch_run_cancelled_before_spawn_marks_cancelled() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/tmp/skill"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                seen.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ),
        );
        ctx.cancelled.lock().unwrap().insert(run_id);
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "cancelled");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatch_run_cancelled_after_run_marks_cancelled() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/fail"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let ctx = dispatch_ctx_with(pool.clone(), fake_executor());
        ctx.cancelled.lock().unwrap().insert(run_id);
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "cancelled");
    }

    #[tokio::test]
    async fn dispatch_run_with_skill_passes_skill_to_executor() {
        let pool = init_memory_pool().unwrap();
        {
            let mut conn = pool.get().unwrap();
            save_agent_config(&mut conn, &sample_config()).unwrap();
        }
        let run_id = {
            let conn = pool.get().unwrap();
            create_run(
                &conn,
                &NewRun {
                    session_key: "ws-1",
                    skill_path: Some("/tmp/skill"),
                    entities: &[],
                    instruction: "x",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap()
        };
        let seen_skills = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let ctx = dispatch_ctx_with(
            pool.clone(),
            observing_executor(
                Arc::new(std::sync::Mutex::new(Vec::new())),
                seen_skills.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ),
        );
        dispatch_run(&ctx, run_id).await;

        let conn = pool.get().unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "success");
        assert_eq!(
            seen_skills.lock().unwrap().as_slice(),
            &[Some("/tmp/skill".to_string())]
        );
    }
}
