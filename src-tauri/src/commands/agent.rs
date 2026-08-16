//! Agent 工作区命令：全局配置读写、工作区提交运行、运行记录查询、会话恢复。
//!
//! 交互模型：用户在版本列表 / 监控源唤起工作区（右侧面板），通过拖拽 / `[[]]`
//! 引用实体（source / release），`@` 选择全局 skill，提交后后台运行 pi 无头进程；
//! 同一会话（session_key）的后续提交复用 pi 会话文件实现多轮继续。
//! 结束状态经 `AgentRunFinished` 事件推送前端刷新。

use crate::agent as agent_runner;
use crate::agent_session::{self, AgentChatMessage};
use crate::db::agent::{self, AgentConfig, AgentEntityRef};
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
#[tauri::command]
#[specta::specta]
pub fn save_agent_config(
    state: tauri::State<'_, AppState>,
    config: AgentConfig,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::save_agent_config(&conn, &config)?;
    logs::write_log_key(
        &conn,
        "INFO",
        "agent.config_saved",
        &serde_json::json!({"enabled": config.enabled, "skills": config.skills.len()}).to_string(),
    );
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
    if instruction.is_empty() && input.entities.is_empty() {
        return Err("err.agent.empty_job".to_string());
    }
    // 实体去重 + 存在性校验（引用解析后 id 必须有效）
    let mut entities: Vec<AgentEntityRef> = Vec::new();
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
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

    let run_id = agent::create_run(
        &conn,
        &session_key,
        skill_path.as_deref(),
        &entities,
        &instruction,
        session_path.as_deref(),
    )?;

    let ctx = agent_runner::dispatch_ctx_from_app(&app);
    tauri::async_runtime::spawn(async move {
        agent_runner::dispatch_run(&ctx, run_id).await;
    });
    Ok(run_id)
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

/// 读取会话的完整聊天消息流（pi 落盘的 JSONL，leaf 路径，时间正序）。
/// 会话文件不存在（新会话未提交）→ 空数组；写入中的半行容忍（下轮轮询补齐）。
#[tauri::command]
#[specta::specta]
pub fn list_agent_messages(session_key: String) -> Result<Vec<AgentChatMessage>, String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let path = session_path_for_key(&session_key);
    if !path.exists() {
        return Ok(Vec::new());
    }
    agent_session::parse_session_file(&path)
}

/// 取消一次正在运行（或排队中）的 Agent 提交。
/// 「停止」= 向 RPC 常驻进程发 `abort`（不杀进程）：会话上下文保留在进程内存
/// 与 JSONL 文件，继续对话直接再提交即可，无需恢复流程。
/// 终态由调度器统一写入（cancelled），本命令不直接写库，避免竞态覆盖。
#[tauri::command]
#[specta::specta]
pub async fn cancel_agent_run(state: tauri::State<'_, AppState>, run_id: i64) -> Result<(), String> {
    // 仅当 run 仍处于 pending / running 时才取消：
    // 对已结束的 run 调用 abort 会误伤当前正在跑的另一 run，
    // 且 run_id 会滞留取消集合无人消费（无界增长）。
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    let run = agent::get_run(&conn, run_id)?.ok_or_else(|| "err.agent.run_not_found".to_string())?;
    if run.status != "pending" && run.status != "running" {
        return Ok(());
    }
    state.agent_rpc.abort_force().await;
    state.agent_cancelled.lock().unwrap().insert(run_id);
    Ok(())
}

/// 删除一个工作区会话：移除会话文件与全部运行记录。
#[tauri::command]
#[specta::specta]
pub fn delete_agent_session(
    state: tauri::State<'_, AppState>,
    session_key: String,
) -> Result<(), String> {
    if !is_valid_session_key(&session_key) {
        return Err("err.agent.invalid_session".to_string());
    }
    let path = session_path_for_key(&session_key);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("err.agent.delete_session|{}", e))?;
    }
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    agent::delete_runs_for_session(&conn, &session_key).map_err(|e| e.to_string())
}

/// 返回在终端恢复该次会话的命令字符串（供复制）。
#[tauri::command]
#[specta::specta]
pub fn get_agent_session_command(
    state: tauri::State<'_, AppState>,
    run_id: i64,
) -> Result<String, String> {
    let path = resolve_session_path(&state, run_id)?;
    let binary = {
        let conn = state.db.get().map_err(|e| e.to_string())?;
        let config = agent::load_agent_config(&conn)?;
        crate::agent_rpc::ensure_supported_type(&config)?;
        crate::agent_rpc::resolve_agent_binary(&config)?
    };
    Ok(format!("\"{}\" --session \"{}\"", binary, path))
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
    let binary = {
        let conn = state.db.get().map_err(|e| e.to_string())?;
        let config = agent::load_agent_config(&conn)?;
        crate::agent_rpc::ensure_supported_type(&config)?;
        crate::agent_rpc::resolve_agent_binary(&config)?
    };
    spawn_terminal(&binary, &path)
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
/// Windows 用 cmd start 开新控制台窗口；Unix 探测常见终端模拟器。
fn spawn_terminal(binary: &str, session_path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "start", "", binary, "--session", session_path])
            .spawn()
            .map_err(|e| format!("err.agent.spawn|{}", e))?;
        let _ = status;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        // 探测常见终端模拟器（按优先级）
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

    #[test]
    fn session_path_uses_sanitized_key() {
        let p = session_path_for_key("abc-123");
        assert!(p.ends_with("ws-abc-123.jsonl"));
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
