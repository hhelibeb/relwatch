use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::{broadcast, oneshot, Notify};

use super::discovery::normalize_path;
use super::os::{self, ProcessScope};
use super::pool::RpcManager;

pub(super) const COMMAND_TIMEOUT_SECS: u64 = 15;
pub(super) const EVENT_CAPACITY: usize = 1024;
const SHUTDOWN_GRACE_MS: u64 = 500;

pub(crate) struct RpcProcess {
    pub(super) _child: tokio::process::Child,
    pub(super) _scope: Option<ProcessScope>,
    pub(super) stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pub(super) pending: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    pub(super) next_id: Arc<AtomicU64>,
    pub(super) dead: Arc<AtomicBool>,
    pub(super) exited: Arc<Notify>,
    pub(super) session: Arc<std::sync::Mutex<Option<String>>>,
    pub(super) events: broadcast::Sender<Value>,
    pub(super) busy_run: AtomicI64,
}

pub struct ProcessLease {
    proc: Arc<RpcProcess>,
    is_temp: bool,
    released: bool,
    mgr: Arc<RpcManager>,
}

impl ProcessLease {
    pub(super) fn new(proc: Arc<RpcProcess>, is_temp: bool, mgr: Arc<RpcManager>) -> Self {
        ProcessLease {
            proc,
            is_temp,
            released: false,
            mgr,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.proc.events.subscribe()
    }

    pub fn pid(&self) -> Option<u32> {
        self.proc.pid()
    }

    pub fn is_temp(&self) -> bool {
        self.is_temp
    }

    #[cfg(test)]
    pub(crate) fn cached_session(&self) -> Option<String> {
        self.proc.session.lock().unwrap().clone()
    }

    pub async fn ensure_session(&self, session_path: &str) -> Result<(), String> {
        let current = self
            .proc
            .session
            .lock()
            .unwrap()
            .clone()
            .map(|s| normalize_path(&s));
        let target = normalize_path(session_path);
        if current.as_deref() != Some(target.as_str()) {
            self.proc
                .command(json!({"type": "switch_session", "sessionPath": session_path}))
                .await?;
            *self.proc.session.lock().unwrap() = Some(session_path.to_string());
        }
        Ok(())
    }

    pub async fn set_model(&self, provider: &str, model_id: &str) -> Result<(), String> {
        self.proc
            .command(json!({"type": "set_model", "provider": provider, "modelId": model_id}))
            .await?;
        Ok(())
    }

    pub async fn prompt(&self, message: &str) -> Result<(), String> {
        self.proc
            .command(json!({"type": "prompt", "message": message}))
            .await?;
        Ok(())
    }

    pub async fn abort_force(&self) {
        if self.proc.command(json!({"type": "abort"})).await.is_err() {
            log::warn!(
                "agent rpc abort 无响应，强杀进程树 (run={})",
                self.proc.busy_run.load(Ordering::SeqCst)
            );
            self.mgr.kill_process(&self.proc).await;
        }
    }

    pub async fn release(mut self) {
        self.released = true;
        if self.is_temp {
            self.mgr.retire_temp(&self.proc).await;
        } else {
            self.mgr.release_permanent(&self.proc).await;
        }
    }

    /// 弃用整个进程：既脱离池，也杀掉进程树。
    ///
    /// 当租约不能确认事件流已归零（未收到 `agent_settled`）时必须走这里：
    /// 把仍有残留事件的进程还给池，下一个任务会收到上一轮的迟到事件。
    pub async fn destroy(mut self) {
        self.released = true;
        self.mgr.kill_process(&self.proc).await;
    }
}

impl Drop for ProcessLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        log::warn!(
            "agent lease dropped without release (run={}); retiring its process",
            self.proc.busy_run.load(Ordering::SeqCst)
        );
        self.proc.kill_detached();
        self.proc.clear_busy();
    }
}

impl RpcProcess {
    pub(super) fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    pub(super) fn is_busy(&self) -> bool {
        self.busy_run.load(Ordering::SeqCst) != 0
    }

    pub(super) fn try_occupy(&self, run_id: i64) -> bool {
        if run_id == 0 || self.is_dead() {
            return false;
        }
        self.busy_run
            .compare_exchange(0, run_id, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub(super) fn clear_busy(&self) {
        self.busy_run.store(0, Ordering::SeqCst);
    }

    pub(super) fn pid(&self) -> Option<u32> {
        if self.dead.load(Ordering::SeqCst) {
            return None;
        }
        self._child.id()
    }

    pub(super) async fn command(&self, mut obj: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst).to_string();
        obj["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let mut line = serde_json::to_string(&obj)
            .map_err(|e| format!("err.agent.rpc_serialize|{}", e))?;
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
                    Err(v
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("err.agent.rpc_rejected")
                        .to_string())
                }
            }
            Ok(Err(_)) => Err("err.agent.rpc_exited".to_string()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("err.agent.rpc_timeout".to_string())
            }
        }
    }

    /// 优雅关闭：关 stdin 后有界等待进程退出，超时则在 blocking 线程强杀整棵进程树（封堵 Unix 孤儿窗口）。
    pub(super) async fn shutdown(&self) {
        let notified = self.exited.notified();
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }
        if !self.is_dead() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(SHUTDOWN_GRACE_MS),
                notified,
            )
            .await;
        }
        if !self.is_dead() {
            self.kill().await;
        } else {
            self.dead.store(true, Ordering::SeqCst);
        }
    }

    /// 不在当前线程上同步跑 `taskkill`/`kill`：Drop 常落在 Tokio worker 上，
    /// 外部命令（尤其是 `Command::output()`）会阻塞整个 worker。
    pub(super) fn kill_detached(&self) {
        self.dead.store(true, Ordering::SeqCst);
        self.exited.notify_waiters();
        let Some(pid) = self._child.id() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(move || os::kill_process_tree(pid));
            }
            Err(_) => os::kill_process_tree(pid),
        }
    }

    pub(super) async fn kill(&self) {
        if let Some(pid) = self._child.id() {
            let _ = tokio::task::spawn_blocking(move || os::kill_process_tree(pid)).await;
        }
        self.dead.store(true, Ordering::SeqCst);
        self.exited.notify_waiters();
    }
}
