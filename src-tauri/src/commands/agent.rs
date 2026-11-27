use crate::agent as agent_runner;
use crate::agent_session::{
    self, is_valid_session_key, session_path_for_key, AgentChatMessage,
};
pub use crate::agent_session::{AgentSessionInfo, PruneAgentSessionsResult};
use crate::db::agent::{self, AgentConfig};
use crate::services::agent_service;
pub use crate::services::agent_service::{AgentJobInput, AgentModelsInfo, AgentRpcStatus};
use crate::types::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_agent_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    agent_service::get_agent_config(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value)]
pub async fn save_agent_config(
    state: tauri::State<'_, AppState>,
    config: AgentConfig,
) -> Result<(), String> {
    agent_service::save_agent_config(&state.db, &state.agent_gate, &state.agent_rpc, config)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_ws_width(state: tauri::State<'_, AppState>) -> Result<i64, String> {
    agent_service::get_agent_ws_width(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn save_agent_ws_width(
    state: tauri::State<'_, AppState>,
    width: i64,
) -> Result<(), String> {
    agent_service::save_agent_ws_width(&state.db, width)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn run_agent_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: AgentJobInput,
) -> Result<i64, String> {
    let run_id = agent_service::enqueue_agent_job(&state.db, &state.agent_rpc, input)
        .await
        .map_err(|e| e.to_ipc_string())?;
    let ctx = agent_runner::dispatch_ctx_from_app(&app);
    tauri::async_runtime::spawn(async move {
        agent_runner::dispatch_run(&ctx, run_id).await;
    });
    Ok(run_id)
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_available_models(
    state: tauri::State<'_, AppState>,
) -> Result<AgentModelsInfo, String> {
    agent_service::get_available_models(&state.db, &state.agent_rpc)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_agent_runs(
    state: tauri::State<'_, AppState>,
    session_key: String,
    limit: Option<i64>,
) -> Result<Vec<agent::AgentRunSummary>, String> {
    agent_service::list_agent_runs(&state.db, session_key, limit)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_queue_status(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<agent::AgentQueueStatus, String> {
    agent_service::get_agent_queue_status(&state.db, session_key)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_queue(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<agent::AgentQueueItem>, String> {
    agent_service::get_agent_queue(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_session_usage(
    session_key: String,
) -> Result<agent_session::AgentSessionUsage, String> {
    agent_service::get_agent_session_usage(session_key)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_agent_messages(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<Vec<AgentChatMessage>, String> {
    agent_service::list_agent_messages(&state.db, session_key)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_agent_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentSessionInfo>, String> {
    agent_service::list_agent_sessions(&state.db)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_agent_run(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<(), String> {
    agent_service::cancel_run(&state.db, &state.agent_rpc, &state.agent_cancelled, run_id)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_agent_session(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<(), String> {
    agent_service::delete_agent_session(
        &state.db,
        &state.agent_rpc,
        &state.agent_cancelled,
        session_key,
    )
    .await
    .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn prune_agent_sessions(
    state: tauri::State<'_, AppState>,
    session_keys: Vec<String>,
) -> Result<PruneAgentSessionsResult, String> {
    agent_service::prune_agent_sessions(
        &state.db,
        &state.agent_rpc,
        &state.agent_cancelled,
        session_keys,
    )
    .await
    .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_rpc_status(
    state: tauri::State<'_, AppState>,
) -> Result<AgentRpcStatus, String> {
    Ok(AgentRpcStatus {
        running: state.agent_rpc.is_running().await,
        pid: state.agent_rpc.pid().await,
        process_count: state.agent_rpc.process_count().await as u32,
        restart_pending: state.agent_rpc.restart_pending(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn restart_agent_rpc(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.agent_rpc.restart_now().await)
}

#[tauri::command]
#[specta::specta]
pub async fn export_agent_session(
    app: tauri::AppHandle,
    session_key: String,
    title: String,
    format: String,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let is_md = match format.as_str() {
        "md" => true,
        "json" => false,
        other => return Err(format!("err.agent.export_bad_format|{}", other)),
    };
    let path = session_path_for_key(&session_key);
    if !path.exists() {
        return Err("err.agent.session_missing".to_string());
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    let default_name = format!(
        "relwatch-session-{}.{}",
        &session_key[..8.min(session_key.len())],
        if is_md { "md" } else { "json" }
    );
    app.dialog()
        .file()
        .add_filter(
            if is_md { "Markdown" } else { "JSON" },
            &[if is_md { "md" } else { "json" }],
        )
        .set_file_name(default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let target = rx
        .await
        .ok()
        .flatten()
        .ok_or("err.agent.export_cancelled")?;
    let target = target
        .as_path()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "err.agent.export_path".to_string())?;

    agent_service::export_agent_session_to_path(path, title, is_md, target)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_session_command(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<String, String> {
    agent_service::get_agent_session_command(&state.db, run_id)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn open_agent_session(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<(), String> {
    agent_service::open_agent_session(&state.db, run_id)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[tauri::command]
#[specta::specta]
pub async fn agent_shutdown_for_update(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.agent_rpc.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{validate_attached_files, MAX_ATTACHED_FILES};
    use crate::agent_session::{
        agent_sessions_dir, align_run_ids, is_safe_session_path, partition_session_keys,
        prune_session_files_and_db, session_key_from_path,
    };
    use crate::db::agent::NewRun;

    fn run_summary(id: i64, started_at: Option<&str>) -> agent::AgentRunSummary {
        agent::AgentRunSummary {
            id,
            session_key: "ws-test".into(),
            skill_path: None,
            entities: "[]".into(),
            instruction: "指令".into(),
            model: None,
            session_path: None,
            status: "success".into(),
            exit_code: Some(0),
            error: None,
            started_at: started_at.map(|s| s.to_string()),
            finished_at: None,
            created_at: "2025-01-01T00:00:00.000Z".into(),
            files: None,
        }
    }

    fn user_message(timestamp: &str) -> AgentChatMessage {
        AgentChatMessage {
            role: "user".into(),
            blocks: Vec::new(),
            timestamp: timestamp.to_string(),
            model: None,
            run_id: None,
        }
    }

    #[test]
    fn validate_attached_files_accepts_existing_files() {
        let dir = std::env::temp_dir().join(format!("relwatch-attach-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.log");
        let b = dir.join("b.png");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "y").unwrap();
        let paths = vec![
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
        ];

        let out = validate_attached_files(Some(&paths)).expect("应通过");
        assert_eq!(out.len(), 2);
        assert!(validate_attached_files(None).unwrap().is_empty());
        assert!(validate_attached_files(Some(&vec![])).unwrap().is_empty());
        let dup = vec!["  ".to_string(), out[0].clone(), out[0].clone()];
        assert_eq!(
            validate_attached_files(Some(&dup)).unwrap(),
            vec![out[0].clone()]
        );

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn validate_attached_files_rejects_bad_paths() {
        let dir = std::env::temp_dir().join(format!("relwatch-attach2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        assert_eq!(
            validate_attached_files(Some(&vec![dir
                .join("nope.log")
                .to_string_lossy()
                .to_string()])),
            Err("err.agent.file_missing".to_string())
        );
        assert_eq!(
            validate_attached_files(Some(&vec![dir.to_string_lossy().to_string()])),
            Err("err.agent.file_not_file".to_string())
        );
        let f = dir.join("one.txt");
        std::fs::write(&f, "x").unwrap();
        let dup: Vec<String> = (0..MAX_ATTACHED_FILES + 1)
            .map(|_| f.to_string_lossy().to_string())
            .collect();
        assert_eq!(validate_attached_files(Some(&dup)).unwrap().len(), 1);

        let many_dedup: Vec<String> = (0..MAX_ATTACHED_FILES + 1)
            .map(|i| {
                let p = dir.join(format!("f{}.txt", i));
                std::fs::write(&p, "x").unwrap();
                p.to_string_lossy().to_string()
            })
            .collect();
        match validate_attached_files(Some(&many_dedup)) {
            Err(e) => assert!(e.starts_with("err.agent.file_too_many|"), "{e}"),
            Ok(v) => panic!("应被数量上限拒绝，实际通过 {} 个", v.len()),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn align_run_ids_pairs_adjacent_runs_in_order() {
        let mut msgs = vec![
            user_message("2025-01-01T00:00:10.000Z"),
            user_message("2025-01-01T00:05:10.000Z"),
        ];
        let runs = vec![
            run_summary(1, Some("2025-01-01T00:00:05.000Z")),
            run_summary(2, Some("2025-01-01T00:05:05.000Z")),
        ];
        align_run_ids(&mut msgs, &runs);
        assert_eq!(msgs[0].run_id, Some(1));
        assert_eq!(msgs[1].run_id, Some(2));
    }

    #[test]
    fn align_run_ids_skips_cancelled_pending_run() {
        let mut msgs = vec![
            user_message("2025-01-01T00:00:10.000Z"),
            user_message("2025-01-01T00:05:10.000Z"),
        ];
        let runs = vec![
            run_summary(1, Some("2025-01-01T00:00:05.000Z")),
            run_summary(2, None),
            run_summary(3, Some("2025-01-01T00:05:05.000Z")),
        ];
        align_run_ids(&mut msgs, &runs);
        assert_eq!(msgs[0].run_id, Some(1));
        assert_eq!(msgs[1].run_id, Some(3));
    }

    #[test]
    fn align_run_ids_skips_run_without_matching_message() {
        let mut msgs = vec![
            user_message("2025-01-01T00:00:10.000Z"),
            user_message("2025-01-01T00:20:10.000Z"),
        ];
        let runs = vec![
            run_summary(1, Some("2025-01-01T00:00:05.000Z")),
            run_summary(2, Some("2025-01-01T00:02:00.000Z")),
            run_summary(3, Some("2025-01-01T00:20:05.000Z")),
        ];
        align_run_ids(&mut msgs, &runs);
        assert_eq!(msgs[0].run_id, Some(1));
        assert_eq!(msgs[1].run_id, Some(3));
    }

    #[test]
    fn align_run_ids_leftover_messages_stay_none() {
        let mut msgs = vec![
            user_message("2025-01-01T00:00:10.000Z"),
            user_message("2025-01-01T00:05:10.000Z"),
        ];
        let runs = vec![run_summary(1, Some("2025-01-01T00:00:05.000Z"))];
        align_run_ids(&mut msgs, &runs);
        assert_eq!(msgs[0].run_id, Some(1));
        assert_eq!(msgs[1].run_id, None);
    }

    #[test]
    fn session_path_uses_sanitized_key() {
        let p = session_path_for_key("abc-123");
        assert!(p.ends_with("ws-abc-123.jsonl"));
    }

    #[test]
    fn session_key_parsed_back_from_file_name() {
        let p = session_path_for_key("f47ac10b-58cc-4372-a567-0e02b2c3d479");
        assert_eq!(
            session_key_from_path(&p).as_deref(),
            Some("f47ac10b-58cc-4372-a567-0e02b2c3d479")
        );
        assert_eq!(
            session_key_from_path(std::path::Path::new("agent-sessions/evil.jsonl")),
            None
        );
        assert_eq!(
            session_key_from_path(std::path::Path::new("agent-sessions/ws-a.txt")),
            None
        );
        assert_eq!(
            session_key_from_path(std::path::Path::new("agent-sessions/ws-.jsonl")),
            None
        );
    }

    #[test]
    fn valid_session_key_accepts_uuid_like_keys() {
        assert!(is_valid_session_key("abc-123"));
        assert!(is_valid_session_key("f47ac10b-58cc-4372-a567-0e02b2c3d479"));
        assert!(is_valid_session_key("ws_1"));
    }

    #[test]
    fn valid_session_key_rejects_path_traversal() {
        assert!(!is_valid_session_key("..\\..\\evil"));
        assert!(!is_valid_session_key("../../evil"));
        assert!(!is_valid_session_key("a/b"));
        assert!(!is_valid_session_key("a\\b"));
        assert!(!is_valid_session_key("a b"));
        assert!(!is_valid_session_key(""));
        assert!(!is_valid_session_key("   "));
        assert!(!is_valid_session_key(&"x".repeat(129)));
        assert!(is_valid_session_key(&"x".repeat(128)));
    }

    #[test]
    fn safe_session_path_rejects_escape_from_agent_dir() {
        let ok = agent_sessions_dir().join("ws-abc.jsonl");
        assert!(is_safe_session_path(&ok.to_string_lossy()));
        assert!(!is_safe_session_path("C:/Windows/evil.jsonl"));
        assert!(!is_safe_session_path("../evil.jsonl"));
    }

    #[test]
    fn prune_session_files_and_db_skips_invalid_keys_and_deletes_valid() {
        let mut conn = crate::db::init::init_memory_db().unwrap();
        let tmp_dir =
            std::env::temp_dir().join(format!("relwatch-prune-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let f1 = tmp_dir.join("ws-s1.jsonl");
        let f2 = tmp_dir.join("ws-s2.jsonl");
        let keep = tmp_dir.join("ws-keep.jsonl");
        std::fs::write(&f1, "{}\n").unwrap();
        std::fs::write(&f2, "{}\n").unwrap();
        std::fs::write(&keep, "{}\n").unwrap();

        for key in ["s1", "s2", "keep"] {
            agent::create_run(
                &conn,
                &NewRun {
                    session_key: key,
                    skill_path: None,
                    entities: &[],
                    instruction: "hi",
                    model: None,
                    session_path: None,
                    files: None,
                },
            )
            .unwrap();
        }

        let (valid, invalid) = partition_session_keys(vec![
            "s1".into(),
            "s1".into(),
            "../evil".into(),
            "s2".into(),
        ]);
        assert_eq!(valid, vec!["s1", "s2"]);
        assert_eq!(invalid, vec!["../evil"]);

        let res = prune_session_files_and_db(&mut conn, &tmp_dir, valid, invalid).unwrap();
        assert_eq!(res.deleted, 2);
        assert_eq!(res.failed, 1);
        assert_eq!(res.failed_keys, vec!["../evil"]);
        assert!(!f1.exists());
        assert!(!f2.exists());
        assert!(keep.exists());
        assert!(agent::list_runs(&conn, "s1", 10).unwrap().is_empty());
        assert!(agent::list_runs(&conn, "s2", 10).unwrap().is_empty());
        assert_eq!(agent::list_runs(&conn, "keep", 10).unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
