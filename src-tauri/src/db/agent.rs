//! Agent 工作区：全局配置（app_settings key）+ 会话提交记录（agent_runs）。
//!
//! 数据模型（全局化设计）：
//! - Agent 配置为**全局单例**，存 app_settings 键值（不走 SETTING_SPECS 注册表，
//!   因其含 JSON 数组字段，与注册表的标量序列化规则不符）：
//!   `agent_enabled` / `agent_type` / `agent_binary` / `agent_model` /
//!   `agent_prompt_suffix` / `agent_timeout_seconds` / `agent_skills`（JSON 数组）。
//! - `agent_runs`：一次工作区提交的运行记录。同一 `session_key`（前端 UUID）的多次
//!   提交共享一个 pi 会话文件（`pi --session <path>` 继续），构成多轮对话；
//!   `entities` 固化本次提交引用的实体（JSON: `[{"kind":"source"|"release","id":N}]`）。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::settings::{
    get_setting_bool, get_setting_i64, get_setting_str, set_setting,
    KEY_AGENT_BINARY, KEY_AGENT_ENABLED, KEY_AGENT_MODEL, KEY_AGENT_PROMPT_SUFFIX,
    KEY_AGENT_SKILLS, KEY_AGENT_TIMEOUT_SECONDS, KEY_AGENT_TYPE, KEY_AGENT_WS_WIDTH,
};

/// 全局 Agent 配置（设置页「Agent」分区读写）。
#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct AgentConfig {
    /// 总开关：关闭时唤起按钮隐藏，运行命令拒绝执行。
    pub enabled: bool,
    /// Agent 类型（"pi" = 本地 pi CLI；新类型在 agent::executor_for 登记）。
    pub agent_type: String,
    /// Agent 可执行文件显式路径（None = 按类型自动探测，如 where/which pi）。
    pub binary: Option<String>,
    /// 模型（None = 该 Agent 默认模型）。
    pub model: Option<String>,
    /// 追加在每次提交 prompt 末尾的固定后缀（如"请输出中文"）。
    pub prompt_suffix: Option<String>,
    /// 子进程超时秒数（超时 kill）。
    pub timeout_seconds: i64,
    /// 全局 skill 备选列表（`@` 菜单数据源；运行前校验存在性）。
    pub skills: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            enabled: false,
            agent_type: "pi".to_string(),
            binary: None,
            model: None,
            prompt_suffix: None,
            timeout_seconds: 300,
            skills: Vec::new(),
        }
    }
}

/// 一次工作区提交引用的实体（拖拽 / `[[]]` 引用统一入口）。
#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct AgentEntityRef {
    /// "source" | "release"
    pub kind: String,
    pub id: i64,
}

/// 一次提交显式选择的 pi 模型引用（provider + modelId，`set_model` 精确匹配用）。
/// `agent_runs.model` 列以 JSON 存储；None = 跟随 pi 当前/默认模型（不发送 set_model）。
#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct AgentModelRef {
    /// pi 模型提供方（如 "anthropic" / "opencode-go"）。
    pub provider: String,
    /// 模型 id（如 "deepseek-v4-flash"；注意 id 可能自带 provider 前缀如 "cline-pass/..."，
    /// 因此必须拆 provider/modelId 两字段，不能用 `provider/id` 拼接做键）。
    pub model_id: String,
}

/// 读取全局 Agent 配置（缺省回退默认值）。
pub fn load_agent_config(conn: &Connection) -> Result<AgentConfig, String> {
    let skills_raw = get_setting_str(conn, KEY_AGENT_SKILLS, "[]")?;
    let skills = serde_json::from_str::<Vec<String>>(&skills_raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
    Ok(AgentConfig {
        enabled: get_setting_bool(conn, KEY_AGENT_ENABLED, false)?,
        agent_type: {
            let t = get_setting_str(conn, KEY_AGENT_TYPE, "pi")?;
            if t.trim().is_empty() { "pi".to_string() } else { t }
        },
        binary: non_empty(get_setting_str(conn, KEY_AGENT_BINARY, "")?),
        model: non_empty(get_setting_str(conn, KEY_AGENT_MODEL, "")?),
        prompt_suffix: non_empty(get_setting_str(conn, KEY_AGENT_PROMPT_SUFFIX, "")?),
        timeout_seconds: get_setting_i64(conn, KEY_AGENT_TIMEOUT_SECONDS, 300)?.max(1),
        skills,
    })
}

/// 保存全局 Agent 配置（skills 去重、trim；空串可选字段归一为 None）。
/// agent_type 必须为受支持类型（与 agent::executor_for 登记集合同步），
/// 保存时即拒绝未知类型，避免运行时才报错。
pub fn save_agent_config(conn: &Connection, cfg: &AgentConfig) -> Result<(), String> {
    match cfg.agent_type.as_str() {
        "pi" => {}
        other => return Err(format!("err.agent.unsupported_type|{}", other)),
    }
    let mut skills: Vec<String> = Vec::new();
    for s in &cfg.skills {
        let t = s.trim();
        if !t.is_empty() && !skills.contains(&t.to_string()) {
            skills.push(t.to_string());
        }
    }
    let skills_json = serde_json::to_string(&skills).map_err(|e| e.to_string())?;
    set_setting(conn, KEY_AGENT_ENABLED, &cfg.enabled.to_string())?;
    set_setting(conn, KEY_AGENT_TYPE, &cfg.agent_type)?;
    set_setting(conn, KEY_AGENT_BINARY, cfg.binary.as_deref().unwrap_or(""))?;
    set_setting(conn, KEY_AGENT_MODEL, cfg.model.as_deref().unwrap_or(""))?;
    set_setting(conn, KEY_AGENT_PROMPT_SUFFIX, cfg.prompt_suffix.as_deref().unwrap_or(""))?;
    set_setting(conn, KEY_AGENT_TIMEOUT_SECONDS, &cfg.timeout_seconds.max(1).to_string())?;
    set_setting(conn, KEY_AGENT_SKILLS, &skills_json)?;
    Ok(())
}

/// 读取 Agent 工作区面板宽度（逻辑 px；未设置返回 0，前端回退默认 440）。
pub fn load_agent_ws_width(conn: &Connection) -> Result<i64, String> {
    get_setting_i64(conn, KEY_AGENT_WS_WIDTH, 0)
}

/// 保存 Agent 工作区面板宽度（前端拖窗口右边框后写入）。
pub fn save_agent_ws_width(conn: &Connection, width: i64) -> Result<(), String> {
    set_setting(conn, KEY_AGENT_WS_WIDTH, &width.max(1).to_string())
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() { None } else { Some(t) }
}

/// 一次工作区提交的运行记录。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentRun {
    pub id: i64,
    /// 工作区会话标识（前端 UUID）；同一会话多次提交共享 pi 会话文件实现多轮继续。
    pub session_key: String,
    /// 本次提交使用的 skill 路径（未选为 None）。
    pub skill_path: Option<String>,
    /// 本次提交引用的实体（JSON 数组，`[{"kind":"source","id":1}]`）。
    pub entities: String,
    /// 用户输入文本（`[[]]` 引用已解析剥离，实体归入 entities 列）。
    pub instruction: String,
    /// 本次提交显式选择的模型（JSON `{"provider":..,"model_id":..}`；None = 跟随 pi 默认）。
    pub model: Option<String>,
    /// 本次提交落盘的 pi 会话文件（`pi --session <path>` 恢复/继续）。
    pub session_path: Option<String>,
    /// pending | running | success | failed | timeout。
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    /// 启动失败等非进程类错误信息。
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

/// 创建一次提交（初始状态 pending，等待调度器执行）。
pub fn create_run(
    conn: &Connection,
    session_key: &str,
    skill_path: Option<&str>,
    entities: &[AgentEntityRef],
    instruction: &str,
    model: Option<&str>,
    session_path: Option<&str>,
) -> Result<i64, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let entities_json = serde_json::to_string(entities).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO agent_runs (session_key, skill_path, entities, instruction, model, session_path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
        params![session_key, skill_path, entities_json, instruction, model, session_path, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// 运行开始：pending → running，记录 started_at。
pub fn mark_run_started(conn: &Connection, run_id: i64) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET status = 'running', started_at = ?2 WHERE id = ?1",
        params![run_id, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 固化本次提交的会话文件路径（create_run 后调用）。
pub fn set_run_session_path(conn: &Connection, run_id: i64, session_path: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE agent_runs SET session_path = ?2 WHERE id = ?1",
        params![run_id, session_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 运行结束：写终态与输出。
pub fn finish_run(
    conn: &Connection,
    run_id: i64,
    status: &str,
    exit_code: Option<i64>,
    stdout: Option<&str>,
    stderr: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs
         SET status = ?2, exit_code = ?3, stdout = ?4, stderr = ?5, error = ?6, finished_at = ?7
         WHERE id = ?1",
        params![run_id, status, exit_code, stdout, stderr, error, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取运行记录。
pub fn get_run(conn: &Connection, run_id: i64) -> Result<Option<AgentRun>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_key, skill_path, entities, instruction, model, session_path, status, exit_code,
                    stdout, stderr, error, started_at, finished_at, created_at
             FROM agent_runs WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map(params![run_id], run_from_row).map_err(|e| e.to_string())?;
    match rows.next() {
        Some(Ok(r)) => Ok(Some(r)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

/// 一次工作区提交的列表摘要（不含 stdout/stderr 大字段，供会话记录列表）。
/// stdout 存的是模型完整输出，列表接口最多拉 100 条，全列返回会拖慢查询与序列化。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentRunSummary {
    pub id: i64,
    pub session_key: String,
    pub skill_path: Option<String>,
    pub entities: String,
    pub instruction: String,
    pub model: Option<String>,
    pub session_path: Option<String>,
    pub status: String,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

/// 按工作区会话列出提交记录摘要（倒序，不含 stdout/stderr 大字段）。
pub fn list_run_summaries(
    conn: &Connection,
    session_key: &str,
    limit: i64,
) -> Result<Vec<AgentRunSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_key, skill_path, entities, instruction, model, session_path, status, exit_code,
                    error, started_at, finished_at, created_at
             FROM agent_runs WHERE session_key = ?1
             ORDER BY id DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_key, limit], |row| {
            Ok(AgentRunSummary {
                id: row.get(0)?,
                session_key: row.get(1)?,
                skill_path: row.get(2)?,
                entities: row.get(3)?,
                instruction: row.get(4)?,
                model: row.get(5)?,
                session_path: row.get(6)?,
                status: row.get(7)?,
                exit_code: row.get(8)?,
                error: row.get(9)?,
                started_at: row.get(10)?,
                finished_at: row.get(11)?,
                created_at: row.get(12)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 按工作区会话列出提交记录（倒序）。
pub fn list_runs(conn: &Connection, session_key: &str, limit: i64) -> Result<Vec<AgentRun>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_key, skill_path, entities, instruction, model, session_path, status, exit_code,
                    stdout, stderr, error, started_at, finished_at, created_at
             FROM agent_runs WHERE session_key = ?1
             ORDER BY id DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_key, limit], run_from_row)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 全局 Agent 队列状态（「排队中」提示的数据源）。
#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct AgentQueueStatus {
    /// 本会话最新 pending run 的全局队列位置（1 = 下一个执行；无 pending → None）。
    pub position: Option<i64>,
    /// 是否存在其他会话的 running run（执行位被占用）。
    pub other_running: bool,
    /// 其他会话 running run 的 session_key（前端可映射为会话标题）。
    pub running_sessions: Vec<String>,
}

/// 查询全局队列状态：本会话最新 pending run 的队列位置 + 其他会话占用情况。
///
/// 调度器为全局单并发（Semaphore::new(1)），pending run 按创建顺序排队；
/// 队列位置 = 全局 status ∈ {pending, running} 且 id 更小的 run 数 + 1。
pub fn agent_queue_status(conn: &Connection, session_key: &str) -> Result<AgentQueueStatus, String> {
    let latest_pending: Option<i64> = conn
        .query_row(
            "SELECT id FROM agent_runs WHERE session_key = ?1 AND status = 'pending' ORDER BY id DESC LIMIT 1",
            params![session_key],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
        .ok();
    let position = match latest_pending {
        Some(run_id) => {
            let ahead: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_runs WHERE status IN ('pending','running') AND id < ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Some(ahead + 1)
        }
        None => None,
    };
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT session_key FROM agent_runs WHERE status = 'running' AND session_key != ?1",
        )
        .map_err(|e| e.to_string())?;
    let running_sessions: Vec<String> = stmt
        .query_map(params![session_key], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(AgentQueueStatus {
        position,
        other_running: !running_sessions.is_empty(),
        running_sessions,
    })
}

/// 删除某会话的全部运行记录（会话文件删除由命令层负责）。
pub fn delete_runs_for_session(conn: &Connection, session_key: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM agent_runs WHERE session_key = ?1",
        params![session_key],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 工作区会话的最近一次会话文件路径（多轮继续：None = 新会话）。
pub fn get_session_path(conn: &Connection, session_key: &str) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT session_path FROM agent_runs
             WHERE session_key = ?1 AND session_path IS NOT NULL AND session_path != ''
             ORDER BY id DESC LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map(params![session_key], |row| row.get::<_, Option<String>>(0)).map_err(|e| e.to_string())?;
    match rows.next() {
        Some(Ok(p)) => Ok(p),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

fn run_from_row(row: &rusqlite::Row) -> rusqlite::Result<AgentRun> {
    Ok(AgentRun {
        id: row.get(0)?,
        session_key: row.get(1)?,
        skill_path: row.get(2)?,
        entities: row.get(3)?,
        instruction: row.get(4)?,
        model: row.get(5)?,
        session_path: row.get(6)?,
        status: row.get(7)?,
        exit_code: row.get(8)?,
        stdout: row.get(9)?,
        stderr: row.get(10)?,
        error: row.get(11)?,
        started_at: row.get(12)?,
        finished_at: row.get(13)?,
        created_at: row.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn config_round_trip_and_defaults() {
        let conn = init_memory_db().unwrap();
        // 默认值
        let cfg = load_agent_config(&conn).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.agent_type, "pi");
        assert!(cfg.binary.is_none());
        assert_eq!(cfg.timeout_seconds, 300);
        assert!(cfg.skills.is_empty());

        // 保存（含重复/空 skill 归一化）
        save_agent_config(
            &conn,
            &AgentConfig {
                enabled: true,
                agent_type: "pi".into(),
                binary: Some("C:/pi.cmd".into()),
                model: Some("m1".into()),
                prompt_suffix: Some("请输出中文".into()),
                timeout_seconds: 120,
                skills: vec!["/s1".into(), "/s1".into(), "  /s2  ".into(), "".into()],
            },
        )
        .unwrap();

        let loaded = load_agent_config(&conn).unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.agent_type, "pi");
        assert_eq!(loaded.binary.as_deref(), Some("C:/pi.cmd"));
        assert_eq!(loaded.skills, vec!["/s1", "/s2"]);
        assert_eq!(loaded.timeout_seconds, 120);
    }

    #[test]
    fn run_lifecycle_pending_running_finished() {
        let conn = init_memory_db().unwrap();
        let entities = vec![
            AgentEntityRef { kind: "source".into(), id: 1 },
            AgentEntityRef { kind: "release".into(), id: 42 },
        ];
        let run_id = create_run(&conn, "ws-abc", Some("/tmp/my-skill"), &entities, "帮我总结", None, None)
            .unwrap();

        let run = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(run.status, "pending");
        assert_eq!(run.session_key, "ws-abc");
        assert_eq!(run.skill_path.as_deref(), Some("/tmp/my-skill"));
        assert_eq!(run.instruction, "帮我总结");
        let parsed: Vec<AgentEntityRef> = serde_json::from_str(&run.entities).unwrap();
        assert_eq!(parsed, entities);

        mark_run_started(&conn, run_id).unwrap();
        assert_eq!(get_run(&conn, run_id).unwrap().unwrap().status, "running");

        set_run_session_path(&conn, run_id, "C:/data/agent-sessions/ws-abc.jsonl").unwrap();
        finish_run(&conn, run_id, "success", Some(0), Some("out"), Some("err"), None).unwrap();
        let done = get_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(done.status, "success");
        assert_eq!(done.exit_code, Some(0));
        assert_eq!(done.session_path.as_deref(), Some("C:/data/agent-sessions/ws-abc.jsonl"));
        assert!(done.finished_at.is_some());

        // 同一会话第二次提交 → 倒序列表 + 会话路径可继续
        let run2 = create_run(&conn, "ws-abc", None, &[], "继续", Some("C:/data/agent-sessions/ws-abc.jsonl"), None)
            .unwrap();
        let runs = list_runs(&conn, "ws-abc", 10).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, run2);
        assert_eq!(runs[1].id, run_id);
        assert_eq!(list_runs(&conn, "ws-abc", 1).unwrap().len(), 1);
        // 其他会话不可见
        assert!(list_runs(&conn, "ws-other", 10).unwrap().is_empty());

        // 会话路径：取最近一次
        assert_eq!(
            get_session_path(&conn, "ws-abc").unwrap().as_deref(),
            Some("C:/data/agent-sessions/ws-abc.jsonl")
        );
        assert!(get_session_path(&conn, "ws-none").unwrap().is_none());
    }

    #[test]
    fn queue_status_reports_position_and_other_running() {
        let conn = init_memory_db().unwrap();
        // 空：无 pending
        let st = agent_queue_status(&conn, "ws-a").unwrap();
        assert_eq!(st.position, None);
        assert!(!st.other_running);
        assert!(st.running_sessions.is_empty());

        // 会话 A 先建 run（running），会话 B 建 pending → B 队列位置 2、other_running
        let ra = create_run(&conn, "ws-a", None, &[], "任务A", None, None).unwrap();
        mark_run_started(&conn, ra).unwrap();
        let rb = create_run(&conn, "ws-b", None, &[], "任务B", None, None).unwrap();
        let st = agent_queue_status(&conn, "ws-b").unwrap();
        assert_eq!(st.position, Some(2));
        assert!(st.other_running);
        assert_eq!(st.running_sessions, vec!["ws-a"]);

        // A 自己视角：无其他会话 running
        let st = agent_queue_status(&conn, "ws-a").unwrap();
        assert!(!st.other_running);
        assert!(st.running_sessions.is_empty());

        // 第三个会话 pending：位置 3
        let _rc = create_run(&conn, "ws-c", None, &[], "任务C", None, None).unwrap();
        let st = agent_queue_status(&conn, "ws-c").unwrap();
        assert_eq!(st.position, Some(3));

        // A 结束、B 开始执行 → B 无 pending（position None）；C 位置 2、other_running
        finish_run(&conn, ra, "success", Some(0), Some(""), None, None).unwrap();
        mark_run_started(&conn, rb).unwrap();
        let st = agent_queue_status(&conn, "ws-b").unwrap();
        assert_eq!(st.position, None);
        assert!(!st.other_running);
        let st = agent_queue_status(&conn, "ws-c").unwrap();
        assert_eq!(st.position, Some(2));
        assert!(st.other_running);
        assert_eq!(st.running_sessions, vec!["ws-b"]);

        // 本会话自己的 pending 不算 other_running
        let st = agent_queue_status(&conn, "ws-c").unwrap();
        assert_eq!(st.running_sessions, vec!["ws-b"]);
        // 无 pending 的会话：position None
        let st = agent_queue_status(&conn, "ws-none").unwrap();
        assert_eq!(st.position, None);
    }

    #[test]
    fn run_serializes_for_specta() {
        let conn = init_memory_db().unwrap();
        let run_id = create_run(&conn, "ws-x", Some("/s"), &[], "指令", None, None).unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        let v = serde_json::to_value(&run).unwrap();
        assert_eq!(v["status"], "pending");
        assert_eq!(v["session_key"], "ws-x");
        assert_eq!(v["instruction"], "指令");
    }
}
