use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, oneshot, Mutex, Notify};

use crate::db::agent::{load_agent_config, AgentConfig, MAX_AGENT_CONCURRENCY};
use super::discovery::{ensure_supported_type, read_scoped_model_patterns, resolve_agent_binary};
use super::os::{self, ProcessScope};
use super::process::{ProcessLease, RpcProcess, EVENT_CAPACITY};
use super::RpcAvailableModel;

const MAX_TEMP_PROCESSES: usize = MAX_AGENT_CONCURRENCY as usize - 1;

/// Agent 配置提供者：解耦 RpcManager 对 r2d2::Pool 的直接依赖，便于独立测试。
pub trait AgentConfigProvider: Send + Sync {
    fn load_config(&self) -> Result<AgentConfig, String>;
}

pub struct DbConfigProvider {
    pool: crate::db::DbPool,
}

impl DbConfigProvider {
    pub fn new(pool: crate::db::DbPool) -> Self {
        Self { pool }
    }
}

impl AgentConfigProvider for DbConfigProvider {
    fn load_config(&self) -> Result<AgentConfig, String> {
        let conn = self
            .pool
            .get()
            .map_err(|e| format!("err.db_connect|{}", e))?;
        load_agent_config(&conn).map_err(|e| e.to_ipc_string())
    }
}

#[derive(Default)]
struct PoolState {
    permanent: Option<Arc<RpcProcess>>,
    temps: Vec<Arc<RpcProcess>>,
    spawning_permanent: bool,
    spawning_temps: usize,
}

impl PoolState {
    fn sweep_dead(&mut self) {
        if self.permanent.as_ref().is_some_and(|p| p.is_dead()) {
            log::info!("agent permanent RPC process died; will respawn on next use");
            self.permanent = None;
        }
        let before = self.temps.len();
        self.temps.retain(|p| !p.is_dead());
        if self.temps.len() != before {
            log::info!(
                "agent dropped {} dead temp RPC process(es)",
                before - self.temps.len()
            );
        }
    }
}

/// 锁外 spawn 期间的槽位占位守卫：若 spawn 失败或 future 被取消，自动回退占位并唤醒等待者。
struct SpawnReservation {
    pool: Arc<Mutex<PoolState>>,
    notify: Arc<Notify>,
    as_permanent: bool,
    armed: bool,
}

impl SpawnReservation {
    fn new(pool: Arc<Mutex<PoolState>>, notify: Arc<Notify>, as_permanent: bool) -> Self {
        Self {
            pool,
            notify,
            as_permanent,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pool = self.pool.clone();
        let notify = self.notify.clone();
        let as_permanent = self.as_permanent;
        let released = {
            if let Ok(mut guard) = pool.try_lock() {
                if as_permanent {
                    guard.spawning_permanent = false;
                } else {
                    guard.spawning_temps = guard.spawning_temps.saturating_sub(1);
                }
                notify.notify_waiters();
                true
            } else {
                false
            }
        };
        if !released {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let mut guard = pool.lock().await;
                    if as_permanent {
                        guard.spawning_permanent = false;
                    } else {
                        guard.spawning_temps = guard.spawning_temps.saturating_sub(1);
                    }
                    notify.notify_waiters();
                });
            }
        }
    }
}

#[cfg(test)]
type SpawnCommandFactory = Arc<dyn Fn(&AgentConfig) -> tokio::process::Command + Send + Sync>;

pub struct RpcManager {
    config_provider: Arc<dyn AgentConfigProvider>,
    pool: Arc<Mutex<PoolState>>,
    pool_notify: Arc<Notify>,
    spawned_models: std::sync::Mutex<Option<Vec<String>>>,
    pending_restart: AtomicBool,
    #[cfg(test)]
    spawn_override: Option<SpawnCommandFactory>,
}

impl RpcManager {
    pub fn new(db_pool: crate::db::DbPool) -> Self {
        Self::with_provider(Arc::new(DbConfigProvider::new(db_pool)))
    }

    pub fn with_provider(config_provider: Arc<dyn AgentConfigProvider>) -> Self {
        RpcManager {
            config_provider,
            pool: Arc::new(Mutex::new(PoolState::default())),
            pool_notify: Arc::new(Notify::new()),
            spawned_models: std::sync::Mutex::new(None),
            pending_restart: AtomicBool::new(false),
            #[cfg(test)]
            spawn_override: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_spawn<F>(
        config_provider: Arc<dyn AgentConfigProvider>,
        factory: F,
    ) -> Self
    where
        F: Fn(&AgentConfig) -> tokio::process::Command + Send + Sync + 'static,
    {
        let mut mgr = Self::with_provider(config_provider);
        mgr.spawn_override = Some(Arc::new(factory));
        mgr
    }

    pub async fn is_running(&self) -> bool {
        let pool = self.pool.lock().await;
        pool.permanent.as_ref().is_some_and(|p| !p.is_dead())
            || pool.temps.iter().any(|p| !p.is_dead())
    }

    pub async fn process_count(&self) -> usize {
        let pool = self.pool.lock().await;
        let permanent = usize::from(pool.permanent.as_ref().is_some_and(|p| !p.is_dead()));
        permanent + pool.temps.iter().filter(|p| !p.is_dead()).count()
    }

    pub async fn has_busy_process(&self) -> bool {
        let pool = self.pool.lock().await;
        pool.permanent.as_ref().is_some_and(|p| p.is_busy())
            || pool.temps.iter().any(|p| p.is_busy())
    }

    /// 租用进程槽位：仅在检查/占位与登记两段短暂持有 `pool` 锁，spawn 在锁外执行，避免阻塞池管理与并行租用。
    pub async fn acquire(self: &Arc<Self>, run_id: i64) -> Result<ProcessLease, String> {
        let as_permanent = {
            let mut pool = self.pool.lock().await;
            pool.sweep_dead();

            if let Some(p) = pool.permanent.as_ref() {
                if p.try_occupy(run_id) {
                    return Ok(ProcessLease::new(p.clone(), false, self.clone()));
                }
            }
            if let Some(p) = pool.temps.iter().find(|p| p.try_occupy(run_id)).cloned() {
                return Ok(ProcessLease::new(p, true, self.clone()));
            }

            let as_permanent = pool.permanent.is_none() && !pool.spawning_permanent;
            if as_permanent {
                pool.spawning_permanent = true;
            } else {
                if pool.temps.len() + pool.spawning_temps >= MAX_TEMP_PROCESSES {
                    log::error!(
                        "agent process pool full ({} active + {} spawning temp processes), rejecting run {}",
                        pool.temps.len(),
                        pool.spawning_temps,
                        run_id
                    );
                    return Err("err.agent.pool_full".to_string());
                }
                pool.spawning_temps += 1;
            }
            as_permanent
        };

        let mut reservation =
            SpawnReservation::new(self.pool.clone(), self.pool_notify.clone(), as_permanent);
        let proc = Arc::new(self.spawn_process().await?);
        proc.try_occupy(run_id);

        {
            let mut pool = self.pool.lock().await;
            reservation.disarm();
            if as_permanent {
                pool.spawning_permanent = false;
                pool.permanent = Some(proc.clone());
            } else {
                pool.spawning_temps = pool.spawning_temps.saturating_sub(1);
                pool.temps.push(proc.clone());
            }
            self.pool_notify.notify_waiters();
        }
        Ok(ProcessLease::new(proc, !as_permanent, self.clone()))
    }

    pub(super) async fn retire_temp(&self, proc: &Arc<RpcProcess>) {
        {
            let mut pool = self.pool.lock().await;
            pool.temps.retain(|p| !Arc::ptr_eq(p, proc));
            proc.clear_busy();
        }
        proc.kill().await;
    }

    pub(super) async fn release_permanent(&self, proc: &Arc<RpcProcess>) {
        let mut pool = self.pool.lock().await;
        if proc.is_dead() && pool.permanent.as_ref().is_some_and(|p| Arc::ptr_eq(p, proc)) {
            pool.permanent = None;
        }
        proc.clear_busy();
    }

    pub(super) async fn kill_process(&self, proc: &Arc<RpcProcess>) {
        {
            let mut pool = self.pool.lock().await;
            pool.temps.retain(|p| !Arc::ptr_eq(p, proc));
            if pool.permanent.as_ref().is_some_and(|p| Arc::ptr_eq(p, proc)) {
                pool.permanent = None;
            }
        }
        proc.kill().await;
    }

    async fn take_all_if_idle(&self) -> Option<Vec<Arc<RpcProcess>>> {
        let mut pool = self.pool.lock().await;
        pool.sweep_dead();
        if pool.spawning_permanent
            || pool.spawning_temps > 0
            || pool.permanent.as_ref().is_some_and(|p| p.is_busy())
            || pool.temps.iter().any(|p| p.is_busy())
        {
            return None;
        }
        let mut all: Vec<Arc<RpcProcess>> = Vec::new();
        all.extend(pool.permanent.take());
        all.append(&mut pool.temps);
        Some(all)
    }

    pub async fn request_restart(&self) {
        self.pending_restart.store(true, Ordering::SeqCst);
        self.restart_idle_if_pending().await;
    }

    pub async fn restart_if_pending(&self) {
        self.restart_idle_if_pending().await;
    }

    async fn restart_idle_if_pending(&self) {
        if !self.pending_restart.load(Ordering::SeqCst) {
            return;
        }
        let Some(procs) = self.take_all_if_idle().await else {
            log::info!("agent pool restart deferred: a run is in progress");
            return;
        };
        for p in procs {
            p.kill().await;
        }
        self.pending_restart.store(false, Ordering::SeqCst);
        log::info!("agent deferred pool restart executed");
    }

    async fn permanent(&self) -> Result<Arc<RpcProcess>, String> {
        loop {
            let notified = {
                let mut pool = self.pool.lock().await;
                if let Some(p) = pool.permanent.as_ref() {
                    if !p.is_dead() {
                        return Ok(p.clone());
                    }
                    pool.permanent = None;
                }
                if pool.spawning_permanent {
                    Some(self.pool_notify.notified())
                } else {
                    pool.spawning_permanent = true;
                    None
                }
            };
            if let Some(wait) = notified {
                wait.await;
                continue;
            }

            let mut reservation =
                SpawnReservation::new(self.pool.clone(), self.pool_notify.clone(), true);
            let proc = Arc::new(self.spawn_process().await?);
            let mut pool = self.pool.lock().await;
            reservation.disarm();
            pool.spawning_permanent = false;
            pool.permanent = Some(proc.clone());
            self.pool_notify.notify_waiters();
            return Ok(proc);
        }
    }

    async fn sync_scoped_models(&self) {
        let current = tokio::task::spawn_blocking(read_scoped_model_patterns)
            .await
            .unwrap_or_default();
        let changed = {
            let snapshot = self.spawned_models.lock().unwrap();
            match snapshot.as_ref() {
                None => false,
                Some(old) => old != &current,
            }
        };
        if changed {
            log::info!("agent scoped-models changed, requesting RPC pool restart");
            self.pending_restart.store(true, Ordering::SeqCst);
            self.restart_idle_if_pending().await;
        }
    }

    pub async fn get_available_models(&self) -> Result<Vec<RpcAvailableModel>, String> {
        self.sync_scoped_models().await;
        let proc = self.permanent().await?;
        let resp = proc.command(json!({"type": "get_available_models"})).await?;
        let models = resp
            .pointer("/data/models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| "err.agent.rpc_bad_response".to_string())?;
        let mut out = Vec::new();
        for m in models {
            let provider = m.get("provider").and_then(|v| v.as_str()).unwrap_or("");
            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if provider.is_empty() || id.is_empty() {
                continue;
            }
            out.push(RpcAvailableModel {
                provider: provider.to_string(),
                id: id.to_string(),
                name: m.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
            });
        }
        Ok(out)
    }

    pub async fn set_model(&self, provider: &str, model_id: &str) -> Result<(), String> {
        let proc = self.permanent().await?;
        proc.command(json!({"type": "set_model", "provider": provider, "modelId": model_id}))
            .await?;
        Ok(())
    }

    pub async fn get_current_model(&self) -> Result<Option<RpcAvailableModel>, String> {
        let proc = self.permanent().await?;
        let resp = proc.command(json!({"type": "get_state"})).await?;
        let m = resp.pointer("/data/model");
        Ok(m.and_then(|v| v.as_object()).map(|m| RpcAvailableModel {
            provider: m
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            id: m
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            name: m.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
        }))
    }

    pub async fn abort_run(&self, run_id: i64) {
        let target = {
            let pool = self.pool.lock().await;
            pool.permanent
                .as_ref()
                .filter(|p| p.busy_run.load(Ordering::SeqCst) == run_id)
                .or_else(|| {
                    pool.temps
                        .iter()
                        .find(|p| p.busy_run.load(Ordering::SeqCst) == run_id)
                })
                .cloned()
        };
        let Some(proc) = target else {
            log::info!("agent abort skipped: run {} has no hosting process", run_id);
            return;
        };
        if proc.command(json!({"type": "abort"})).await.is_err() {
            log::warn!("agent rpc abort 无响应，强杀进程树 (run={})", run_id);
            self.kill_process(&proc).await;
        }
    }

    pub async fn shutdown(&self) {
        let procs: Vec<Arc<RpcProcess>> = {
            let mut pool = self.pool.lock().await;
            let mut all: Vec<Arc<RpcProcess>> = Vec::new();
            all.extend(pool.permanent.take());
            all.append(&mut pool.temps);
            all
        };
        if procs.is_empty() {
            return;
        }
        futures_util::future::join_all(procs.iter().map(|p| p.shutdown())).await;
    }

    async fn spawn_process(&self) -> Result<RpcProcess, String> {
        let provider = self.config_provider.clone();
        #[cfg(test)]
        let has_override = self.spawn_override.is_some();
        #[cfg(not(test))]
        let has_override = false;

        let (config, binary, scoped_patterns) = tokio::task::spawn_blocking(move || {
            let config = provider.load_config()?;
            ensure_supported_type(&config)?;
            let binary = if has_override {
                String::new()
            } else {
                resolve_agent_binary(&config)?
            };
            let scoped = read_scoped_model_patterns();
            Ok::<_, String>((config, binary, scoped))
        })
        .await
        .map_err(|e| format!("err.task_failed|{}", e))??;

        let mut cmd = {
            #[cfg(test)]
            if let Some(factory) = &self.spawn_override {
                factory(&config)
            } else {
                build_pi_command(&config, &binary, &scoped_patterns)
            }
            #[cfg(not(test))]
            {
                build_pi_command(&config, &binary, &scoped_patterns)
            }
        };

        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().map_err(|e| format!("err.agent.spawn|{}", e))?;
        *self.spawned_models.lock().unwrap() = Some(scoped_patterns);
        let stdin = tokio::sync::Mutex::new(
            child
                .stdin
                .take()
                .ok_or_else(|| "err.agent.spawn|no stdin".to_string())?,
        );
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "err.agent.spawn|no stdout".to_string())?;

        let pending = Arc::new(std::sync::Mutex::new(HashMap::<
            String,
            oneshot::Sender<Value>,
        >::new()));
        let dead = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(Notify::new());
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let events_tx = events.clone();
        let pending_clone = pending.clone();
        let dead_clone = dead.clone();
        let exited_clone = exited.clone();

        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                let line = match lines.next_line().await {
                    Ok(Some(l)) => l,
                    _ => break,
                };
                let value: Value = match serde_json::from_str(line.trim()) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if value.get("type").and_then(|t| t.as_str()) == Some("response") {
                    if let Some(id) = value.get("id").and_then(|i| i.as_str()) {
                        if let Some(tx) = pending_clone.lock().unwrap().remove(id) {
                            let _ = tx.send(value);
                        }
                    }
                } else {
                    let _ = events_tx.send(value);
                }
            }
            dead_clone.store(true, Ordering::SeqCst);
            exited_clone.notify_waiters();
            for (_, tx) in pending_clone.lock().unwrap().drain() {
                let _ = tx.send(json!({
                    "type": "response",
                    "success": false,
                    "error": "err.agent.rpc_exited"
                }));
            }
            let _ = events_tx.send(json!({"type": "rpc_exited"}));
        });

        let scope = child.id().and_then(ProcessScope::attach);

        Ok(RpcProcess {
            _scope: scope,
            _child: child,
            stdin,
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
            dead,
            exited,
            session: Arc::new(std::sync::Mutex::new(None)),
            events,
            busy_run: AtomicI64::new(0),
        })
    }

    pub async fn pid(&self) -> Option<u32> {
        let pool = self.pool.lock().await;
        pool.permanent.as_ref().and_then(|p| p.pid())
    }

    pub fn restart_pending(&self) -> bool {
        self.pending_restart.load(Ordering::SeqCst)
    }

    pub async fn restart_now(&self) -> bool {
        let Some(procs) = self.take_all_if_idle().await else {
            log::info!("agent manual restart skipped: a run is in progress");
            return false;
        };
        for p in procs {
            p.kill().await;
        }
        self.pending_restart.store(false, Ordering::SeqCst);
        true
    }
}

fn build_pi_command(
    config: &AgentConfig,
    binary: &str,
    scoped_patterns: &[String],
) -> tokio::process::Command {
    let mut cmd = os::configure_spawn_command(binary);
    if let Some(wd) = config
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cmd.current_dir(wd);
    }
    cmd.args([
        "--mode",
        "rpc",
        "--no-context-files",
        "--no-approve",
        "--no-extensions",
    ]);
    for skill in &config.skills {
        cmd.args(["--skill", skill]);
    }
    if let Some(m) = &config.model {
        cmd.args(["--model", m]);
    }
    if !scoped_patterns.is_empty() {
        cmd.args(["--models", &scoped_patterns.join(",")]);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticConfigProvider(AgentConfig);

    impl AgentConfigProvider for StaticConfigProvider {
        fn load_config(&self) -> Result<AgentConfig, String> {
            Ok(self.0.clone())
        }
    }

    fn default_test_config() -> AgentConfig {
        AgentConfig {
            enabled: true,
            agent_type: "pi".to_string(),
            binary: None,
            model: None,
            skills: vec![],
            prompt_suffix: None,
            timeout_seconds: 60,
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

    /// 跨平台极简 mock RPC 进程（基于 node）：支持 get_available_models/get_state/switch_session/prompt/abort/crash。
    fn fake_rpc_command(script: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("node");
        cmd.arg("-e").arg(script);
        #[cfg(windows)]
        {
            cmd.creation_flags(0x0800_0000);
        }
        cmd.kill_on_drop(true);
        cmd
    }

    const FAKE_RPC_SCRIPT: &str = r#"
const rl = require('readline').createInterface({ input: process.stdin });
rl.on('line', (line) => {
  if (!line.trim()) return;
  const req = JSON.parse(line);
  if (req.type === 'get_available_models') {
    console.log(JSON.stringify({ type: 'response', id: req.id, success: true, data: { models: [{ provider: 'mock', id: 'm1', name: 'Mock 1' }] } }));
  } else if (req.type === 'get_state') {
    console.log(JSON.stringify({ type: 'response', id: req.id, success: true, data: { model: { provider: 'mock', id: 'm1' } } }));
  } else if (req.type === 'switch_session' || req.type === 'set_model') {
    console.log(JSON.stringify({ type: 'response', id: req.id, success: true }));
  } else if (req.type === 'prompt') {
    if (req.message === 'CRASH') {
      process.exit(1);
    }
    console.log(JSON.stringify({ type: 'response', id: req.id, success: true }));
    if (req.message !== 'HANG') {
      console.log(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'hi' } }));
      console.log(JSON.stringify({ type: 'agent_end' }));
      console.log(JSON.stringify({ type: 'agent_settled' }));
    }
  } else if (req.type === 'abort') {
    console.log(JSON.stringify({ type: 'response', id: req.id, success: true }));
    console.log(JSON.stringify({ type: 'agent_end' }));
    console.log(JSON.stringify({ type: 'agent_settled' }));
  }
});
"#;

    #[tokio::test]
    async fn empty_pool_management_ops_are_noops() {
        let rpc = RpcManager::with_provider(Arc::new(StaticConfigProvider(default_test_config())));
        assert!(!rpc.is_running().await);
        assert_eq!(rpc.process_count().await, 0);
        assert!(rpc.pid().await.is_none());
        assert!(!rpc.has_busy_process().await);
        rpc.abort_run(1).await;
        assert!(rpc.restart_now().await);
    }

    #[tokio::test]
    async fn restart_request_consumed_immediately_when_idle() {
        let rpc = RpcManager::with_provider(Arc::new(StaticConfigProvider(default_test_config())));
        assert!(!rpc.restart_pending());
        rpc.request_restart().await;
        assert!(!rpc.restart_pending());
        rpc.restart_if_pending().await;
        assert!(!rpc.restart_pending());
    }

    #[tokio::test]
    async fn shutdown_returns_immediately_without_process() {
        let rpc = RpcManager::with_provider(Arc::new(StaticConfigProvider(default_test_config())));
        let t0 = std::time::Instant::now();
        rpc.shutdown().await;
        assert!(t0.elapsed() < std::time::Duration::from_millis(500));
    }

    #[tokio::test]
    async fn pool_lease_concurrent_temp_abort_crash_recovery_and_shutdown() {
        if !node_available() {
            return;
        }
        let rpc = Arc::new(RpcManager::with_test_spawn(
            Arc::new(StaticConfigProvider(default_test_config())),
            |_| fake_rpc_command(FAKE_RPC_SCRIPT),
        ));

        // 1. 首次 acquire 拿到常驻进程，并发 acquire 拿到临时进程
        let lease1 = rpc.acquire(101).await.expect("acquire permanent");
        assert!(!lease1.is_temp());
        assert_eq!(rpc.process_count().await, 1);
        assert!(rpc.has_busy_process().await);

        let lease2 = rpc.acquire(102).await.expect("acquire temp");
        assert!(lease2.is_temp());
        assert_eq!(rpc.process_count().await, 2);

        // 2. 运行中 restart_now 必须拒绝，request_restart 置为延迟
        assert!(!rpc.restart_now().await);
        rpc.request_restart().await;
        assert!(rpc.restart_pending());

        // 3. 精确 abort_run 触发 lease2 事件流而不误伤 lease1
        let mut rx2 = lease2.subscribe();
        lease2.prompt("HANG").await.unwrap();
        rpc.abort_run(102).await;
        let ev = tokio::time::timeout(std::time::Duration::from_secs(3), rx2.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ev["type"], "agent_end");

        // 4. 临时进程 release 后立刻退役销毁，常驻进程保留
        lease2.release().await;
        assert_eq!(rpc.process_count().await, 1);

        // 5. 崩溃恢复：触发常驻进程崩溃后，下次 get_available_models 自动重建
        let mut rx1 = lease1.subscribe();
        let _ = lease1.prompt("CRASH").await;
        let exit_ev = tokio::time::timeout(std::time::Duration::from_secs(3), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exit_ev["type"], "rpc_exited");
        lease1.release().await;

        let models = rpc.get_available_models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "m1");

        // 6. 延迟重启在空闲后被消费
        rpc.restart_if_pending().await;
        assert!(!rpc.restart_pending());
        assert_eq!(rpc.process_count().await, 0);

        // 7. shutdown 优雅关闭并迅速返回
        let _ = rpc.get_current_model().await.unwrap();
        assert_eq!(rpc.process_count().await, 1);
        let t0 = std::time::Instant::now();
        rpc.shutdown().await;
        assert_eq!(rpc.process_count().await, 0);
        assert!(t0.elapsed() < std::time::Duration::from_secs(2));
    }

    #[tokio::test]
    async fn shutdown_force_kills_unresponsive_process_after_grace_period() {
        if !node_available() {
            return;
        }
        // 故意忽略 stdin EOF 死等定时器，验证 shutdown 超时后强杀兜底
        let stubborn = "setInterval(() => {}, 10000);";
        let rpc = Arc::new(RpcManager::with_test_spawn(
            Arc::new(StaticConfigProvider(default_test_config())),
            move |_| fake_rpc_command(stubborn),
        ));
        let lease = rpc.acquire(201).await.unwrap();
        lease.release().await;
        assert!(rpc.is_running().await);

        let t0 = std::time::Instant::now();
        rpc.shutdown().await;
        assert!(!rpc.is_running().await);
        assert!(t0.elapsed() >= std::time::Duration::from_millis(450));
        assert!(t0.elapsed() < std::time::Duration::from_secs(4));
    }
}
