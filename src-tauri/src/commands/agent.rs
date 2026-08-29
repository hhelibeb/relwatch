//! Agent 工作区命令：全局配置读写、工作区提交运行、运行记录查询、会话恢复。
//!
//! 交互模型：用户在版本列表 / 监控源唤起工作区（右侧面板），通过拖拽 / `[[]]`
//! 引用实体（source / release），`@` 选择全局 skill，提交后后台运行 pi 无头进程；
//! 同一会话（session_key）的后续提交复用 pi 会话文件实现多轮继续。
//! 结束状态经 `AgentRunFinished` 事件推送前端刷新。

use crate::agent as agent_runner;
use crate::agent_session::{self, AgentChatMessage};
use crate::db::agent::{self, AgentConfig, AgentEntityRef, AgentModelRef, NewRun};
use crate::db::logs;
use crate::types::AppState;
use serde::{Deserialize, Serialize};
use specta::Type;

/// 工作区会话文件目录（RelWatch 数据目录下）。
fn agent_sessions_dir() -> std::path::PathBuf {
    crate::db::init::app_data_dir().join("agent-sessions")
}

/// 工作区会话文件路径：`agent-sessions/ws-<session_key>.jsonl`。
/// 同一会话的多次提交共享该文件，pi `--session <path>` 存在即继续。
fn session_path_for_key(session_key: &str) -> std::path::PathBuf {
    agent_sessions_dir().join(format!("ws-{}.jsonl", session_key))
}

/// 校验工作区会话 key：仅允许 ASCII 字母数字 / 短横线 / 下划线，长度 1..=128。
///
/// 会话 key 会被直接拼入会话文件路径（`agent-sessions/ws-<key>.jsonl`），
/// 若不限制字符集，`..` / 路径分隔符等可造成路径穿越（任意 .jsonl 文件
/// 删除 / 读取）。前端用 `crypto.randomUUID()`（UUID v4）天然满足白名单。
fn is_valid_session_key(key: &str) -> bool {
    let k = key.trim();
    !k.is_empty()
        && k.chars().count() <= 128
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 校验 DB 中固化的会话文件路径仍位于 agent-sessions 目录内
/// （防御历史脏数据 / DB 被篡改导致的路径穿越）。
/// 用 strip_prefix + 分隔符边界校验，避免 `agent-sessions2/evil` 这类前缀穿透。
fn is_safe_session_path(path: &str) -> bool {
    let dir = agent_sessions_dir();
    let norm = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
    let p = norm(std::path::Path::new(path));
    let d = norm(&dir);
    p.strip_prefix(&d)
        .map(|rest| rest.is_empty() || rest.starts_with('/'))
        .unwrap_or(false)
}

/// 读取全局 Agent 配置。
#[tauri::command]
#[specta::specta]
pub fn get_agent_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::load_agent_config(&conn)
}

/// 保存全局 Agent 配置。
/// 进程级字段（agent_type / binary / model / skills）变化时强杀常驻 RPC 进程：
/// spawn 只在启动时读一次这些字段，不重启则新配置静默不生效（新增 skill 后 @ 它
/// 会回到 /skill: 透传失效，改 model 会静默用旧模型）；下次提交 ensure_started 自动
/// 重启并恢复会话。timeout / prompt_suffix / enabled 每次调度重读，无需重启。
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
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let old = agent::load_agent_config(&conn)?;
    agent::save_agent_config(&conn, &config)?;
    logs::write_log_key(
        &conn,
        "INFO",
        "agent.config_saved",
        &serde_json::json!({"enabled": config.enabled, "skills": config.skills.len()}).to_string(),
    );
    if agent_process_level_changed(&old, &config) {
        // H-6 修复：不直接 kill_now——正在生成的 run 会被 rpc_exited 中断、记 failed
        // 且 token 作废。request_restart 带 running 守卫：空闲立即重启生效，
        // 有 running run 时延迟到当前 run 结束（dispatch_run 收尾 restart_if_pending）。
        log::info!("agent config process-level fields changed, requesting RPC process restart");
        state.agent_rpc.request_restart().await;
    }
    Ok(())
}

/// 进程级配置字段是否变化（变化需重启常驻进程生效）。
fn agent_process_level_changed(a: &AgentConfig, b: &AgentConfig) -> bool {
    a.agent_type != b.agent_type
        || a.binary != b.binary
        || a.model != b.model
        || a.working_dir != b.working_dir
        || a.skills != b.skills
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

// ---- 本地文件附件校验（评审「本地文件/图片附件」）----

/// 单次提交最多附加的本地文件数。
/// 每个文件都要进 prompt、并可能被 pi 逐个读取，放开数量会撑爆上下文；
/// 10 个足以覆盖「几个日志 + 几张截图」的真实场景。
const MAX_ATTACHED_FILES: usize = 10;
/// 单文件上限（字节，20 MB）。
/// 只传路径不读内容，故这个上限不是内存约束，而是**意图校验**：
/// 超大文件（视频 / 磁盘镜像）几乎不可能是「让 Agent 看一眼」的目标，
/// 多半是误选——提前挡下比让 pi 读一半再失败更省事。
const MAX_ATTACHED_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// 校验并归一化本地文件附件：去空、去重、校验存在性与大小、限制数量。
///
/// 与实体引用（`entities`）的区别：实体是应用内数据（source/release，按 id 查库），
/// 文件是用户本地磁盘路径。**只校验路径本身**，不读取内容——内容由 pi 自己的工具
/// 按需读取（见 `agent::append_local_files` 的 prompt 说明）。
fn validate_attached_files(files: Option<&Vec<String>>) -> Result<Vec<String>, String> {
    let Some(list) = files else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let p = raw.trim();
        if p.is_empty() {
            continue;
        }
        if out.iter().any(|x| x == p) {
            continue;
        }
        let meta = std::fs::metadata(p).map_err(|_| "err.agent.file_missing".to_string())?;
        if !meta.is_file() {
            return Err("err.agent.file_not_file".to_string());
        }
        if meta.len() > MAX_ATTACHED_FILE_BYTES {
            return Err("err.agent.file_too_large".to_string());
        }
        out.push(p.to_string());
        if out.len() > MAX_ATTACHED_FILES {
            return Err(format!("err.agent.file_too_many|{}", MAX_ATTACHED_FILES));
        }
    }
    Ok(out)
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
    // 实体去重 + 存在性校验（引用解析后 id 必须有效）
    let mut entities: Vec<AgentEntityRef> = Vec::new();
    let mut conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let config = agent::load_agent_config(&conn)?;
    if !config.enabled {
        return Err("err.agent.disabled".to_string());
    }
    for r in &input.entities {
        if entities.iter().any(|e| e.kind == r.kind && e.id == r.id) {
            continue;
        }
        let exists = match r.kind.as_str() {
            // 注意：get_source 返回 Result<Option<_>>，已删除的源是 Ok(None)，
            // 必须用 is_some() 判定存在（is_ok() 会把已删除实体误判为存在）
            "source" => crate::db::sources::get_source(&conn, r.id)?.is_some(),
            "release" => crate::db::releases::get_release(&conn, r.id)?.is_some(),
            _ => false,
        };
        if !exists {
            return Err("err.agent.entity_missing".to_string());
        }
        entities.push(r.clone());
    }
    // skill：指定时必须在全局列表（`@` 菜单数据源即全局列表）
    let skill_path = match input.skill_path.as_deref().map(|s| s.trim().to_string()) {
        Some(p) if !p.is_empty() => {
            if !config.skills.contains(&p) {
                return Err("err.agent.skill_not_configured".to_string());
            }
            Some(p)
        }
        _ => None,
    };

    // 本地文件附件：存在性 / 类型 / 大小 / 数量校验（提交前即拒绝，不占队列位）
    let files = validate_attached_files(input.files.as_ref())?;

    // 空提交判定以清洗后的输入为准（指令 / 实体 / 附件三者皆空才拒绝）：
    // 只附加文件、不写指令也算有效提交——「看看这个日志」的意图已由附件承载，
    // 前端空校验同口径放行（见 AgentWorkspace.handleSubmit），后端必须一致，
    // 否则附件-only 提交会在这里被 err.agent.empty_job 打回。
    if instruction.is_empty() && entities.is_empty() && files.is_empty() {
        return Err("err.agent.empty_job".to_string());
    }

    // 会话文件：历史提交已有则复用（pi --session 继续），否则新建。
    // 历史路径须通过目录前缀校验，防脏数据路径穿越。
    let session_path = match agent::get_session_path(&conn, &session_key)? {
        Some(p) if is_safe_session_path(&p) => Some(p),
        _ => {
            let path = session_path_for_key(&session_key);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            Some(path.to_string_lossy().to_string())
        }
    };

    let model_json: Option<String> = input
        .model
        .as_ref()
        .map(|m| serde_json::to_string(m).map_err(|e| e.to_string()))
        .transpose()?;

    // 提交前模型预检（评审 3.7）：显式选择的模型若不在 pi 当前可用列表里
    // （provider 未配置鉴权 / 在 pi 侧被 /scoped-models 禁用 / 进程重启后配置变了），
    // 直接拒绝——否则要等排队、启动、set_model、发 prompt 之后才以
    // err.agent.model_error 失败，用户白等一轮且拿到的还是泛化报错。
    // 「默认」（model = None）不预检：那是跟随进程现状，pi 侧自会兜底。
    if let Some(m) = &input.model {
        verify_model_available(&state, m).await?;
    }

    // 同会话守卫 + 建 run 放进 BEGIN IMMEDIATE 事务（评审 3.2 追加，P2）：
    // count 检查与 INSERT 之间若不加锁，两个并发提交可同时读到 0 再各自插入
    // （TOCTOU）——前端 submitting 锁挡住了 UI 途径，但 DevTools / 未来多客户端
    // 直接调命令仍可触发。BEGIN IMMEDIATE 立即拿写锁，并发请求串行化：
    // 先到者 check+insert+commit，后到者等锁后 count=1 被拒。
    // 前端在会话有活跃 run 时按钮即「停止」、Enter 被拒（canStop），
    // 该守卫把同一语义落到后端，正常 UI 流程下永不触发。
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if agent::active_run_count_for_session(&tx, &session_key)? > 0 {
        return Err("err.agent.session_busy".to_string());
    }
    let files_json: Option<String> = if files.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&files).map_err(|e| e.to_string())?)
    };
    let run_id = agent::create_run(&tx, &NewRun { session_key: &session_key, skill_path: skill_path.as_deref(), entities: &entities, instruction: &instruction, model: model_json.as_deref(), session_path: session_path.as_deref(), files: files_json.as_deref() })?;
    tx.commit().map_err(|e| e.to_string())?;

    let ctx = agent_runner::dispatch_ctx_from_app(&app);
    tauri::async_runtime::spawn(async move {
        agent_runner::dispatch_run(&ctx, run_id).await;
    });
    Ok(run_id)
}

/// 校验显式选择的模型在 pi 侧确实可用（提交前预检，评审 3.7）。
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
                if !state.agent_rpc.has_running_run().await {
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
/// 「排队中」提示的数据源：调度器全局单并发（Semaphore::new(1)），
/// 本会话 pending 说明有其他 run 占用执行位，前端据此提示「其他会话执行中」。
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
/// 会话侧栏运行状态点 / 横幅「被谁占用 · 前往停止」的数据源：调度器全局单并发，
/// 活跃 run 即执行队列，前端据此给每个会话画运行状态（执行中 / 排队第 N 位），
/// 并定位「哪个会话的 run 正在执行」以便横幅一键跳转。
#[tauri::command]
#[specta::specta]
pub fn get_agent_queue(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<agent::AgentQueueItem>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::agent_queue(&conn).map_err(|e| e.to_string())
}

/// 查询会话文件的上下文水位（消息条数 / 文本字符数 / 文件字节数）。
///
/// 「上下文水位可见性」的数据源：relwatch 侧不做会话长度治理（依赖 pi 自身管理），
/// 但应向用户暴露水位——接近上限时提示开新会话。token 为前端估算（字符数 ÷ 2）。
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
/// 备注 / 重试入口精确挂到对应气泡（替代 60 秒时间窗猜测）。
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

/// user 消息 ↔ run 的对位（started_at 邻近校验，60 秒窗）。
///
/// 逐条 user 消息按创建顺序找匹配 run：run 无 `started_at`（排队中被取消，未真正
/// 执行）或 `started_at` 与消息时间相差 >60s（派发前失败 / prompt 失败等未落盘
/// 消息的路径）→ 视为「该 run 未产生这条消息」，跳过；匹配到的写入 `msg.run_id`。
/// 无唯一标识可做绝对直连，此校验是启发式上限——不匹配置 None 落回前端
/// 时间窗兜底（前端 runForMessage 同带 60s 校验，双层拒绝错挂）。
fn align_run_ids(messages: &mut [AgentChatMessage], runs: &[agent::AgentRunSummary]) {
    let mut run_idx = 0usize;
    for m in messages.iter_mut() {
        if m.role != "user" {
            continue;
        }
        let msg_ms = rfc3339_to_millis(&m.timestamp).unwrap_or(0);
        while run_idx < runs.len() {
            let r = &runs[run_idx];
            let start_ms = r.started_at.as_deref().and_then(rfc3339_to_millis);
            let adjacent = start_ms.is_some_and(|s| (msg_ms - s).unsigned_abs() < 60_000);
            run_idx += 1;
            if adjacent {
                m.run_id = Some(r.id);
                break;
            }
            // 不邻近：该 run 未产生这条消息（继续找下一条 run）
        }
    }
}

/// 磁盘发现的一个工作区会话（会话索引丢失后的恢复数据源）。
///
/// 会话**文件**在 Roaming 数据目录（`agent-sessions/ws-<key>.jsonl`，与 database.db 同级），
/// 会话**索引**却在 WebView2 缓存目录树（localStorage）——任何磁盘清理工具扫到
/// `EBWebView` 就一锅端，文件毫发无损但 UI 里再也看不到它们。GUI 无 CLI 那样的
/// `ls | grep` 自救手段，故必须由后端提供发现能力：**文件即索引**。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentSessionInfo {
    /// 会话 key（与前端 session_key 同源，UUID）。
    pub session_key: String,
    /// 从会话文件首条 user 消息重建的标题（无 user 消息时为空串，前端用占位标题兜底）。
    pub title: String,
    /// 会话文件绝对路径。
    pub session_path: String,
    /// 最后活跃时间（RFC3339 UTC；文件 mtime 与最近一次提交时间的较晚者）。
    pub updated_at: String,
    /// 最近一次提交的状态（无 run 记录时为空串）。
    pub last_status: String,
    /// 该会话的累计提交次数。
    pub run_count: i64,
}

/// 扫描会话目录，列出磁盘上全部工作区会话（按最后活跃时间倒序）。
///
/// 前端用它与 localStorage 索引合并：索引里有的保持原样（用户改过的标题/模型优先），
/// 索引里没有的自动补入并标记为「恢复的会话」。会话目录不存在（从未提交过）→ 空列表。
#[tauri::command]
#[specta::specta]
pub fn list_agent_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentSessionInfo>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let digests = agent::latest_run_digests(&conn)?;
    let dir = agent_sessions_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // 目录不存在 = 从未提交过，非错误（与 list_agent_messages 的空数组语义一致）
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(session_key) = session_key_from_path(&path) else {
            continue;
        };
        if !is_valid_session_key(&session_key) {
            continue;
        }
        let file_ms = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let digest = digests.get(&session_key);
        let run_ms = digest
            .and_then(|d| d.last_created_at.as_deref())
            .and_then(rfc3339_to_millis)
            .unwrap_or(0);
        out.push(AgentSessionInfo {
            title: crate::agent_session::session_title_from_file(&path).unwrap_or_default(),
            session_key,
            session_path: path.to_string_lossy().to_string(),
            // 文件 mtime 与最近提交时间取较晚者：pi 落盘时机与 run 创建不完全同步，
            // 两者都可能更晚（提交后仍在流式写文件 / 记录补写）
            updated_at: millis_to_rfc3339(file_ms.max(run_ms)),
            last_status: digest
                .and_then(|d| d.last_status.clone())
                .unwrap_or_default(),
            run_count: digest.map(|d| d.run_count).unwrap_or(0),
        });
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

/// 从会话文件名反解 session_key（`ws-<key>.jsonl` → `<key>`）；非会话文件返回 None。
fn session_key_from_path(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let stem = name.strip_suffix(".jsonl")?;
    let key = stem.strip_prefix("ws-")?;
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

fn millis_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

fn rfc3339_to_millis(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
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
            state.agent_rpc.abort_force().await;
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
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    // 先取消该会话的全部活跃 run（若有）：删除 = 停止，防止 pi 继续烧 token
    // 产出写入已删除记录后静默丢弃。同一会话可排队多个 run（运行中仍可提交），
    // 仅取第一条会遗留 pending run：调度执行时重建已删的会话文件、向已删记录
    // 写终态（静默 no-op）、发事件——「删除=停止」承诺被打破，会话“复活”。
    //
    // 按状态直接查询（评审 3.9）：此前是「取最近 50 条摘要再筛状态」，理论上存在
    // 活跃 run 落到第 51 条之后被漏掉的窗口；这类窗口的概率再低，也不该由一个
    // 「漏一个就破坏承诺」的操作来承担，故换成无 LIMIT 的状态查询。
    for run_id in agent::active_runs_for_session(&conn, &session_key)? {
        cancel_run_inner(&state, run_id).await?;
    }
    let path = session_path_for_key(&session_key);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("err.agent.delete_session|{}", e))?;
    }
    agent::delete_runs_for_session(&conn, &session_key).map_err(|e| e.to_string())
}

/// pi 常驻进程的健康状态（工作区头部指示灯数据源）。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentRpcStatus {
    /// 进程是否存活（`pi --mode rpc` 常驻进程）。
    pub running: bool,
    /// 进程 pid（未运行时 None）。
    pub pid: Option<u32>,
    /// 进程级配置变更是否因「有 run 在跑」被推迟到当前任务结束后生效（评审 3.8）。
    pub restart_pending: bool,
}

/// 查询 pi 常驻进程状态（健康指示 + 配置推迟提示）。
///
/// 惰性设计：不主动拉起进程——进程未启动属正常（首次提交时才 spawn），
/// 指示灯显示「未运行」即可，不该为了让灯变绿而白白起一个 node 进程。
#[tauri::command]
#[specta::specta]
pub async fn get_agent_rpc_status(state: tauri::State<'_, AppState>) -> Result<AgentRpcStatus, String> {
    Ok(AgentRpcStatus {
        running: state.agent_rpc.is_running().await,
        pid: state.agent_rpc.pid().await,
        restart_pending: state.agent_rpc.restart_pending(),
    })
}

/// 手动重启 pi 常驻进程（改了 pi 路径/模型/skill 后想立刻生效，或怀疑进程卡死）。
///
/// 有正在执行的 run 时**拒绝**重启（kill 会以 `rpc_exited` 中断当前 run、记 failed，
/// 已产生的词元全部作废）；返回 false 表示被拒绝，前端据此提示「当前有任务在跑」。
/// 返回 true 表示已重启（下次提交时 ensure_started 惰性重新拉起）。
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

/// 返回在终端恢复该次会话的命令字符串（供复制）。
#[tauri::command]
#[specta::specta]
pub fn get_agent_session_command(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<String, String> {
    let path = resolve_session_path(&state, run_id)?;
    let (binary, working_dir) = {
        let conn = state.db.get().map_err(|e| e.to_string())?;
        let config = agent::load_agent_config(&conn)?;
        crate::agent_rpc::ensure_supported_type(&config)?;
        let binary = crate::agent_rpc::resolve_agent_binary(&config)?;
        (binary, config.working_dir)
    };
    // 带工作目录前缀：恢复的会话 cwd 与工作区内一致（pi 会话 cwd 固化在 JSONL 里，
    // 终端启动时先 cd 过去，后续 bash 工具行为才一致）。
    let wd = working_dir.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let base = format!("\"{}\" --session \"{}\"", binary, path);
    Ok(match wd {
        Some(w) if cfg!(windows) => format!("cd /d \"{}\" && {}", w, base),
        Some(w) => format!("cd \"{}\" && {}", w, base),
        None => base,
    })
}

/// 在独立终端窗口中打开该次运行的 pi 会话（`pi --session <path>`），恢复完整执行过程。
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
        let config = agent::load_agent_config(&conn)?;
        crate::agent_rpc::ensure_supported_type(&config)?;
        let binary = crate::agent_rpc::resolve_agent_binary(&config)?;
        (binary, config.working_dir)
    };
    spawn_terminal(&binary, &path, working_dir.as_deref())
}

/// 校验 run 存在且已落会话，返回会话文件路径。
fn resolve_session_path(state: &tauri::State<'_, AppState>, run_id: i64) -> Result<String, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let run = agent::get_run(&conn, run_id)?.ok_or_else(|| "err.agent.run_not_found".to_string())?;
    run.session_path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "err.agent.no_session".to_string())
}

/// 在新终端窗口中启动 `pi --session <path>`。
/// Windows 用 cmd start 开新控制台窗口（/D 指定起始目录）；Unix 探测常见终端模拟器。
/// working_dir：工作区配置的工作目录（None = 不指定，终端用默认 cwd）。
fn spawn_terminal(binary: &str, session_path: &str, working_dir: Option<&str>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg("start").arg("");
        // start 语法：START ["title"] [/D path] ...  —— /D 指定新窗口起始目录
        if let Some(wd) = working_dir {
            cmd.arg("/D").arg(wd);
        }
        cmd.arg(binary).arg("--session").arg(session_path);
        cmd.spawn().map_err(|e| format!("err.agent.spawn|{}", e))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // 探测常见终端模拟器（按优先级）；仅调用 Command 标准方法，无需 std CommandExt trait
        let terminals = [
            ("x-terminal-emulator", vec!["-e"]),
            ("gnome-terminal", vec!["--"]),
            ("konsole", vec!["-e"]),
            ("xfce4-terminal", vec!["-e"]),
        ];
        for (name, args) in terminals {
            if std::process::Command::new("which").arg(name).output().map(|o| o.status.success()).unwrap_or(false) {
                let mut cmd = std::process::Command::new(name);
                cmd.args(&args);
                cmd.arg(binary).arg("--session").arg(session_path);
                // 终端模拟器继承本进程 cwd，其启动的 pi/bash 子进程随之继承
                if let Some(wd) = working_dir {
                    cmd.current_dir(wd);
                }
                cmd.spawn().map_err(|e| format!("err.agent.spawn|{}", e))?;
                return Ok(());
            }
        }
        return Err("err.agent.no_terminal".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
