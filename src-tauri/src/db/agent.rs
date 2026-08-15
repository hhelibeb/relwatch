//! Agent 工作区：全局配置（app_settings key）+ 会话提交记录（agent_runs）。
//!
//! 数据模型（全局化设计）：
//! - Agent 配置为**全局单例**，存 app_settings 键值（不走 SETTING_SPECS 注册表，
//!   因其含 JSON 数组字段，与注册表的标量序列化规则不符）：
//!   `agent_enabled` / `agent_pi_binary` / `agent_pi_model` / `agent_prompt_suffix` /
//!   `agent_timeout_seconds` / `agent_skills`（JSON 数组）。
//! - `agent_runs`：一次工作区提交的运行记录。同一 `session_key`（前端 UUID）的多次
//!   提交共享一个 pi 会话文件（`pi --session <path>` 继续），构成多轮对话；
//!   `entities` 固化本次提交引用的实体（JSON: `[{"kind":"source"|"release","id":N}]`）。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::settings::{
    get_setting_bool, get_setting_i64, get_setting_str, set_setting,
    KEY_AGENT_ENABLED, KEY_AGENT_PI_BINARY, KEY_AGENT_PI_MODEL, KEY_AGENT_PROMPT_SUFFIX,
    KEY_AGENT_SKILLS, KEY_AGENT_TIMEOUT_SECONDS,
};

/// 全局 Agent 配置（设置页「AI → Agent」分区读写）。
#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct AgentConfig {
    /// 总开关：关闭时唤起按钮隐藏，运行命令拒绝执行。
    pub enabled: bool,
    /// pi 可执行文件显式路径（None = 自动探测 where/which pi）。
    pub pi_binary: Option<String>,
    /// pi 模型（None = pi 默认模型）。
    pub pi_model: Option<String>,
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
            pi_binary: None,
            pi_model: None,
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
        pi_binary: non_empty(get_setting_str(conn, KEY_AGENT_PI_BINARY, "")?),
        pi_model: non_empty(get_setting_str(conn, KEY_AGENT_PI_MODEL, "")?),
        prompt_suffix: non_empty(get_setting_str(conn, KEY_AGENT_PROMPT_SUFFIX, "")?),
        timeout_seconds: get_setting_i64(conn, KEY_AGENT_TIMEOUT_SECONDS, 300)?.max(1),
        skills,
    })
}

/// 保存全局 Agent 配置（skills 去重、trim；空串可选字段归一为 None）。
pub fn save_agent_config(conn: &Connection, cfg: &AgentConfig) -> Result<(), String> {
    let mut skills: Vec<String> = Vec::new();
    for s in &cfg.skills {
        let t = s.trim();
        if !t.is_empty() && !skills.contains(&t.to_string()) {
            skills.push(t.to_string());
        }
    }
    let skills_json = serde_json::to_string(&skills).map_err(|e| e.to_string())?;
    set_setting(conn, KEY_AGENT_ENABLED, &cfg.enabled.to_string())?;
    set_setting(conn, KEY_AGENT_PI_BINARY, cfg.pi_binary.as_deref().unwrap_or(""))?;
    set_setting(conn, KEY_AGENT_PI_MODEL, cfg.pi_model.as_deref().unwrap_or(""))?;
    set_setting(conn, KEY_AGENT_PROMPT_SUFFIX, cfg.prompt_suffix.as_deref().unwrap_or(""))?;
    set_setting(conn, KEY_AGENT_TIMEOUT_SECONDS, &cfg.timeout_seconds.max(1).to_string())?;
    set_setting(conn, KEY_AGENT_SKILLS, &skills_json)?;
    Ok(())
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
    session_path: Option<&str>,
) -> Result<i64, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let entities_json = serde_json::to_string(entities).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO agent_runs (session_key, skill_path, entities, instruction, session_path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
        params![session_key, skill_path, entities_json, instruction, session_path, now],
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
            "SELECT id, session_key, skill_path, entities, instruction, session_path, status, exit_code,
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

/// 按工作区会话列出提交记录（倒序）。
pub fn list_runs(conn: &Connection, session_key: &str, limit: i64) -> Result<Vec<AgentRun>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_key, skill_path, entities, instruction, session_path, status, exit_code,
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
        session_path: row.get(5)?,
        status: row.get(6)?,
        exit_code: row.get(7)?,
        stdout: row.get(8)?,
        stderr: row.get(9)?,
        error: row.get(10)?,
        started_at: row.get(11)?,
        finished_at: row.get(12)?,
        created_at: row.get(13)?,
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
        assert!(cfg.pi_binary.is_none());
        assert_eq!(cfg.timeout_seconds, 300);
        assert!(cfg.skills.is_empty());

        // 保存（含重复/空 skill 归一化）
        save_agent_config(
            &conn,
            &AgentConfig {
                enabled: true,
                pi_binary: Some("C:/pi.cmd".into()),
                pi_model: Some("m1".into()),
                prompt_suffix: Some("请输出中文".into()),
                timeout_seconds: 120,
                skills: vec!["/s1".into(), "/s1".into(), "  /s2  ".into(), "".into()],
            },
        )
        .unwrap();

        let loaded = load_agent_config(&conn).unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.pi_binary.as_deref(), Some("C:/pi.cmd"));
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
        let run_id = create_run(&conn, "ws-abc", Some("/tmp/my-skill"), &entities, "帮我总结", None)
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
        let run2 = create_run(&conn, "ws-abc", None, &[], "继续", Some("C:/data/agent-sessions/ws-abc.jsonl"))
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
    fn run_serializes_for_specta() {
        let conn = init_memory_db().unwrap();
        let run_id = create_run(&conn, "ws-x", Some("/s"), &[], "指令", None).unwrap();
        let run = get_run(&conn, run_id).unwrap().unwrap();
        let v = serde_json::to_value(&run).unwrap();
        assert_eq!(v["status"], "pending");
        assert_eq!(v["session_key"], "ws-x");
        assert_eq!(v["instruction"], "指令");
    }
}
