//! Agent 工作区命令：全局配置读写、工作区提交运行、运行记录查询、会话恢复。
//!
//! 交互模型：用户在版本列表 / 监控源唤起工作区（右侧面板），通过拖拽 / `[[]]`
//! 引用实体（source / release），`@` 选择全局 skill，提交后后台运行 pi 无头进程；
//! 同一会话（session_key）的后续提交复用 pi 会话文件实现多轮继续。
//! 结束状态经 `AgentRunFinished` 事件推送前端刷新。

use crate::agent::{
    self as agent_runner, agent_process_level_changed, format_session_command,
    resolve_session_binary, spawn_terminal, validate_attached_files,
};
use crate::agent_session::{
    self, agent_sessions_dir, align_run_ids, discover_disk_sessions, is_safe_session_path,
    is_valid_session_key, partition_session_keys, prune_session_files_and_db,
    remove_session_file_in, session_path_for_key, AgentChatMessage,
};
pub use crate::agent_session::{AgentSessionInfo, PruneAgentSessionsResult};
use crate::db::agent::{self, AgentConfig, AgentEntityRef, AgentModelRef, NewRun};
use crate::db::logs;
use crate::types::AppState;
use serde::{Deserialize, Serialize};
use specta::Type;

/// 读取全局 Agent 配置。
#[tauri::command]
#[specta::specta]
pub fn get_agent_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::load_agent_config(&conn)
}

/// 保存全局 Agent 配置。
/// 进程级字段（agent_type / binary / model / working_dir / skills）变化时重启 RPC 进程池：
/// spawn 只在启动时读一次这些字段，不重启则新配置静默不生效（新增 skill 后 @ 它
/// 会回到 /skill: 透传失效，改 model 会静默用旧模型）；有 run 在跑时推迟到当前 run
/// 结束，下次提交自动重建。
/// max_concurrency 变化不重启进程，只调调度层闸门：抬高立即多出执行位，压低则等
/// 正在跑的 run 收尾归还执行位后到位。
/// timeout / prompt_suffix / enabled 每次调度重读，无需重启。
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value)]
pub async fn save_agent_config(
    state: tauri::State<'_, AppState>,
    config: AgentConfig,
) -> Result<(), String> {
    // 工作目录必须存在：spawn 时作为 pi 进程 cwd，填错路径 spawn 失败且错误
    // 信息不友好（含 OS error），保存时即拒绝（空串 = 未配置，跳过校验）。
    if let Some(wd) = config.working_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !std::path::Path::new(wd).is_dir() {
            return Err("err.agent.working_dir_not_found".to_string());
        }
    }
    let old = {
        let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let old = agent::load_agent_config(&conn)?;
        agent::save_agent_config(&mut conn, &config)?;
        logs::write_log_key(
            &conn,
            "INFO",
            "agent.config_saved",
            &serde_json::json!({"enabled": config.enabled, "skills": config.skills.len()}).to_string(),
        );
        old
    };
    let old_max = old.max_concurrency;
    let new_max = state.agent_gate.set_limit(config.max_concurrency);
    if new_max != old_max {
        log::info!("agent concurrency limit changed: {} -> {}", old_max, new_max);
    }
    if agent_process_level_changed(&old, &config) {
        // 不直接 kill：正在生成的 run 会被中断并浪费词元，改为有活跃 run 时延迟到收尾后重建
        state.agent_rpc.request_restart().await;
    }
    Ok(())
}

/// 读取 Agent 工作区面板宽度（逻辑 px；未设置返回 0，前端回退默认 440）。
#[tauri::command]
#[specta::specta]
pub fn get_agent_ws_width(state: tauri::State<'_, AppState>) -> Result<i64, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::load_agent_ws_width(&conn)
}

/// 保存 Agent 工作区面板宽度（前端拖窗口右边框调节后写入）。
#[tauri::command]
#[specta::specta]
pub fn save_agent_ws_width(
    state: tauri::State<'_, AppState>,
    width: i64,
) -> Result<(), String> {
    // 防御：面板宽度只允许合理范围（1..=2000 逻辑 px）
    if !(1..=2000).contains(&width) {
        return Err("err.agent.ws_width_range".to_string());
    }
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::save_agent_ws_width(&conn, width)
}

/// 工作区提交的完整输入。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentJobInput {
    /// 工作区会话标识（前端 UUID）。同会话多次提交共享 pi 会话文件（多轮继续）。
    pub session_key: String,
    /// 本次提交引用的实体（拖拽 / `[[]]` 解析结果）。
    pub entities: Vec<AgentEntityRef>,
    /// 本次提交使用的 skill 路径（`@` 选择；None = 全局列表首个）。
    pub skill_path: Option<String>,
    /// 用户输入文本（引用已解析剥离）。
    pub instruction: String,
    /// 本次提交显式选择的模型（None = 跟随 pi 当前/默认模型）。
    pub model: Option<AgentModelRef>,
    /// 本次提交附带的本地文件绝对路径（文件对话框自选；空/None = 无附件）。
    #[serde(default)]
    pub files: Option<Vec<String>>,
}

/// 工作区提交：校验 → 建 run → 后台调度执行，返回 run_id。
///
/// - Agent 未启用、skill 未配置、实体无效时拒绝；
/// - 会话文件已存在（历史提交）则复用，实现多轮继续；
/// - 实际执行在后台任务完成，终态经 `AgentRunFinished` 事件推送。
#[tauri::command]
#[specta::specta]
pub async fn run_agent_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: AgentJobInput,
) -> Result<i64, String> {
    let session_key = input.session_key.trim().to_string();
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let instruction = input.instruction.trim().to_string();
    let (entities, skill_path, session_path) = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let config = agent::load_agent_config(&conn)?;
        if !config.enabled {
            return Err("err.agent.disabled".to_string());
        }
        let mut entities: Vec<AgentEntityRef> = Vec::new();
        for r in &input.entities {
            if entities.iter().any(|e| e.kind == r.kind && e.id == r.id) {
                continue;
            }
            let exists = match r.kind.as_str() {
                "source" => crate::db::sources::get_source(&conn, r.id)?.is_some(),
                "release" => crate::db::releases::get_release(&conn, r.id)?.is_some(),
                _ => false,
            };
            if !exists {
                return Err("err.agent.entity_missing".to_string());
            }
            entities.push(r.clone());
        }
        let skill_path = match input.skill_path.as_deref().map(|s| s.trim().to_string()) {
            Some(p) if !p.is_empty() => {
                if !config.skills.contains(&p) {
                    return Err("err.agent.skill_not_configured".to_string());
                }
                Some(p)
            }
            _ => None,
        };
        let existing_session_path = match agent::get_session_path(&conn, &session_key)? {
            Some(p) if is_safe_session_path(&p) => Some(p),
            _ => None,
        };
        (entities, skill_path, existing_session_path)
    };

    let files = validate_attached_files(input.files.as_ref())?;

    if instruction.is_empty() && entities.is_empty() && files.is_empty() {
        return Err("err.agent.empty_job".to_string());
    }

    let model_json: Option<String> = input
        .model
        .as_ref()
        .map(|m| serde_json::to_string(m).map_err(|e| e.to_string()))
        .transpose()?;

    // 显式指定模型需在入队前预检（避免排队后才报泛化错误）；None 跟随 pi 当前默认模型跳过预检
    if let Some(m) = &input.model {
        verify_model_available(&state, m).await?;
    }

    let session_path = match session_path {
        Some(p) => Some(p),
        None => {
            let path = session_path_for_key(&session_key);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            Some(path.to_string_lossy().to_string())
        }
    };

    let files_json: Option<String> = if files.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&files).map_err(|e| e.to_string())?)
    };

    // BEGIN IMMEDIATE 串行化同会话并发提交，防止 check 与 insert 间的 TOCTOU。
    let run_id = {
        let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if agent::active_run_count_for_session(&tx, &session_key)? > 0 {
            return Err("err.agent.session_busy".to_string());
        }
        let run_id = agent::create_run(&tx, &NewRun { session_key: &session_key, skill_path: skill_path.as_deref(), entities: &entities, instruction: &instruction, model: model_json.as_deref(), session_path: session_path.as_deref(), files: files_json.as_deref() })?;
        tx.commit().map_err(|e| e.to_string())?;
        run_id
    };

    let ctx = agent_runner::dispatch_ctx_from_app(&app);
    tauri::async_runtime::spawn(async move {
        agent_runner::dispatch_run(&ctx, run_id).await;
    });
    Ok(run_id)
}

/// 校验显式选择的模型在 pi 侧确实可用（提交前预检）。
///
/// 匹配口径与 `set_model` 一致（provider + modelId 双字段精确匹配）：
/// pi 的 model id 可能自带 provider 前缀（如 `cline-pass/deepseek-v4-flash`），
/// 不能用 `provider/id` 拼接做比较。
async fn verify_model_available(state: &AppState, m: &AgentModelRef) -> Result<(), String> {
    let models = state.agent_rpc.get_available_models().await?;
    let ok = models
        .iter()
        .any(|x| x.provider == m.provider && x.id == m.model_id);
    if ok {
        return Ok(());
    }
    // 列表为空时无法判定「不可用」（可能是 RPC 枚举异常而非鉴权失效），
    // 放行让真实调用去暴露问题，避免把枚举故障谎报成鉴权故障。
    if models.is_empty() {
        log::warn!("agent model precheck skipped: pi returned an empty model list");
        return Ok(());
    }
    Err(format!(
        "err.agent.model_unavailable|{}/{}",
        m.provider, m.model_id
    ))
}

/// 工作区可选模型信息：pi 当前可用模型列表 + 当前激活模型（「默认」选项实际落点）。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentModelsInfo {
    /// scope model：pi 已配置鉴权、可直接使用的模型列表。
    pub models: Vec<crate::agent_rpc::RpcAvailableModel>,
    /// pi 进程当前激活模型（None = 无模型）。「默认」选项将使用该模型。
    pub current: Option<crate::agent_rpc::RpcAvailableModel>,
}

/// 查询 pi 当前可用模型（scope model）与当前激活模型，供工作区模型下拉。
/// 惰性拉取：RPC 进程未启动则先启动（常驻进程，后续 run 复用）。
/// Agent 未启用时直接返回空（不拉起常驻进程，避免无谓资源占用）。
#[tauri::command]
#[specta::specta]
pub async fn get_agent_available_models(
    state: tauri::State<'_, AppState>,
) -> Result<AgentModelsInfo, String> {
    // 先确认 Agent 类型受支持（模型枚举/切换目前仅 pi 支持）
    let config = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        agent::load_agent_config(&conn)?
    };
    crate::agent_rpc::ensure_supported_type(&config)?;
    // 未启用：不惰性拉起常驻 pi 进程（打开工作区只耗一次 RPC 枚举）；
    // 下拉只剩「默认」项，启用后重新打开工作区即恢复。
    if !config.enabled {
        return Ok(AgentModelsInfo { models: Vec::new(), current: None });
    }
    let models = state.agent_rpc.get_available_models().await?;
    // 模型下拉只展示 pi 的 scoped-models（settings.json enabledModels 解析的模型集合）
    let scoped = crate::agent_rpc::read_scoped_model_patterns();
    let models = crate::agent_rpc::filter_scoped_models(models, &scoped);
    let mut current = state.agent_rpc.get_current_model().await?;
    // 「默认」落点修复：进程当前模型会被上一个会话的显式选择污染，而用户直觉
    // 「默认 = 全局配置 model」。全局配置了 model（可解析出 provider/id）且与进程
    // 当前不一致时，恢复为全局 model（纯 id 无法精确 set_model，保持现状）。
    if let Some(cfg_m) = config.model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let matches = current
            .as_ref()
            .map(|c| cfg_m == c.id || cfg_m == format!("{}/{}", c.provider, c.id))
            .unwrap_or(false);
        if !matches {
            if let Some((provider, model_id)) = cfg_m.split_once('/') {
                // 守卫：有正在执行的 run 时跳过回写——set_model 是进程级状态操作，
                // 正在生成的 run 可能在 prompt 前刚 set_model(显式选择)，中途切模型
                // 会污染本次生成。run 结束后下次打开工作区再恢复默认。
                if !state.agent_rpc.has_busy_process().await {
                    state.agent_rpc.set_model(provider, model_id).await?;
                    current = state.agent_rpc.get_current_model().await?;
                }
            }
        }
    }
    Ok(AgentModelsInfo { models, current })
}

/// 查询工作区会话的提交记录（倒序摘要，不含 stdout/stderr 大字段，默认最近 20 条）。
#[tauri::command]
#[specta::specta]
pub fn list_agent_runs(
    state: tauri::State<'_, AppState>,
    session_key: String,
    limit: Option<i64>,
) -> Result<Vec<agent::AgentRunSummary>, String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let limit = limit.unwrap_or(20).clamp(1, 100);
    agent::list_run_summaries(&conn, &session_key, limit).map_err(|e| e.to_string())
}

/// 查询全局 Agent 队列状态：本会话 pending run 的队列位置 + 其他会话占用情况。
/// 「排队中」提示的数据源：pending 说明已达并发上限（`agent_max_concurrency`），
/// 前端据此提示「等待空闲执行位」并展示占用者。
/// 注意并发上限 > 1 时，其他会话正在跑不等于本会话被阻塞（还有空闲位就直接上）。
#[tauri::command]
#[specta::specta]
pub fn get_agent_queue_status(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<agent::AgentQueueStatus, String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::agent_queue_status(&conn, &session_key).map_err(|e| e.to_string())
}

/// 查询全局队列（全部活跃 run，按执行顺序升序）。
///
/// 会话侧栏运行状态点 / 横幅「被谁占用 · 前往停止」的数据源：活跃 run
/// （pending / running）即执行队列，前端据此给每个会话画运行状态
/// （执行中 / 排队第 N 位），并定位「哪些会话的 run 正在执行」以便横幅一键跳转。
/// 并发上限 > 1 时前 `max_concurrency` 个可能同时在跑，位置只表示创建顺序。
#[tauri::command]
#[specta::specta]
pub fn get_agent_queue(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<agent::AgentQueueItem>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::agent_queue(&conn).map_err(|e| e.to_string())
}

/// 查询会话文件的上下文水位（当前上下文词元/模型窗口 + 消息数 + 累计词元与成本）。
///
/// 「上下文水位可见性」的数据源：relwatch 侧不做会话长度治理（依赖 pi 自身管理），
/// 但应向用户暴露水位——`context_tokens / context_window` 与 pi footer 的
/// `5.2% / 1.0M` 同口径（见 agent_context 模块），累计字段回答「花了多少」。
#[tauri::command]
#[specta::specta]
pub fn get_agent_session_usage(
    session_key: String,
) -> Result<agent_session::AgentSessionUsage, String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let path = session_path_for_key(&session_key);
    if !path.exists() {
        return Ok(agent_session::AgentSessionUsage::empty());
    }
    agent_session::session_usage(&path).ok_or_else(|| "err.agent.read_session".to_string())
}

/// 读取会话的完整聊天消息流（pi 落盘的 JSONL，leaf 路径，时间正序）。
/// 会话文件不存在（新会话未提交）→ 空数组；写入中的半行容忍（下轮轮询补齐）。
///
/// 对位 run_id：把 user 消息按创建顺序直连到本会话的 run 记录，前端据此把失败
/// 备注 / 重试入口精确挂到对应气泡。
/// 注意「一次提交 = 一个 run + 一条 user 消息」并非恒成立——存在 run 不产生消息
/// 的路径（排队中被取消 / 派发前失败 / RPC 启动或 prompt 失败），纯顺序对位会把
/// 后续消息整体错位一位。因此对位带 started_at 邻近校验（60s 窗），把未产生
/// 消息的 run 跳过（排队取消的 run 无 started_at，天然被跳过）。
#[tauri::command]
#[specta::specta]
pub fn list_agent_messages(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<Vec<AgentChatMessage>, String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let path = session_path_for_key(&session_key);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut messages = agent_session::parse_session_file(&path)?;
    // 本会话 run 记录按创建顺序升序（db 层倒序，内存反转）
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let mut runs_desc = agent::list_run_summaries(&conn, &session_key, 1000)?;
    runs_desc.reverse();
    align_run_ids(&mut messages, &runs_desc);
    Ok(messages)
}

#[tauri::command]
#[specta::specta]
pub fn list_agent_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentSessionInfo>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let digests = agent::latest_run_digests(&conn)?;
    Ok(discover_disk_sessions(&agent_sessions_dir(), &digests))
}

/// 取消一次正在运行（或排队中）的 Agent 提交。
/// 「停止」= 向 RPC 常驻进程发 `abort`（不杀进程）：会话上下文保留在进程内存
/// 与 JSONL 文件，继续对话直接再提交即可，无需恢复流程。
/// 终态由调度器统一写入（cancelled），本命令不直接写库，避免竞态覆盖。
#[tauri::command]
#[specta::specta]
pub async fn cancel_agent_run(state: tauri::State<'_, AppState>, run_id: i64) -> Result<(), String> {
    cancel_run_inner(&state, run_id).await
}

/// 取消一次 run 的核心逻辑（供 cancel_agent_run 与 delete_agent_session 复用）。
async fn cancel_run_inner(state: &AppState, run_id: i64) -> Result<(), String> {
    // 仅当 run 仍处于 pending / running 时才取消：
    // 对已结束的 run 调用 abort 会误伤当前正在跑的另一 run，
    // 且 run_id 会滞留取消集合无人消费（无界增长）。
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let run = agent::get_run(&conn, run_id)?.ok_or_else(|| "err.agent.run_not_found".to_string())?;
    if run.status != "pending" && run.status != "running" {
        return Ok(());
    }
    let was_running = run.status == "running";
    // 必须先插取消标记再 abort：pi 的 abort 命令 = session.abort() → waitForIdle()，
    // 响应一定晚于 agent_end；dispatch 收到 agent_end(aborted) 即返回并同步消费标记。
    // 若先 abort 后插标记，dispatch 会在标记插入前完成 → run 误记 failed + aborted。
    state.agent_cancelled.lock().unwrap().insert(run_id);
    if was_running {
        // 仅 running（已拿到 semaphore、占用进程的当前 run）才需要 abort 打断生成；
        // pending（排队中）尚未占用进程，abort 是进程级的、会误伤当前正在跑的 run。
        // 插标记与 abort 之间 dispatch 可能已完成（run 结束、下一 run 开始），
        // 二次校验收窄该窗口：run 已落终态则跳过 abort 并移除滞留标记。
        let still_running = state
            .db
            .get()
            .ok()
            .and_then(|conn| agent::get_run(&conn, run_id).ok().flatten())
            .map(|r| r.status == "running")
            .unwrap_or(false);
        if still_running {
            // 按 run_id 精确定位承载进程后中止：abort 是进程级命令，并行下无参版本
            // 会打断**其他会话**正在跑的 run（用户点「停止」反而停了别人的任务）。
            state.agent_rpc.abort_run(run_id).await;
        }
    }
    // 二次校验（收窄 TOCTOU）：run 恰在插入标记前完成（dispatch 已落库终态并消费）
    // → 移除滞留标记，与 dispatch 出口的 clear_cancel_marker 呼应，防集合无界增长。
    if let Ok(conn) = state.db.get() {
        if let Ok(Some(r)) = agent::get_run(&conn, run_id) {
            if r.status != "pending" && r.status != "running" {
                state.agent_cancelled.lock().unwrap().remove(&run_id);
            }
        }
    }
    Ok(())
}

async fn cancel_runs_best_effort(state: &AppState, run_ids: Vec<i64>) -> Result<(), String> {
    for run_id in run_ids {
        if let Err(e) = cancel_run_inner(state, run_id).await {
            if e != "err.agent.run_not_found" {
                return Err(e);
            }
        }
    }
    Ok(())
}

/// 删除一个工作区会话：移除会话文件与全部运行记录。
///
/// 若该会话存在活跃 run（pending / running），先取消（停止）再删除：
/// 正在运行的 pi 进程会继续烧 token 直到自然结束或超时，产出写入已删除记录后
/// 静默丢弃——用户直觉是「删除=停止」，因此删除即停止，避免静默丢产出。
/// 前端在确认对话框中提示「正在运行，删除将同时停止」。
#[tauri::command]
#[specta::specta]
pub async fn delete_agent_session(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<(), String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let active_ids = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        agent::active_runs_for_session(&conn, &session_key)?
    };
    cancel_runs_best_effort(&state, active_ids).await?;
    remove_session_file_in(&agent_sessions_dir(), &session_key)
        .map_err(|e| format!("err.agent.delete_session|{}", e))?;
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::delete_runs_for_session(&conn, &session_key).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn prune_agent_sessions(
    state: tauri::State<'_, AppState>,
    session_keys: Vec<String>,
) -> Result<PruneAgentSessionsResult, String> {
    let (valid_keys, invalid_keys) = partition_session_keys(session_keys);
    if valid_keys.is_empty() {
        return Ok(PruneAgentSessionsResult {
            deleted: 0,
            failed: invalid_keys.len() as u32,
            failed_keys: invalid_keys,
        });
    }

    let active_run_ids = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        agent::batch_active_runs_for_sessions(&conn, &valid_keys)?
    };
    cancel_runs_best_effort(&state, active_run_ids).await?;

    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    prune_session_files_and_db(&mut conn, &agent_sessions_dir(), valid_keys, invalid_keys)
}

/// pi RPC 进程池的健康状态（工作区头部指示灯数据源）。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentRpcStatus {
    /// 池中是否有存活进程（`pi --mode rpc`）。
    pub running: bool,
    /// 常驻进程 pid（未运行时 None）；临时进程 pid 不在此暴露，总数见 process_count。
    pub pid: Option<u32>,
    /// 存活进程数（1 = 仅常驻；>1 = 有会话正在并行，各占一个 pi 进程）。
    pub process_count: u32,
    /// 进程级配置变更是否因「有 run 在跑」被推迟到当前任务结束后生效。
    pub restart_pending: bool,
}

/// 查询 pi RPC 进程池状态（健康指示 + 配置推迟提示）。
///
/// 惰性设计：不主动拉起进程——进程未启动属正常（首次提交时才 spawn），
/// 指示灯显示「未运行」即可，不该为了让灯变绿而白白起一个 node 进程。
#[tauri::command]
#[specta::specta]
pub async fn get_agent_rpc_status(state: tauri::State<'_, AppState>) -> Result<AgentRpcStatus, String> {
    Ok(AgentRpcStatus {
        running: state.agent_rpc.is_running().await,
        pid: state.agent_rpc.pid().await,
        process_count: state.agent_rpc.process_count().await as u32,
        restart_pending: state.agent_rpc.restart_pending(),
    })
}

/// 手动重启 pi RPC 进程池（改了 pi 路径/模型/skill 后想立刻生效，或怀疑进程卡死）。
///
/// 有 run 在跑时**拒绝**重启（kill 会以 `rpc_exited` 中断当前 run、记 failed，
/// 已产生的词元全部作废）；返回 false 表示被拒绝，前端据此提示「当前有任务在跑」。
/// 返回 true 表示已清空进程池（下次提交时惰性重建）。
#[tauri::command]
#[specta::specta]
pub async fn restart_agent_rpc(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.agent_rpc.restart_now().await)
}

/// 导出会话为 Markdown 或 JSON 文件（经保存对话框选择落盘位置）。
///
/// 两种格式的定位不同：
/// - `md`：给人读（复盘 / 贴进文档），保留工具调用与 bash 输出；
/// - `json`：给程序读（结构化消息数组，便于二次处理）。
///
/// 返回实际写入的路径；用户取消对话框时返回 `err.agent.export_cancelled`。
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
    let default_name = format!("relwatch-session-{}.{}", &session_key[..8.min(session_key.len())], if is_md { "md" } else { "json" });
    app.dialog()
        .file()
        .add_filter(if is_md { "Markdown" } else { "JSON" }, &[if is_md { "md" } else { "json" }])
        .set_file_name(default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let target = rx.await.ok().flatten().ok_or("err.agent.export_cancelled")?;
    let target = target
        .as_path()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "err.agent.export_path".to_string())?;

    let messages = agent_session::parse_session_file(&path)?;
    let usage = agent_session::session_usage(&path);
    let content = if is_md {
        agent_session::render_markdown(&title, &agent_session::export_messages(&messages), usage.as_ref())
    } else {
        serde_json::to_string_pretty(&agent_session::export_messages(&messages))
            .map_err(|e| format!("err.agent.export_serialize|{}", e))?
    };
    std::fs::write(&target, content).map_err(|e| format!("err.agent.export_write|{}", e))?;
    Ok(target)
}

#[tauri::command]
#[specta::specta]
pub fn get_agent_session_command(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<String, String> {
    let path = resolve_session_path(&state, run_id)?;
    let (binary, working_dir) = {
        let conn = state.db.get().map_err(|e| e.to_string())?;
        resolve_session_binary(&conn)?
    };
    Ok(format_session_command(&binary, &path, working_dir.as_deref()))
}

#[tauri::command]
#[specta::specta]
pub fn open_agent_session(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<(), String> {
    let path = resolve_session_path(&state, run_id)?;
    if !std::path::Path::new(&path).exists() {
        return Err("err.agent.session_missing".to_string());
    }
    let (binary, working_dir) = {
        let conn = state.db.get().map_err(|e| e.to_string())?;
        resolve_session_binary(&conn)?
    };
    spawn_terminal(&binary, &path, working_dir.as_deref())
}

fn resolve_session_path(state: &tauri::State<'_, AppState>, run_id: i64) -> Result<String, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let run = agent::get_run(&conn, run_id)?.ok_or_else(|| "err.agent.run_not_found".to_string())?;
    run.session_path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "err.agent.no_session".to_string())
}

/// 应用内更新安装前调用：优雅关闭 pi RPC 常驻进程（关 stdin → pi 自身清理其子进程）。
/// 与 `RunEvent::Exit` 的关闭语义一致——更新安装会硬杀主进程（Windows NSIS `exit(0)` /
/// Linux relaunch），不走 `RunEvent::Exit`，若不提前关闭 pi 会残留。
/// 仅 Linux/macOS 在 `relaunch()` 前调用本命令；Windows 上安装器接管退出，前端不会执行到。
#[tauri::command]
#[specta::specta]
pub async fn agent_shutdown_for_update(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.agent_rpc.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::MAX_ATTACHED_FILES;
    use crate::agent_session::session_key_from_path;

    /// 构造一条 run 摘要（测试对位用；started_at None = 排队取消/未启动）。
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
        // None / 空列表 → 空结果（无附件是常态，不是错误）
        assert!(validate_attached_files(None).unwrap().is_empty());
        assert!(validate_attached_files(Some(&vec![])).unwrap().is_empty());
        // 空白项与重复项被剔除
        let dup = vec!["  ".to_string(), out[0].clone(), out[0].clone()];
        assert_eq!(validate_attached_files(Some(&dup)).unwrap(), vec![out[0].clone()]);

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn validate_attached_files_rejects_bad_paths() {
        let dir = std::env::temp_dir().join(format!("relwatch-attach2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        // 不存在
        assert_eq!(
            validate_attached_files(Some(&vec![dir.join("nope.log").to_string_lossy().to_string()])),
            Err("err.agent.file_missing".to_string())
        );
        // 目录不是文件
        assert_eq!(
            validate_attached_files(Some(&vec![dir.to_string_lossy().to_string()])),
            Err("err.agent.file_not_file".to_string())
        );
        // 重复项先去重，上限必须在不重复样本上验证（重复 11 次同一路径只算 1 个）
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
            Err(e) => assert!(
                e.starts_with("err.agent.file_too_many|"),
                "应为数量超限，实际: {}",
                e
            ),
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
        // run2 排队中被取消（无 started_at，pi 从未写消息）：
        // run3 的消息不得错位挂到 run2（对位后仍指向 run3）
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
        // run2 派发前失败（有 started_at 但距下一条消息 >60s）：跳过
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
        // run 记录被清理（消息多于 run）：多余消息保持 run_id=None
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
        // 非会话文件（无 ws- 前缀 / 非 .jsonl / 空 key）一律忽略
        assert_eq!(session_key_from_path(std::path::Path::new("agent-sessions/evil.jsonl")), None);
        assert_eq!(session_key_from_path(std::path::Path::new("agent-sessions/ws-a.txt")), None);
        assert_eq!(session_key_from_path(std::path::Path::new("agent-sessions/ws-.jsonl")), None);
    }

    #[test]
    fn valid_session_key_accepts_uuid_like_keys() {
        assert!(is_valid_session_key("abc-123"));
        assert!(is_valid_session_key("f47ac10b-58cc-4372-a567-0e02b2c3d479"));
        assert!(is_valid_session_key("ws_1"));
    }

    #[test]
    fn valid_session_key_rejects_path_traversal() {
        // 路径穿越 / 非法字符一律拒绝
        assert!(!is_valid_session_key("..\\..\\evil"));
        assert!(!is_valid_session_key("../../evil"));
        assert!(!is_valid_session_key("a/b"));
        assert!(!is_valid_session_key("a\\b"));
        assert!(!is_valid_session_key("a b"));
        assert!(!is_valid_session_key(""));
        assert!(!is_valid_session_key("   "));
        // 超长拒绝
        assert!(!is_valid_session_key(&"x".repeat(129)));
        // 128 上限内通过
        assert!(is_valid_session_key(&"x".repeat(128)));
    }

    #[test]
    fn safe_session_path_rejects_escape_from_agent_dir() {
        // 正常路径（agent-sessions 目录内）通过
        let ok = agent_sessions_dir().join("ws-abc.jsonl");
        assert!(is_safe_session_path(&ok.to_string_lossy()));
        // 穿越出目录拒绝
        assert!(!is_safe_session_path("C:/Windows/evil.jsonl"));
        assert!(!is_safe_session_path("../evil.jsonl"));
    }

    #[test]
    fn prune_session_files_and_db_skips_invalid_keys_and_deletes_valid() {
        let mut conn = crate::db::init::init_memory_db().unwrap();
        let tmp_dir = std::env::temp_dir().join(format!("relwatch-prune-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let f1 = tmp_dir.join("ws-s1.jsonl");
        let f2 = tmp_dir.join("ws-s2.jsonl");
        let keep = tmp_dir.join("ws-keep.jsonl");
        std::fs::write(&f1, "{}\n").unwrap();
        std::fs::write(&f2, "{}\n").unwrap();
        std::fs::write(&keep, "{}\n").unwrap();

        for key in ["s1", "s2", "keep"] {
            agent::create_run(&conn, &NewRun {
                session_key: key,
                skill_path: None,
                entities: &[],
                instruction: "hi",
                model: None,
                session_path: None,
                files: None,
            })
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
