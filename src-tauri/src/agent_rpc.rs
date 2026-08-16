//! pi RPC 常驻进程管理 —— 工作区对话的驱动核心。
//!
//! 相比旧的「每次提交 spawn 一个 `pi -p` 一次性进程」模型，RPC 模式是
//! **一个常驻子进程**（`pi --mode rpc`，JSON 协议 over stdin/stdout）：
//!
//! - 提交 = `prompt` 命令（stdin 写一行 JSON），事件流从 stdout 实时返回
//! - 停止 = `abort` 命令（**不杀进程**：会话上下文留在进程内存 + JSONL 文件，
//!   继续对话直接再发 `prompt`，无需任何恢复流程）
//! - 切换会话 = `switch_session <file>`（SessionManager.open 语义：不存在即创建）
//! - 优雅退出 = 关闭 stdin（EOF 触发 pi 自身 shutdown 清理子进程）
//! - 崩溃恢复 = 读线程 EOF 标记 dead，下次命令前自动重启并恢复会话
//!
//! 事件流通过 broadcast channel 分发给「当前 run 的执行器」（RpcExecutor 等待
//! agent_settled 判定终态），同时经 `on_stream` 回调实时转发前端（打字机效果）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::db::agent::{load_agent_config, AgentConfig};

/// 单条命令等待响应的超时（RPC 响应 = 命令被接受/排队，即时返回）。
const COMMAND_TIMEOUT_SECS: u64 = 15;
/// 事件广播容量（文本 delta 高频，留足缓冲；executor 消费慢时允许丢旧帧）。
const EVENT_CAPACITY: usize = 1024;

/// RPC 常驻进程管理器（AppState 单例，工作区共享）。
pub struct RpcManager {
    db_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    inner: Mutex<Option<Arc<RpcProcess>>>,
    events: broadcast::Sender<Value>,
}

impl RpcManager {
    pub fn new(db_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>) -> Self {
        let (tx, _) = broadcast::channel(EVENT_CAPACITY);
        RpcManager {
            db_pool,
            inner: Mutex::new(None),
            events: tx,
        }
    }

    /// 订阅原始 RPC 事件流（executor 每轮 run 订阅一次）。
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    /// 当前是否有存活进程。
    pub async fn is_running(&self) -> bool {
        self.inner
            .lock()
            .await
            .as_ref()
            .map(|p| !p.dead.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// 确保进程存活：惰性启动或崩溃后重启，并恢复上次绑定的会话。
    pub async fn ensure_started(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(p) = guard.as_ref() {
            if !p.dead.load(Ordering::SeqCst) {
                return Ok(());
            }
        }
        // 崩溃前绑定的会话（重启后恢复）
        let prev = guard.as_ref().and_then(|p| p.session.lock().unwrap().clone());
        let proc = self.spawn_process().await?;
        if let Some(path) = prev {
            let _ = proc.command(json!({"type": "switch_session", "sessionPath": path})).await;
            *proc.session.lock().unwrap() = Some(path);
        }
        *guard = Some(Arc::new(proc));
        Ok(())
    }

    /// 确保 RPC 进程绑定到指定会话文件（不存在时 pi 自动创建）。
    pub async fn ensure_session(&self, session_path: &str) -> Result<(), String> {
        self.ensure_started().await?;
        let guard = self.inner.lock().await;
        let proc = guard.as_ref().ok_or_else(|| "err.agent.rpc_not_started".to_string())?;
        let current = proc.session.lock().unwrap().clone().map(|s| normalize_path(&s));
        let target = normalize_path(session_path);
        if current.as_deref() != Some(target.as_str()) {
            proc.command(json!({"type": "switch_session", "sessionPath": session_path})).await?;
            *proc.session.lock().unwrap() = Some(session_path.to_string());
        }
        Ok(())
    }

    /// 发送 prompt 命令（工作区提交）。
    pub async fn prompt(&self, message: &str) -> Result<(), String> {
        self.ensure_started().await?;
        let guard = self.inner.lock().await;
        let proc = guard.as_ref().ok_or_else(|| "err.agent.rpc_not_started".to_string())?;
        proc.command(json!({"type": "prompt", "message": message})).await?;
        Ok(())
    }

    /// 中止当前生成（不杀进程；无进程时静默忽略）。
    /// 返回 Err 表示 abort 命令本身无响应（RPC 进程卡死），调用方应升级为强杀。
    pub async fn abort(&self) -> Result<(), String> {
        let guard = self.inner.lock().await;
        if let Some(proc) = guard.as_ref() {
            proc.command(json!({"type": "abort"})).await.map(|_| ())
        } else {
            Ok(())
        }
    }

    /// 中止当前生成；abort 命令无响应（进程卡死）时升级为强杀进程树，
    /// 下次 ensure_started 自动重启并恢复会话。
    pub async fn abort_force(&self) {
        if self.abort().await.is_err() {
            log::warn!("agent rpc abort 无响应，强杀进程树");
            self.kill_now().await;
        }
    }

    /// 优雅关闭：关 stdin 让 pi 走自身清理流程（杀它 spawn 的工具子进程）。
    pub async fn shutdown(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(proc) = guard.take() {
            proc.shutdown().await;
        }
    }

    /// 强杀进程（abort 无响应等极端兜底）；随后 ensure_started 会重启恢复。
    pub async fn kill_now(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(proc) = guard.take() {
            proc.kill().await;
        }
    }

    /// 从 DB 读 Agent 配置并 spawn RPC 进程。
    async fn spawn_process(&self) -> Result<RpcProcess, String> {
        let config = {
            let conn = self.db_pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
            load_agent_config(&conn)?
        };
        ensure_supported_type(&config)?;
        let binary = resolve_agent_binary(&config)?;

        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(&binary);
        // Windows 下 GUI 父进程 spawn 控制台程序默认会分配新控制台窗口（弹出黑框），
        // 必须显式 CREATE_NO_WINDOW；stdin/stdout 重定向不影响控制台分配。
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        // Unix：pi 作为新进程组首领（setsid 语义），使 kill_process_tree 的负 pid
        // 能整树击杀（含 pi spawn 的 bash 等子进程），且不误伤 relwatch 自身进程组。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.args(["--mode", "rpc", "--no-context-files", "--no-approve", "--no-extensions"]);
        for skill in &config.skills {
            cmd.args(["--skill", skill]);
        }
        if let Some(m) = &config.model {
            cmd.args(["--model", m]);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().map_err(|e| format!("err.agent.spawn|{}", e))?;
        let stdin = tokio::sync::Mutex::new(child.stdin.take().ok_or_else(|| "err.agent.spawn|no stdin".to_string())?);
        let stdout = child.stdout.take().ok_or_else(|| "err.agent.spawn|no stdout".to_string())?;

        let pending = Arc::new(std::sync::Mutex::new(HashMap::<String, oneshot::Sender<Value>>::new()));
        let dead = Arc::new(AtomicBool::new(false));
        let events = self.events.clone();
        let pending_clone = pending.clone();
        let dead_clone = dead.clone();

        // stdout 读循环：response → 投递 pending；事件 → 广播
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                let line = match lines.next_line().await {
                    Ok(Some(l)) => l,
                    _ => break,
                };
                let value: Value = match serde_json::from_str(line.trim()) {
                    Ok(v) => v,
                    Err(_) => continue, // 半行/非 JSON 输出容忍
                };
                if value.get("type").and_then(|t| t.as_str()) == Some("response") {
                    if let Some(id) = value.get("id").and_then(|i| i.as_str()) {
                        if let Some(tx) = pending_clone.lock().unwrap().remove(id) {
                            let _ = tx.send(value);
                        }
                    }
                } else {
                    let _ = events.send(value);
                }
            }
            // EOF：进程退出（正常 shutdown 或崩溃）
            dead_clone.store(true, Ordering::SeqCst);
            for (_, tx) in pending_clone.lock().unwrap().drain() {
                let _ = tx.send(json!({"type": "response", "success": false, "error": "err.agent.rpc_exited"}));
            }
            // 广播合成事件：正在等待事件流的 executor 立即失败返回，而不是干等超时
            // （此前崩溃场景 run 会挂到 deadline 才以 timeout 收敛，前端期间看不到进展）
            let _ = events.send(json!({"type": "rpc_exited"}));
        });

        Ok(RpcProcess {
            _child: child,
            stdin,
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
            dead,
            session: Arc::new(std::sync::Mutex::new(None)),
        })
    }
}

/// 路径归一化（Windows 大小写不敏感比较用）。
fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s.trim_end_matches('/').to_string()
}

/// 校验 Agent 类型受支持（目前仅 pi；新类型在 agent::executor_for 登记）。
pub fn ensure_supported_type(config: &AgentConfig) -> Result<(), String> {
    match config.agent_type.as_str() {
        "pi" => Ok(()),
        other => Err(format!("err.agent.unsupported_type|{}", other)),
    }
}

/// 解析 Agent 可执行文件：显式配置 > PATH 探测 > 常见 npm 全局路径。
pub fn resolve_agent_binary(config: &AgentConfig) -> Result<String, String> {
    if let Some(bin) = &config.binary {
        return Ok(bin.clone());
    }
    let probe = if cfg!(windows) { "where" } else { "which" };
    let mut probe_cmd = std::process::Command::new(probe);
    probe_cmd.arg("pi");
    // Windows 下同样禁止弹出控制台窗口（GUI 父进程 spawn 控制台程序默认弹窗）
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        probe_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    if let Ok(out) = probe_cmd.output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Some(line) = s.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()) {
                    return Ok(line);
                }
            }
        }
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let candidate = std::path::Path::new(&appdata).join("npm").join("pi.cmd");
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }
    }
    Err("err.agent.pi_not_found".to_string())
}

/// 一个存活（或待清理）的 RPC 进程句柄。
struct RpcProcess {
    _child: tokio::process::Child,
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    next_id: Arc<AtomicU64>,
    dead: Arc<AtomicBool>,
    /// 当前绑定的会话文件（崩溃恢复用；ensure_session 更新）。
    session: Arc<std::sync::Mutex<Option<String>>>,
}

impl RpcProcess {
    /// 发送一条命令并等待响应（带超时）。
    async fn command(&self, mut obj: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst).to_string();
        obj["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let mut line = serde_json::to_string(&obj).map_err(|e| format!("err.agent.rpc_serialize|{}", e))?;
        line.push('\n');
        let write = {
            let mut stdin = self.stdin.lock().await;
            let write = stdin.write_all(line.as_bytes()).await;
            let flush = stdin.flush().await;
            write.and(flush)
        };
        if write.is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("err.agent.rpc_write".to_string());
        }
        match tokio::time::timeout(std::time::Duration::from_secs(COMMAND_TIMEOUT_SECS), rx).await {
            Ok(Ok(v)) => {
                if v.get("success").and_then(|s| s.as_bool()).unwrap_or(false) {
                    Ok(v)
                } else {
                    Err(v.get("error").and_then(|e| e.as_str()).unwrap_or("err.agent.rpc_rejected").to_string())
                }
            }
            Ok(Err(_)) => Err("err.agent.rpc_exited".to_string()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("err.agent.rpc_timeout".to_string())
            }
        }
    }

    /// 优雅关闭：关闭 stdin（EOF → pi shutdown）。
    async fn shutdown(&self) {
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }
        // 给 pi 一点清理时间
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        self.dead.store(true, Ordering::SeqCst);
    }

    /// 强杀进程树。
    async fn kill(&self) {
        if let Some(pid) = self._child.id() {
            crate::agent::kill_process_tree(pid);
        }
        self.dead.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_handles_separators_and_case() {
        assert_eq!(normalize_path(r"C:\Data\Sessions\ws-1.jsonl"), "c:/data/sessions/ws-1.jsonl");
        assert_eq!(normalize_path("/data/ws/"), "/data/ws");
        assert_eq!(normalize_path(r"E:\a\b"), "e:/a/b");
    }
}
