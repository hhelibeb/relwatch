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

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::db::agent::{load_agent_config, AgentConfig};

/// 单条命令等待响应的超时（RPC 响应 = 命令被接受/排队，即时返回）。
const COMMAND_TIMEOUT_SECS: u64 = 15;

/// pi 可选模型（scope model：provider 已配置鉴权、可直接使用）。
/// 来自 RPC `get_available_models` / `get_state` 返回的 Model JSON，仅提取前端需要的字段。
/// 注意 id 可能自带 provider 前缀（如 `cline-pass/deepseek-v4-flash`），
/// 因此 `set_model` 必须用 provider + modelId 双字段精确匹配。
#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct RpcAvailableModel {
    pub provider: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}
/// 事件广播容量（文本 delta 高频，留足缓冲；executor 消费慢时允许丢旧帧）。
const EVENT_CAPACITY: usize = 1024;

/// pi 进程生命周期域（Windows JobObject 句柄）。
///
/// 优雅退出靠「关 stdin → pi 自己清理」，强杀 relwatch 时 OS 关闭管道句柄也能触发
/// pi 退出（见文件头注释）。但**句柄关闭不等于 pi 一定来得及清理它 spawn 的工具子进程**：
/// pi 正在执行长工具时，自身退出与子进程清理之间存在窗口，会留下孤儿。
/// 把 pi 纳入 JobObject 并设 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`：句柄随 RpcProcess
/// 一起释放时，**内核一次性终止作业内全部进程**（含 pi 及其所有后代），窗口由内核关闭。
/// Unix 侧对应机制是 `process_group(0)`（spawn 时已设置，负 pid 整树击杀）。
#[cfg(windows)]
struct ProcessScope {
    job: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ProcessScope {
    /// 创建 JobObject 并把指定进程纳入其中（失败返回 None：降级为无生命周期域，
    /// 不影响主流程——JobObject 只是兜底加固，不是进程运行的前提）。
    fn attach(pid: u32) -> Option<Self> {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

        unsafe {
            let job = match CreateJobObjectW(None, None) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("agent JobObject 创建失败，进程生命周期域降级: {}", e);
                    return None;
                }
            };
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
                std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_ok();
            if !ok {
                log::warn!("agent JobObject 限制设置失败，进程生命周期域降级");
                let _ = CloseHandle(job);
                return None;
            }
            // 打开目标进程需 TERMINATE + SET_QUOTA（后者是 Assign 到 JobObject 所需）
            let proc = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                false,
                pid,
            );
            let proc = match proc {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("agent JobObject 打开进程失败，进程生命周期域降级: {}", e);
                    let _ = CloseHandle(job);
                    return None;
                }
            };
            let assigned = AssignProcessToJobObject(job, proc).is_ok();
            let _ = CloseHandle(proc);
            if !assigned {
                log::warn!("agent JobObject 纳管进程失败，进程生命周期域降级");
                let _ = CloseHandle(job);
                return None;
            }
            let _: HANDLE = job;
            Some(ProcessScope { job })
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessScope {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        // 关句柄即触发 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE：作业内进程被内核终止。
        // 正常路径下 pi 早已优雅退出（作业为空，无副作用）；异常路径才是它的意义所在。
        unsafe {
            let _ = CloseHandle(self.job);
        }
    }
}

// Windows 句柄非 Send（原始指针），ProcessScope 只在 spawn/kill 路径上被持有与释放，
// 不跨线程共享；显式声明以满足 RpcProcess 的 Send 约束。
#[cfg(windows)]
unsafe impl Send for ProcessScope {}
#[cfg(windows)]
unsafe impl Sync for ProcessScope {}

/// RPC 常驻进程管理器（AppState 单例，工作区共享）。
pub struct RpcManager {
    db_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    inner: Mutex<Option<Arc<RpcProcess>>>,
    events: broadcast::Sender<Value>,
    /// 最近一次 spawn 时读取的 scoped-models 模式快照（None = 尚未 spawn 过）。
    /// pi settings.json 的 enabledModels 变化不会自动生效（spawn 时经 --models
    /// 传入一次），检测到与当前不一致时重启进程使新配置生效。
    spawned_models: Mutex<Option<Vec<String>>>,
    /// 延迟重启标记（H-6）：进程级配置变更时若有 running run，先置位不打断生成，
    /// 当前 run 结束（dispatch_run 收尾 restart_if_pending）后再 kill 重启生效。
    pending_restart: AtomicBool,
}

impl RpcManager {
    pub fn new(db_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>) -> Self {
        let (tx, _) = broadcast::channel(EVENT_CAPACITY);
        RpcManager {
            db_pool,
            inner: Mutex::new(None),
            events: tx,
            spawned_models: Mutex::new(None),
            pending_restart: AtomicBool::new(false),
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

    /// 是否有正在执行的 run（占用 RPC 进程）。进程级操作（kill 重启 / set_model 回写）
    /// 的守卫：避免打断正在生成的 run。查询失败时保守返回 true（不执行进程级操作）。
    pub async fn has_running_run(&self) -> bool {
        let Ok(conn) = self.db_pool.get() else {
            return true;
        };
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE status = 'running')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true)
    }

    /// 请求重启常驻进程（进程级配置变更后调用，H-6）：有正在执行的 run 时
    /// **不打断**生成（kill 会以 rpc_exited 中断当前 run、记 failed 烧掉 token），
    /// 置延迟标记，由 dispatch_run 收尾的 restart_if_pending 在 run 结束后重启；
    /// 空闲时立即重启。与 sync_scoped_models 的 running 守卫语义一致。
    pub async fn request_restart(&self) {
        if self.has_running_run().await {
            log::info!(
                "agent config process-level fields changed but a run is in progress; deferring restart"
            );
            self.pending_restart.store(true, Ordering::SeqCst);
        } else {
            self.kill_now().await;
        }
    }

    /// 消费延迟重启标记（dispatch_run 收尾调用）：当前 run 已结束，执行之前被
    /// 推迟的 kill，使新配置在下次 ensure_started（下一次提交）生效。
    pub async fn restart_if_pending(&self) {
        if self.pending_restart.swap(false, Ordering::SeqCst) {
            log::info!("agent deferred restart now executing after run finished");
            self.kill_now().await;
        }
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

    /// scoped-models 同步：pi settings.json 的 enabledModels 变化时（spawn 后仅启动时
    /// 经 --models 传入一次）重启常驻进程，使新模型集合生效。首次 spawn 前不动作
    /// （ensure_started 会按当前配置启动）。
    /// 有正在执行的 run 时**不重启**（kill 会以 rpc_exited 中断生成）；快照保持旧值，
    /// 下次检测仍会尝试，直到进程空闲。
    async fn sync_scoped_models(&self) {
        let current = read_scoped_model_patterns();
        let changed = match self.spawned_models.lock().await.as_ref() {
            None => false, // 尚未 spawn：ensure_started 按当前配置启动即可
            Some(old) => old != &current,
        };
        if changed {
            if self.has_running_run().await {
                log::info!("agent scoped-models changed but a run is in progress; deferring restart");
                return;
            }
            log::info!("agent scoped-models changed, restarting RPC process");
            self.kill_now().await;
            // 快照保留旧值直到 spawn 成功更新（spawn 失败时下次检测仍会重启重试）
        }
    }

    /// 枚举 pi 当前可用的模型（scope model：已配置鉴权）。
    pub async fn get_available_models(&self) -> Result<Vec<RpcAvailableModel>, String> {
        self.sync_scoped_models().await;
        self.ensure_started().await?;
        let guard = self.inner.lock().await;
        let proc = guard.as_ref().ok_or_else(|| "err.agent.rpc_not_started".to_string())?;
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

    /// 切换 pi 当前模型到指定 provider + modelId（`set_model`）。
    pub async fn set_model(&self, provider: &str, model_id: &str) -> Result<(), String> {
        self.ensure_started().await?;
        let guard = self.inner.lock().await;
        let proc = guard.as_ref().ok_or_else(|| "err.agent.rpc_not_started".to_string())?;
        proc.command(json!({"type": "set_model", "provider": provider, "modelId": model_id})).await?;
        Ok(())
    }

    /// 读 pi 进程当前激活模型（「默认」选项的实际落点；无模型时 None）。
    pub async fn get_current_model(&self) -> Result<Option<RpcAvailableModel>, String> {
        self.ensure_started().await?;
        let guard = self.inner.lock().await;
        let proc = guard.as_ref().ok_or_else(|| "err.agent.rpc_not_started".to_string())?;
        let resp = proc.command(json!({"type": "get_state"})).await?;
        let m = resp.pointer("/data/model");
        Ok(m.and_then(|v| v.as_object()).map(|m| RpcAvailableModel {
            provider: m.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            id: m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            name: m.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
        }))
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

        // 平台条件化启动：Windows 经 cmd /C 包裹（npm 全局安装的 pi.cmd 是批处理，
        // CreateProcess 无法直接执行 .cmd，必须交给 cmd 解释）；Unix 直接 spawn
        // 二进制（which 探测返回的是可执行文件），不能套 cmd（Unix 无 cmd，ENOENT
        // 会导致所有提交 err.agent.spawn，Agent 工作区完全不可用）。
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(&binary);
            // Windows 下 GUI 父进程 spawn 控制台程序默认会分配新控制台窗口（弹出黑框），
            // 必须显式 CREATE_NO_WINDOW；stdin/stdout 重定向不影响控制台分配。
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new(&binary);
            // Unix：pi 作为新进程组首领（setsid 语义），使 kill_process_tree 的负 pid
            // 能整树击杀（含 pi spawn 的 bash 等子进程），且不误伤 relwatch 自身进程组。
            c.process_group(0);
            c
        };
        // 工作目录：全局配置指定时作为 pi 进程 cwd（bash 工具继承，避免默认落在
        // 安装目录/项目根；空串视为未配置）。
        if let Some(wd) = config.working_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            cmd.current_dir(wd);
        }
        cmd.args(["--mode", "rpc", "--no-context-files", "--no-approve", "--no-extensions"]);
        for skill in &config.skills {
            cmd.args(["--skill", skill]);
        }
        if let Some(m) = &config.model {
            cmd.args(["--model", m]);
        }
        // scoped-models：用户通过 pi `/scoped-models` 启用/禁用循环的模型集合，
        // 持久化在 pi settings.json 的 enabledModels。传给 RPC 进程让 pi 原生解析为
        // scopedModels（会话内循环/默认模型一致），工作区模型下拉也按此过滤。
        let scoped_patterns = read_scoped_model_patterns();
        if !scoped_patterns.is_empty() {
            cmd.args(["--models", &scoped_patterns.join(",")]);
        }
        // 记录本次 spawn 的 scoped-models 快照（sync_scoped_models 检测变化用）
        *self.spawned_models.lock().await = Some(scoped_patterns);
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

        // 纳管进生命周期域（此时进程刚 spawn，尚未 spawn 自己的子进程，纳管即时生效）
        #[cfg(windows)]
        let scope = child.id().and_then(ProcessScope::attach);

        Ok(RpcProcess {
            #[cfg(windows)]
            _scope: scope,
            _child: child,
            stdin,
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
            dead,
            session: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// 当前进程 pid（无存活进程时 None）——进程健康指示的数据源。
    pub async fn pid(&self) -> Option<u32> {
        let guard = self.inner.lock().await;
        guard.as_ref().and_then(|p| p.pid())
    }

    /// 进程级配置变更是否正处于「推迟生效」状态（等当前 run 结束后重启）。
    ///
    /// 用户改了 pi 路径 / 模型 / skill 后，若有 run 正在生成，重启会被推迟以避免
    /// 打断生成（烧掉已产生的词元）。此前这段时间 UI 无任何提示，用户以为改了没生效
    /// （评审 3.8）；现在把它暴露出来，前端可在设置保存后提示「将在当前任务结束后生效」。
    pub fn restart_pending(&self) -> bool {
        self.pending_restart.load(Ordering::SeqCst)
    }

    /// 手动重启常驻进程（进程健康指示的「一键重启」）。
    ///
    /// 有正在执行的 run 时拒绝重启（与 request_restart 同守卫）：kill 会以 rpc_exited
    /// 中断当前 run、记 failed 且已产生的词元作废，代价太大。
    /// 返回是否真的执行了重启（false = 因有 run 在跑而拒绝）。
    pub async fn restart_now(&self) -> bool {
        if self.has_running_run().await {
            log::info!("agent manual restart skipped: a run is in progress");
            return false;
        }
        self.kill_now().await;
        true
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

// ---- scoped-models：读取 pi settings.json 的 enabledModels 并按模式过滤 ----

/// pi 的 agent 配置目录（settings.json 所在目录）。对齐 pi config.getAgentDir()：
/// 环境变量 `PI_CODING_AGENT_DIR` 优先（直接用其值），否则默认 `~/.pi/agent`。
pub fn scoped_settings_path() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        let t = dir.trim();
        if !t.is_empty() {
            return Some(std::path::Path::new(t).join("settings.json"));
        }
    }
    dirs::home_dir().map(|h| h.join(".pi").join("agent").join("settings.json"))
}

/// 读取 pi settings.json 的 `enabledModels`（`/scoped-models` 命令持久化的模型模式列表，
/// 例如 `["clinepass/cline-pass/qwen3.8-max", "opencode-go/*"]`）。
/// 读不到 / 解析失败 / 为空 → 空列表（表示不限 scope，显示全部可用模型）。
pub fn read_scoped_model_patterns() -> Vec<String> {
    let Some(path) = scoped_settings_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    v.get("enabledModels")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn is_thinking_level(s: &str) -> bool {
    matches!(s, "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max")
}

/// 裁剪 `pattern:thinking` 后缀（仅当后缀是合法思考等级时才裁，避免误伤模型 id 里的冒号）。
fn strip_thinking_suffix(pattern: &str) -> &str {
    if let Some(idx) = pattern.rfind(':') {
        if is_thinking_level(&pattern[idx + 1..]) {
            return &pattern[..idx];
        }
    }
    pattern
}

fn has_glob(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

/// 简易 glob（`*` / `?`），大小写不敏感。对齐 pi 用 minimatch 覆盖 `provider/id` 与 `id`。
fn glob_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') => {
                // `*` 匹配零或多个字符
                (0..=t.len()).any(|i| rec(&p[1..], &t[i..]))
            }
            Some(b'?') => !t.is_empty() && rec(&p[1..], &t[1..]),
            Some(&c) => {
                !t.is_empty()
                    && t[0].eq_ignore_ascii_case(&c)
                    && rec(&p[1..], &t[1..])
            }
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

/// 单个 model 是否命中某个 scoped 模式（对齐 pi resolveModelScopeFromModels 的常见语义）：
/// - 全局模式（`*`/`?`/`[`）→ 对 `provider/id` 与 `id` 做 glob 匹配
/// - 非全局 → 依次精确匹配 `provider/id`、`id`、`name`；否则大小写不敏感子串兜底
pub fn model_matches_scoped_pattern(pattern: &str, m: &RpcAvailableModel) -> bool {
    let pat = strip_thinking_suffix(pattern);
    let full = format!("{}/{}", m.provider, m.id);
    if has_glob(pat) {
        return glob_match(pat, &full) || glob_match(pat, &m.id);
    }
    let patl = pat.to_lowercase();
    let full_l = full.to_lowercase();
    let id_l = m.id.to_lowercase();
    let name_l = m.name.as_deref().unwrap_or("").to_lowercase();
    if full_l == patl || id_l == patl || (!name_l.is_empty() && name_l == patl) {
        return true;
    }
    full_l.contains(&patl) || id_l.contains(&patl) || (!name_l.is_empty() && name_l.contains(&patl))
}

/// 按 scoped 模式过滤模型列表（保持原顺序去重）。
/// 无 scoped 配置或解析结果为空时回退到全量（避免下拉空，防御模式不命中）。
pub fn filter_scoped_models(models: Vec<RpcAvailableModel>, patterns: &[String]) -> Vec<RpcAvailableModel> {
    if patterns.is_empty() {
        return models;
    }
    let mut out: Vec<RpcAvailableModel> = Vec::new();
    for m in &models {
        if patterns.iter().any(|p| model_matches_scoped_pattern(p, m)) && !out.contains(m) {
            out.push(m.clone());
        }
    }
    if out.is_empty() {
        models
    } else {
        out
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
    /// Windows 生命周期域（JobObject 句柄）；Unix 侧靠 process_group 击杀整树，无需它。
    #[cfg(windows)]
    _scope: Option<ProcessScope>,
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    next_id: Arc<AtomicU64>,
    dead: Arc<AtomicBool>,
    /// 当前绑定的会话文件（崩溃恢复用；ensure_session 更新）。
    session: Arc<std::sync::Mutex<Option<String>>>,
}

impl RpcProcess {
    /// 进程 pid（进程已退出时 tokio 仍返回上次的 id，故与 dead 标记一起判定）。
    fn pid(&self) -> Option<u32> {
        if self.dead.load(Ordering::SeqCst) {
            return None;
        }
        self._child.id()
    }

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
        assert_eq!(normalize_path("/data/ws/"), "/data/ws");
        // 大小写归一化仅在 Windows 生效（路径比较在 Windows 大小写不敏感，Unix 区分大小写）
        if cfg!(windows) {
            assert_eq!(normalize_path(r"C:\Data\Sessions\ws-1.jsonl"), "c:/data/sessions/ws-1.jsonl");
            assert_eq!(normalize_path(r"E:\a\b"), "e:/a/b");
        }
    }

    /// JobObject 生命周期域：句柄释放时作业内进程应被内核终止。
    ///
    /// 这段是本次改动里唯一的 unsafe FFI，且失败是静默的（日志 warn 后降级），
    /// 必须有运行时验证——否则「纳管失败」与「纳管成功但标志无效」都无人察觉。
    #[cfg(windows)]
    #[test]
    fn process_scope_terminates_child_on_drop() {
        use std::process::{Command, Stdio};
        // `cmd /C pause` 等一次按键：stdin 管道保持打开且从不写入 → 进程会一直阻塞，
        // 用它当「不会自己退出」的探针，避免依赖 sleep 时长（测试确定性与耗时都更好）。
        let mut child = Command::new("cmd")
            .args(["/C", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cmd /C pause");
        let pid = child.id();

        let scope = ProcessScope::attach(pid).expect("JobObject 纳管应成功");
        drop(scope); // 关闭句柄 → JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE 生效

        // 子进程应被终止；若纳管/标志无效，pause 会一直阻塞到超时，测试失败
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok(),
            "JobObject 句柄关闭后，作业内的子进程应被内核终止"
        );
    }

    fn model(provider: &str, id: &str, name: Option<&str>) -> RpcAvailableModel {
        RpcAvailableModel {
            provider: provider.to_string(),
            id: id.to_string(),
            name: name.map(|s| s.to_string()),
        }
    }

    #[test]
    fn glob_match_handles_star_and_question() {
        assert!(glob_match("opencode-go/*", "opencode-go/qwen3.7-max"));
        assert!(glob_match("cline-pass/*deepseek*", "cline-pass/deepseek-v4-flash"));
        assert!(!glob_match("opencode-go/*", "clinepass/deepseek-v4-flash"));
        assert!(glob_match("gpt?", "gpt1"));
        assert!(!glob_match("gpt?", "gpt12"));
        assert!(glob_match("*sonnet*", "anthropic/claude-sonnet-4"));
    }

    #[test]
    fn strip_thinking_suffix_only_for_levels() {
        assert_eq!(strip_thinking_suffix("deepseek-v4:high"), "deepseek-v4");
        assert_eq!(strip_thinking_suffix("deepseek-v4"), "deepseek-v4");
        // 模型 id 末尾的冒号（如 openrouter :exacto）不被当作思考等级误裁
        assert_eq!(strip_thinking_suffix("openrouter/gpt-4o:exacto"), "openrouter/gpt-4o:exacto");
    }

    #[test]
    fn model_matches_scoped_pattern_exact_and_fuzzy() {
        let m = model("clinepass", "cline-pass/deepseek-v4-flash", Some("DeepSeek V4 Flash (ClinePass)"));
        // 精确 provider/id 命中
        assert!(model_matches_scoped_pattern("clinepass/cline-pass/deepseek-v4-flash", &m));
        // id 精确命中
        assert!(model_matches_scoped_pattern("cline-pass/deepseek-v4-flash", &m));
        // name 精确命中
        assert!(model_matches_scoped_pattern("DeepSeek V4 Flash (clinepass)", &m));
        // 子串模糊命中
        assert!(model_matches_scoped_pattern("deepseek-v4", &m));
        // 不匹配
        assert!(!model_matches_scoped_pattern("qwen3.7-max", &m));
    }

    #[test]
    fn filter_scoped_models_selects_only_enabled_and_dedups() {
        let all = vec![
            model("deepseek", "deepseek-v4-flash", Some("DeepSeek V4 Flash")),
            model("opencode-go", "qwen3.7-max", Some("Qwen3.7 Max")),
            model("clinepass", "cline-pass/qwen3.8-max", Some("Qwen3.8 Max (ClinePass)")),
            model("clinepass", "cline-pass/deepseek-v4-flash", Some("DeepSeek V4 Flash (ClinePass)")),
        ];
        let patterns = vec![
            "clinepass/cline-pass/qwen3.8-max".to_string(),
            "clinepass/cline-pass/deepseek-v4-flash".to_string(),
        ];
        let filtered = filter_scoped_models(all.clone(), &patterns);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|m| m.id == "cline-pass/qwen3.8-max"));
        assert!(filtered.iter().any(|m| m.id == "cline-pass/deepseek-v4-flash"));
        // 保持原顺序
        assert_eq!(filtered[0].id, "cline-pass/qwen3.8-max");
    }

    #[test]
    fn filter_scoped_models_empty_patterns_or_no_match_falls_back_to_all() {
        let all = vec![model("a", "x", None), model("b", "y", None)];
        // 无 scoped 配置 → 全量
        assert_eq!(filter_scoped_models(all.clone(), &[]).len(), 2);
        // 有模式但都不命中 → 回退全量（防空下拉）
        let nohit = vec!["zzz-none".to_string()];
        assert_eq!(filter_scoped_models(all, &nohit).len(), 2);
    }
}
