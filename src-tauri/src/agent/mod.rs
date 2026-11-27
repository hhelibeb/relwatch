mod executor;
mod prompt;
mod scheduler;
mod terminal;

use std::sync::Arc;

use async_trait::async_trait;

use crate::db::agent::AgentConfig;
pub use crate::error::AgentRunError as AgentError;

pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_TIMEOUT: &str = "timeout";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_UNKNOWN: &str = "unknown";

pub struct AgentContext<'a> {
    pub run_id: i64,
    pub skill_path: Option<&'a str>,
    pub session_path: Option<&'a str>,
    pub entity_texts: &'a [String],
    pub files: &'a [String],
    pub instruction: &'a str,
    pub prompt_suffix: Option<&'a str>,
    pub model: Option<&'a crate::db::agent::AgentModelRef>,
    pub first_turn: bool,
    pub timeout_seconds: u64,
    pub on_stream: Option<&'a (dyn Fn(&serde_json::Value) + Send + Sync)>,
}

pub struct AgentOutcome {
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    fn agent_type(&self) -> &'static str;
    async fn execute(&self, ctx: &AgentContext<'_>) -> Result<AgentOutcome, AgentError>;
}

pub fn executor_for(
    agent_type: &str,
    config: &AgentConfig,
    rpc: Arc<crate::agent_rpc::RpcManager>,
) -> Result<Arc<dyn AgentExecutor>, String> {
    match agent_type {
        "pi" => Ok(Arc::new(RpcExecutor::new(config, rpc))),
        other => Err(format!("err.agent.unsupported_type|{}", other)),
    }
}

pub use crate::agent_rpc::kill_process_tree;
pub use executor::{skill_short_name, RpcExecutor};
pub use prompt::{
    append_local_files, build_prompt, render_release_entity, render_source_entity,
};
pub use scheduler::{dispatch_ctx_from_app, dispatch_run, AgentDispatchCtx};
pub use terminal::{
    agent_process_level_changed, format_session_command, resolve_session_binary, spawn_terminal,
    validate_attached_files, MAX_ATTACHED_FILES, MAX_ATTACHED_FILE_BYTES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_pool;
    use serde_json::json;

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

    #[test]
    fn executor_for_rejects_unknown_type() {
        match executor_for(
            "claude",
            &sample_config(),
            Arc::new(crate::agent_rpc::RpcManager::new(init_memory_pool().unwrap())),
        ) {
            Err(e) => assert!(e.contains("err.agent.unsupported_type|claude")),
            Ok(_) => panic!("expected error"),
        }
        assert!(executor_for(
            "pi",
            &sample_config(),
            Arc::new(crate::agent_rpc::RpcManager::new(init_memory_pool().unwrap())),
        )
        .is_ok());
    }

    #[test]
    fn config_json_round_trip() {
        let v = json!({
            "enabled": true, "agent_type": "pi", "binary": null, "model": "m",
            "prompt_suffix": null, "timeout_seconds": 300, "skills": ["/s1"],
            "working_dir": null, "max_concurrency": 3
        });
        let c: AgentConfig = serde_json::from_value(v).unwrap();
        assert!(c.enabled);
        assert_eq!(c.agent_type, "pi");
        assert_eq!(c.skills, vec!["/s1"]);
        assert_eq!(c.max_concurrency, 3);
    }
}
