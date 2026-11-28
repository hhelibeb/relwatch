use std::sync::Arc;

use async_trait::async_trait;

use crate::db::agent::AgentConfig;
use crate::error::AgentRunError;
use super::prompt::{append_local_files, build_prompt};
use super::{AgentContext, AgentError, AgentExecutor, AgentOutcome};

const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// 中止/取消后等待 `agent_settled` 的上界。
///
/// 收到 `agent_settled` 才说明该进程的事件流已归零，可以安全归还池；超时未收到
/// 则只能销毁进程——否则下一个任务会收到上一轮迟到的事件（误判取消 / `err.agent.end_lost`）。
const SETTLE_BUDGET_SECS: u64 = 3;

/// 租约的一次使用结果。
struct LeasedOutcome {
    result: Result<AgentOutcome, AgentError>,
    /// 事件流是否确认归零（收到 `agent_settled`）；false 时调用方必须销毁进程。
    settled: bool,
}

impl LeasedOutcome {
    /// 进程没有启动过 agent turn（协议层失败），事件流必然干净。
    fn clean(result: Result<AgentOutcome, AgentError>) -> Self {
        Self {
            result,
            settled: true,
        }
    }

    /// 事件流未确认归零：进程不可回池。
    fn dirty(result: Result<AgentOutcome, AgentError>) -> Self {
        Self {
            result,
            settled: false,
        }
    }
}

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
    ) -> LeasedOutcome {
        use serde_json::Value;
        use tokio::sync::broadcast::error::RecvError;

        if let Some(sp) = ctx.session_path {
            if let Err(e) = lease.ensure_session(sp).await {
                return LeasedOutcome::clean(Err(e.into()));
            }
        }
        let mut rx = lease.subscribe();
        let model_set = match ctx.model {
            Some(m) => lease.set_model(&m.provider, &m.model_id).await,
            None => match &self.default_model {
                Some((provider, model_id)) => lease.set_model(provider, model_id).await,
                None => Ok(()),
            },
        };
        if let Err(e) = model_set {
            return LeasedOutcome::clean(Err(e.into()));
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
        if let Err(e) = lease.prompt(&append_local_files(&message, ctx.files)).await {
            // prompt 可能已送达 pi 而只是应答超时：agent 也许已经在跑，不能当干净进程归还
            return LeasedOutcome::dirty(Err(e.into()));
        }

        let timeout = if ctx.timeout_seconds > 0 {
            ctx.timeout_seconds
        } else {
            DEFAULT_TIMEOUT_SECS
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
        let mut stdout = String::new();
        let mut last_messages: Vec<Value> = Vec::new();
        let mut saw_agent_end = false;
        let mut cancelled = false;
        let mut settled = false;
        // 进入收尾阶段后改用独立预算：任务 deadline 那时早已过期，继续拿它算剩余时间会立即放弃等待
        let mut drain_deadline: Option<tokio::time::Instant> = None;

        loop {
            let until = drain_deadline.unwrap_or(deadline);
            let remaining = until.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                if drain_deadline.is_some() {
                    break;
                }
                lease.abort_force().await;
                drain_deadline = Some(
                    tokio::time::Instant::now()
                        + std::time::Duration::from_secs(SETTLE_BUDGET_SECS),
                );
                continue;
            }
            let received = tokio::time::timeout(remaining, rx.recv()).await;
            let value = match received {
                Ok(Ok(v)) => v,
                Ok(Err(RecvError::Lagged(n))) => {
                    log::warn!("agent rpc event stream lagged, skipped {} events", n);
                    continue;
                }
                Ok(Err(RecvError::Closed)) => {
                    return LeasedOutcome::dirty(Err(AgentRunError::key_with_output(
                        "err.agent.rpc_exited",
                        stdout,
                    )))
                }
                Err(_) => {
                    if drain_deadline.is_some() {
                        break;
                    }
                    lease.abort_force().await;
                    drain_deadline = Some(
                        tokio::time::Instant::now()
                            + std::time::Duration::from_secs(SETTLE_BUDGET_SECS),
                    );
                    continue;
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
                        // 用户取消：pi 侧已中止，等 agent_settled 收尾后再让进程回池
                        cancelled = true;
                        if drain_deadline.is_none() {
                            drain_deadline = Some(
                                tokio::time::Instant::now()
                                    + std::time::Duration::from_secs(SETTLE_BUDGET_SECS),
                            );
                        }
                    }
                }
                Some("rpc_exited") => {
                    return LeasedOutcome::dirty(Err(AgentRunError::key_with_output(
                        "err.agent.rpc_exited",
                        stdout,
                    )))
                }
                Some("agent_settled") => {
                    if !saw_agent_end {
                        return LeasedOutcome::dirty(Err(AgentRunError::end_lost(stdout)));
                    }
                    settled = true;
                    break;
                }
                _ => {}
            }
        }

        if cancelled {
            return LeasedOutcome {
                result: Err(AgentRunError::aborted(stdout)),
                settled,
            };
        }
        if !settled {
            return LeasedOutcome {
                result: Err(AgentRunError::timeout_secs(timeout, stdout)),
                settled: false,
            };
        }
        if last_messages.iter().any(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("assistant")
                && m.get("errorMessage").is_some()
        }) {
            return LeasedOutcome {
                result: Err(AgentRunError::key_with_output("err.agent.model_error", stdout)),
                settled,
            };
        }
        LeasedOutcome {
            result: Ok(AgentOutcome {
                exit_code: Some(0),
                stdout,
                stderr: String::new(),
            }),
            settled,
        }
    }
}

#[async_trait]
impl AgentExecutor for RpcExecutor {
    fn agent_type(&self) -> &'static str {
        "pi"
    }

    async fn execute(&self, ctx: &AgentContext<'_>) -> Result<AgentOutcome, AgentError> {
        if ctx.is_cancelled.is_some_and(|f| f()) {
            return Err(AgentRunError::aborted(String::new()));
        }
        let lease = self.rpc.acquire(ctx.run_id).await?;
        // 租进程可能排队；等到的瞬间取消也完全可能刚落下（那时 abort 因 run 未占进程而空转），
        // 此时还没发 prompt，进程干净，直接归还即可。
        if ctx.is_cancelled.is_some_and(|f| f()) {
            lease.release().await;
            return Err(AgentRunError::aborted(String::new()));
        }
        let outcome = self.execute_leased(ctx, &lease).await;
        if outcome.settled {
            lease.release().await;
        } else {
            log::warn!(
                "agent run {} 未收到 agent_settled，销毁进程而非归还池",
                ctx.run_id
            );
            lease.destroy().await;
        }
        outcome.result
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

    use crate::agent_rpc::{AgentConfigProvider, RpcManager};

    struct StaticConfigProvider(AgentConfig);

    impl AgentConfigProvider for StaticConfigProvider {
        fn load_config(&self) -> Result<AgentConfig, String> {
            Ok(self.0.clone())
        }
    }

    fn default_config() -> AgentConfig {
        AgentConfig {
            enabled: true,
            agent_type: "pi".to_string(),
            binary: None,
            model: None,
            skills: vec![],
            prompt_suffix: None,
            timeout_seconds: 30,
            working_dir: None,
            max_concurrency: 3,
        }
    }

    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn fake_rpc_command(script: &'static str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("node");
        cmd.arg("-e").arg(script);
        #[cfg(windows)]
        {
            cmd.creation_flags(0x0800_0000);
        }
        cmd.kill_on_drop(true);
        cmd
    }

    const SETTLED_SCRIPT: &str = r#"
const rl = require('readline').createInterface({ input: process.stdin });
rl.on('line', (line) => {
  if (!line.trim()) return;
  const req = JSON.parse(line);
  console.log(JSON.stringify({ type: 'response', id: req.id, success: true }));
  if (req.type === 'prompt') {
    console.log(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'hi' } }));
    console.log(JSON.stringify({ type: 'agent_end', messages: [{ role: 'assistant', stopReason: 'stop' }] }));
    console.log(JSON.stringify({ type: 'agent_settled' }));
  }
});
"#;

    // 取消后只发 agent_end、永远不发 agent_settled：事件流归零无法确认
    const NO_SETTLE_SCRIPT: &str = r#"
const rl = require('readline').createInterface({ input: process.stdin });
rl.on('line', (line) => {
  if (!line.trim()) return;
  const req = JSON.parse(line);
  console.log(JSON.stringify({ type: 'response', id: req.id, success: true }));
  if (req.type === 'prompt') {
    console.log(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'partial' } }));
    console.log(JSON.stringify({ type: 'agent_end', messages: [{ role: 'assistant', stopReason: 'aborted' }] }));
  }
});
"#;

    fn ctx<'a>(instruction: &'a str) -> AgentContext<'a> {
        AgentContext {
            run_id: 7,
            skill_path: None,
            session_path: None,
            entity_texts: &[],
            files: &[],
            instruction,
            prompt_suffix: None,
            model: None,
            first_turn: true,
            timeout_seconds: 30,
            on_stream: None,
            is_cancelled: None,
        }
    }

    #[tokio::test]
    async fn execute_keeps_process_in_pool_after_agent_settled() {
        if !node_available() {
            return;
        }
        let rpc = Arc::new(RpcManager::with_test_spawn(
            Arc::new(StaticConfigProvider(default_config())),
            |_| fake_rpc_command(SETTLED_SCRIPT),
        ));
        let executor = RpcExecutor::new(&default_config(), rpc.clone());

        let out = executor.execute(&ctx("hi")).await.unwrap();
        assert_eq!(out.stdout, "hi");
        assert_eq!(rpc.process_count().await, 1, "常驻进程应保留在池里");
        assert!(!rpc.has_busy_process().await);

        // 进程已干净，可被下一个任务直接复用
        let again = executor.execute(&ctx("hi")).await.unwrap();
        assert_eq!(again.stdout, "hi");
        assert_eq!(rpc.process_count().await, 1);
    }

    #[tokio::test]
    async fn execute_skips_prompt_when_run_already_cancelled() {
        if !node_available() {
            return;
        }
        let rpc = Arc::new(RpcManager::with_test_spawn(
            Arc::new(StaticConfigProvider(default_config())),
            |_| fake_rpc_command(SETTLED_SCRIPT),
        ));
        let executor = RpcExecutor::new(&default_config(), rpc.clone());

        let cancelled = || true;
        let mut c = ctx("hi");
        c.is_cancelled = Some(&cancelled);
        let err = match executor.execute(&c).await {
            Ok(out) => panic!("应当直接取消，实际执行完成 stdout={}", out.stdout),
            Err(e) => e,
        };
        assert_eq!(err.to_ipc_string(), "err.agent.aborted");
        assert_eq!(
            rpc.process_count().await,
            0,
            "已取消的 run 不应再去租/启一个进程"
        );
    }

    #[tokio::test]
    async fn execute_destroys_process_when_abort_never_settles() {
        if !node_available() {
            return;
        }
        let rpc = Arc::new(RpcManager::with_test_spawn(
            Arc::new(StaticConfigProvider(default_config())),
            |_| fake_rpc_command(NO_SETTLE_SCRIPT),
        ));
        let executor = RpcExecutor::new(&default_config(), rpc.clone());

        let err = match executor.execute(&ctx("hi")).await {
            Ok(out) => panic!("应当返回 aborted 错误，实际成功且 stdout={}", out.stdout),
            Err(e) => e,
        };
        assert_eq!(err.to_ipc_string(), "err.agent.aborted");
        assert_eq!(err.partial_stdout(), "partial");
        assert_eq!(
            rpc.process_count().await,
            0,
            "未确认 agent_settled 的进程必须被销毁而非归还池"
        );
    }

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
