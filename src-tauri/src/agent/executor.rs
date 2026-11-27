use std::sync::Arc;

use async_trait::async_trait;

use crate::db::agent::AgentConfig;
use crate::error::AgentRunError;
use super::prompt::{append_local_files, build_prompt};
use super::{AgentContext, AgentError, AgentExecutor, AgentOutcome};

const DEFAULT_TIMEOUT_SECS: u64 = 300;

pub fn skill_short_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let mut segs: Vec<&str> = trimmed.split(['/', '\\']).collect();
    let mut seg = segs.pop().unwrap_or("");
    if !seg.is_empty() && !segs.is_empty() && has_file_extension(seg) {
        seg = segs.pop().unwrap_or(seg);
    }
    if seg.is_empty() {
        trimmed.to_string()
    } else {
        seg.to_string()
    }
}

fn has_file_extension(seg: &str) -> bool {
    seg.rsplit_once('.')
        .map(|(_, ext)| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or(false)
}

pub struct RpcExecutor {
    rpc: Arc<crate::agent_rpc::RpcManager>,
    prompt_suffix: Option<String>,
    default_model: Option<(String, String)>,
}

impl RpcExecutor {
    pub fn new(config: &AgentConfig, rpc: Arc<crate::agent_rpc::RpcManager>) -> Self {
        let default_model = config
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|m| m.split_once('/'))
            .map(|(p, id)| (p.to_string(), id.to_string()));
        RpcExecutor {
            rpc,
            prompt_suffix: config.prompt_suffix.clone(),
            default_model,
        }
    }

    async fn execute_leased(
        &self,
        ctx: &AgentContext<'_>,
        lease: &crate::agent_rpc::ProcessLease,
    ) -> Result<AgentOutcome, AgentError> {
        use serde_json::Value;
        use tokio::sync::broadcast::error::RecvError;

        if let Some(sp) = ctx.session_path {
            lease.ensure_session(sp).await?;
        }
        let mut rx = lease.subscribe();
        match ctx.model {
            Some(m) => lease.set_model(&m.provider, &m.model_id).await?,
            None => {
                if let Some((provider, model_id)) = &self.default_model {
                    lease.set_model(provider, model_id).await?;
                }
            }
        }

        let base = build_prompt(
            ctx.entity_texts,
            ctx.instruction,
            self.prompt_suffix.as_deref(),
            ctx.skill_path.is_some(),
            ctx.first_turn,
        );
        let message = match ctx.skill_path {
            Some(skill) => format!("/skill:{} {}", skill_short_name(skill), base),
            None => base,
        };
        lease.prompt(&append_local_files(&message, ctx.files)).await?;

        let timeout = if ctx.timeout_seconds > 0 {
            ctx.timeout_seconds
        } else {
            DEFAULT_TIMEOUT_SECS
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
        let mut stdout = String::new();
        let mut last_messages: Vec<Value> = Vec::new();
        let mut saw_agent_end = false;
        let mut settled = false;

        while !settled {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                lease.abort_force().await;
                return Err(AgentRunError::timeout_secs(timeout, stdout));
            }
            let received = tokio::time::timeout(remaining, rx.recv()).await;
            let value = match received {
                Ok(Ok(v)) => v,
                Ok(Err(RecvError::Lagged(n))) => {
                    log::warn!("agent rpc event stream lagged, skipped {} events", n);
                    continue;
                }
                Ok(Err(RecvError::Closed)) => {
                    return Err(AgentRunError::key_with_output("err.agent.rpc_exited", stdout))
                }
                Err(_) => {
                    lease.abort_force().await;
                    return Err(AgentRunError::timeout_secs(timeout, stdout));
                }
            };
            if let Some(cb) = ctx.on_stream {
                cb(&value);
            }
            match value.get("type").and_then(|t| t.as_str()) {
                Some("message_update") => {
                    if let Some(delta) = value
                        .pointer("/assistantMessageEvent/delta")
                        .and_then(|d| d.as_str())
                    {
                        stdout.push_str(delta);
                    }
                }
                Some("agent_end") => {
                    saw_agent_end = true;
                    if let Some(msgs) = value.get("messages").and_then(|m| m.as_array()) {
                        last_messages = msgs.clone();
                    }
                    if is_aborted(&last_messages) {
                        return Err(AgentRunError::aborted(stdout));
                    }
                }
                Some("rpc_exited") => {
                    return Err(AgentRunError::key_with_output("err.agent.rpc_exited", stdout))
                }
                Some("agent_settled") => {
                    if !saw_agent_end {
                        return Err(AgentRunError::end_lost(stdout));
                    }
                    settled = true;
                }
                _ => {}
            }
        }

        if last_messages.iter().any(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("assistant")
                && m.get("errorMessage").is_some()
        }) {
            return Err(AgentRunError::key_with_output("err.agent.model_error", stdout));
        }
        Ok(AgentOutcome {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
        })
    }
}

#[async_trait]
impl AgentExecutor for RpcExecutor {
    fn agent_type(&self) -> &'static str {
        "pi"
    }

    async fn execute(&self, ctx: &AgentContext<'_>) -> Result<AgentOutcome, AgentError> {
        let lease = self.rpc.acquire(ctx.run_id).await?;
        let result = self.execute_leased(ctx, &lease).await;
        lease.release().await;
        result
    }
}

fn is_aborted(messages: &[serde_json::Value]) -> bool {
    messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("assistant")
            && m.get("stopReason").and_then(|s| s.as_str()) == Some("aborted")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_short_name_matches_ts_impl() {
        assert_eq!(skill_short_name(r"E:\.pi\skills\commit\SKILL.md"), "commit");
        assert_eq!(skill_short_name("skills/commit/SKILL.md"), "commit");
        assert_eq!(skill_short_name(".pi/skills/release/SKILL.md"), "release");
        assert_eq!(skill_short_name("/tmp/skill"), "skill");
        assert_eq!(skill_short_name("skills/commit"), "commit");
        assert_eq!(skill_short_name("E:/pi/skills"), "skills");
        assert_eq!(skill_short_name("commit"), "commit");
        assert_eq!(skill_short_name("skills/commit/"), "commit");
        assert_eq!(skill_short_name("/"), "");
        assert_eq!(skill_short_name("skills/release/notes"), "notes");
    }
}
