use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 降级日志文件大小上限（1MB）：超过即轮转为 `fallback.log.1`（覆盖上一份），
/// 保证 DB 长期不可用时该文件不会无界增长。
const FALLBACK_MAX_BYTES: u64 = 1024 * 1024;

/// 降级日志目录：生产为 `%APPDATA%\RelWatch\logs`；单测走系统临时目录——
/// 「DB 写入失败」的用例会真的落文件，不能污染真实用户数据。
fn fallback_dir() -> PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join("relwatch-test-logs")
    } else {
        crate::db::init::app_data_dir().join("logs")
    }
}

/// 日志**降级通道**（V23）：DB 写入失败时把一行追加到 `logs/fallback.log`。
///
/// 为什么必须降级：V2 的全局错误兜底把「看不见的失败」变成「日志页可见」，
/// 但若日志写入本身失败（DB 锁 / 文件被占 / 磁盘异常）且依然静默，兜底通道
/// 就自毁了——用户以为「日志里一定有」，实际什么都没有。
///
/// 不重试 DB：失败通常是持锁或连接不可用，重试只会放大锁竞争。
/// 行格式 `时间|level|key|args`（与 DB 表的列顺序一致，便于事后人工导入）。
pub fn fallback_write(dir: &Path, line: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("fallback.log");
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= FALLBACK_MAX_BYTES {
        // 轮转移除旧备份；失败（如被占用）时继续追加，只损失边界
        let _ = std::fs::rename(&path, dir.join("fallback.log.1"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")
}

/// 写入失败时的统一降级：尽最大努力留痕，任何失败都不再向上抛
/// （调用方均忽略返回值 `()`，抛错会迫使全部调用点改签名）。
fn fallback_on_failure(level: &str, key: &str, args: &str) {
    let line = format!(
        "{}|{}|{}|{}",
        chrono::Utc::now().to_rfc3339(),
        level,
        key,
        args
    );
    let _ = fallback_write(&fallback_dir(), &line);
}

/// 主动把一条日志写进降级文件，**不尝试 DB**。
///
/// 与 [`fallback_on_failure`] 的区别在意图：那个是「DB 写失败」的被动降级；这里是
/// 调用方**明知不该碰 DB** 时的主动落盘。典型场景是**退出路径** —— 进程即将终止，
/// 同步取 DB 连接可能阻塞主线程（正是要消除的反模式），但这条记录又有事后排查价值：
/// release 版无控制台，`eprintln!` 完全不可见，不落文件就什么都留不下。
///
/// 行格式与 [`fallback_on_failure`] 一致（时间|level|key|args，与 DB `logs` 表列序
/// 对齐，可人工导入）。key 仍应在 `src/i18n/*.ts` 注册：若将来把它导入 DB，日志页
/// 要靠它渲染文案，否则会直接显示裸 key。
pub fn write_fallback_only(level: &str, key: &str, args: &str) {
    let line = format!(
        "{}|{}|{}|{}",
        chrono::Utc::now().to_rfc3339(),
        level,
        key,
        args
    );
    let _ = fallback_write(&fallback_dir(), &line);
}

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct LogEntry {
    pub id: i64,
    pub level: String,
    pub message: String,
    pub created_at: String,
    pub message_key: Option<String>,
    pub message_args: Option<String>,
    pub rendered_message: Option<String>,
}

/// 日志展示标识：YouTube 源用频道名（description）替代 channel_id，repo 置空；
/// 其余源保持 owner/repo 原样（GitHub 的 `{owner}/{repo}` 模板才能正常渲染）。
pub fn source_log_ident(
    source_type: &str,
    owner: &str,
    repo: &str,
    description: Option<&str>,
) -> (String, String) {
    if source_type == "youtube" {
        let name = description.filter(|d| !d.is_empty()).unwrap_or(owner);
        (name.to_string(), String::new())
    } else {
        (owner.to_string(), repo.to_string())
    }
}

/// release 级日志展示标识：YouTube 源额外用视频标题（release_name）替代 video_id。
/// 返回 (owner, repo, tag) 三元组，供 `{owner}/{repo} {tag}` 类模板渲染。
pub fn release_log_ident(r: &crate::db::releases::ReleaseInfo) -> (String, String, String) {
    if r.source_type == "youtube" {
        let owner = r
            .source_description
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| r.owner.clone());
        (owner, String::new(), r.release_name.clone())
    } else {
        (r.owner.clone(), r.repo.clone(), r.tag_name.clone())
    }
}

pub fn write_log(conn: &Connection, level: &str, message: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    // 所有日志出口默认脱敏（V28）：凭据（userinfo / query 参数 / 已知 token 形状）
    // 不得以明文落库——日志可搜索、可导出，等同于凭据外泄。
    let message = crate::redact::redact(message);
    if let Err(e) = conn.execute(
        "INSERT INTO logs (level, message, created_at) VALUES (?1, ?2, ?3)",
        params![level, message, now],
    ) {
        log::error!("日志写入失败（降级写文件，V23）: {}", e);
        fallback_on_failure(level, &message, "");
    }
}

pub fn write_log_key(conn: &Connection, level: &str, key: &str, args: &str) {
    let now = chrono::Utc::now().to_rfc3339();

    // 所有日志出口默认脱敏（V28）：先把 args 过滤再渲染，使 message_args 与
    // rendered_message 两侧都无明文，且渲染输入与落库内容一致。
    let args = crate::redact::redact(args);

    // 读取用户语言设置，渲染翻译文本用于搜索
    // 注意：rendered_message 在写入时固定了当前 locale。切换语言后已有日志行
    // 的 rendered_message 不会自动重新渲染，导致关键词搜索只能命中当前语言的行。
    // 这是有意为之的设计决策（locale-frozen），避免每次语言切换都触发全表回填。
    // 远期方案：改为惰性渲染，搜索时从 message_key + message_args 实时计算展示文本。
    let locale = crate::db::settings::get_setting_str(
        conn,
        crate::db::settings::KEY_LANGUAGE,
        &crate::db::settings::get_default_language(),
    ).unwrap_or_else(|_| crate::db::settings::get_default_language());
    let args_value: serde_json::Value = serde_json::from_str(&args).unwrap_or_default();
    let rendered = crate::i18n::render(key, &args_value, &locale);

    if let Err(e) = conn.execute(
        "INSERT INTO logs (level, message, message_key, message_args, rendered_message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![level, key, key, args, rendered, now],
    ) {
        log::error!("日志写入失败（降级写文件，V23）: {}", e);
        fallback_on_failure(level, key, &args);
    }
}

pub fn search_logs(
    conn: &Connection,
    keyword: &str,
    level: Option<&str>,
    page: i64,
    page_size: i64,
) -> Result<(Vec<LogEntry>, i64), String> {
    let offset = (page - 1) * page_size;
    let has_keyword = !keyword.is_empty();
    let has_level = level.is_some_and(|l| !l.is_empty() && l != "all");

    let mut sql = String::from(
        "SELECT id, level, message, message_key, message_args, rendered_message, created_at, COUNT(*) OVER() as total FROM logs WHERE 1=1"
    );
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if has_keyword {
        let pattern = format!("%{}%", keyword);
        sql.push_str(" AND (message LIKE ? OR level LIKE ? OR message_key LIKE ? OR rendered_message LIKE ?)");
        params_vec.push(Box::new(pattern.clone()));
        params_vec.push(Box::new(pattern.clone()));
        params_vec.push(Box::new(pattern.clone()));
        params_vec.push(Box::new(pattern));
    }

    if has_level {
        sql.push_str(" AND level = ?");
        params_vec.push(Box::new(level.unwrap().to_string()));
    }

    sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
    params_vec.push(Box::new(page_size));
    params_vec.push(Box::new(offset));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let mut total: i64 = 0;
    let logs = stmt
        .query_map(params_refs.as_slice(), |row| {
            total = row.get(7)?;
            Ok(LogEntry {
                id: row.get(0)?,
                level: row.get(1)?,
                message: row.get(2)?,
                message_key: row.get(3)?,
                message_args: row.get(4)?,
                rendered_message: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok((logs, total))
}

pub fn get_logs(conn: &Connection, limit: i64) -> Result<Vec<LogEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, level, message, message_key, message_args, rendered_message, created_at FROM logs ORDER BY id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;

    let logs = stmt
        .query_map(params![limit], |row| {
            Ok(LogEntry {
                id: row.get(0)?,
                level: row.get(1)?,
                message: row.get(2)?,
                message_key: row.get(3)?,
                message_args: row.get(4)?,
                rendered_message: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(logs)
}

pub fn delete_old_logs(conn: &Connection, days: i64) {
    if days <= 0 {
        return;
    }
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let _ = conn.execute("DELETE FROM logs WHERE created_at < ?1", rusqlite::params![cutoff]);
}

pub fn clear_logs(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM logs", rusqlite::params![])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 回填所有已有日志的 rendered_message（一次性迁移辅助）
pub fn backfill_rendered_messages(conn: &Connection) -> Result<usize, String> {
    let locale = crate::db::settings::get_setting_str(conn, crate::db::settings::KEY_LANGUAGE, "zh-CN")
        .unwrap_or_else(|_| "zh-CN".to_string());

    // 找出所有需要回填的行：有 message_key 但 rendered_message 为 NULL
    let mut stmt = conn
        .prepare(
            "SELECT id, message_key, message_args FROM logs WHERE message_key IS NOT NULL AND rendered_message IS NULL"
        )
        .map_err(|e| e.to_string())?;

    let rows: Vec<(i64, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let count = rows.len();
    if count == 0 {
        return Ok(0);
    }

    let mut updated = 0usize;
    for (id, key, args_opt) in &rows {
        let args_str = args_opt.as_deref().unwrap_or("{}");
        let args_value: serde_json::Value = serde_json::from_str(args_str).unwrap_or_default();
        let rendered = crate::i18n::render(key, &args_value, &locale);
        match conn.execute(
            "UPDATE logs SET rendered_message = ?1 WHERE id = ?2",
            params![rendered, id],
        ) {
            Ok(n) => updated += n,
            Err(e) => {
                eprintln!("[migration] backfill row {id} failed: {e}");
                return Err(format!("backfill row {id} failed: {e}"));
            }
        }
    }

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_logs_write_and_read() {
        let conn = init_memory_db().unwrap();
        write_log(&conn, "INFO", "msg1");
        write_log(&conn, "ERROR", "msg2");
        write_log_key(&conn, "INFO", "test.key", r#"{"a":"1"}"#);
        let logs = get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].message, "test.key");
        assert_eq!(logs[0].message_key.as_deref(), Some("test.key"));
        assert_eq!(logs[0].message_args.as_deref(), Some(r#"{"a":"1"}"#));
        // write_log 的 rendered_message 为 None
        assert!(logs[2].rendered_message.is_none());
        // write_log_key 的 unknown key 回退到 key 本身
        assert_eq!(logs[0].rendered_message.as_deref(), Some("test.key"));
    }

    #[test]
    fn test_logs_boundary() {
        let conn = init_memory_db().unwrap();
        write_log(&conn, "INFO", "msg1");
        write_log(&conn, "ERROR", "msg2");

        let logs = get_logs(&conn, 0).unwrap();
        assert_eq!(logs.len(), 0);

        let logs = get_logs(&conn, 100).unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn test_write_log_key_renders_rendered_message() {
        let conn = init_memory_db().unwrap();
        // 预设语言为 zh-CN
        crate::db::settings::set_setting(&conn, crate::db::settings::KEY_LANGUAGE, "zh-CN").unwrap();

        // 使用整数 count（与 Rust json!() 行为一致）
        write_log_key(&conn, "INFO", "check.auto",
            r#"{"owner":"user","repo":"repo","count":3}"#);
        let logs = get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].rendered_message.as_deref(),
            Some("检查 user/repo: 3 个新版本")
        );
    }

    #[test]
    fn test_write_log_key_renders_with_language_setting() {
        let conn = init_memory_db().unwrap();
        // 预设语言为 en-US
        crate::db::settings::set_setting(&conn, crate::db::settings::KEY_LANGUAGE, "en-US").unwrap();

        // 使用整数 count
        write_log_key(&conn, "INFO", "check.auto",
            r#"{"owner":"user","repo":"repo","count":1}"#);
        let logs = get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].rendered_message.as_deref(),
            Some("Check user/repo: 1 new release(s)")
        );
    }

    #[test]
    fn test_search_logs_matches_rendered_message() {
        let conn = init_memory_db().unwrap();
        crate::db::settings::set_setting(&conn, crate::db::settings::KEY_LANGUAGE, "zh-CN").unwrap();

        write_log_key(&conn, "INFO", "check.auto",
            r#"{"owner":"user","repo":"repo","count":3}"#);

        // 搜索渲染文本中的关键词
        let (entries, total) = search_logs(&conn, "3 个新版本", None, 1, 10).unwrap();
        assert_eq!(total, 1, "应通过 rendered_message 匹配");
        assert_eq!(entries[0].rendered_message.as_deref(), Some("检查 user/repo: 3 个新版本"));
    }

    #[test]
    fn test_search_logs_matches_rendered_message_zero() {
        let conn = init_memory_db().unwrap();
        crate::db::settings::set_setting(&conn, crate::db::settings::KEY_LANGUAGE, "zh-CN").unwrap();

        // count 为 0 的场景
        write_log_key(&conn, "INFO", "check.auto",
            r#"{"owner":"githubuser","repo":"somerepo","count":0}"#);

        let (entries, total) = search_logs(&conn, "0 个新版本", None, 1, 10).unwrap();
        assert_eq!(total, 1, "rendered_message 应匹配 0 个新版本");
        assert_eq!(
            entries[0].rendered_message.as_deref(),
            Some("检查 githubuser/somerepo: 0 个新版本")
        );
    }

    #[test]
    fn test_search_logs_still_matches_message_key() {
        let conn = init_memory_db().unwrap();
        write_log_key(&conn, "INFO", "check.auto",
            r#"{"owner":"user","repo":"repo","count":1}"#);

        // 原来的 key 搜索仍然有效
        let (_entries, total) = search_logs(&conn, "check.auto", None, 1, 10).unwrap();
        assert_eq!(total, 1, "message key 搜索仍然有效");
    }

    #[test]
    fn test_search_logs_filters_by_level() {
        let conn = init_memory_db().unwrap();
        write_log(&conn, "INFO", "info msg");
        write_log(&conn, "WARN", "warn msg");
        write_log(&conn, "ERROR", "error msg");

        let (entries, total) = search_logs(&conn, "", Some("ERROR"), 1, 10).unwrap();
        assert_eq!(total, 1);
        assert_eq!(entries[0].level, "ERROR");
        assert_eq!(entries[0].message, "error msg");

        let (entries, total) = search_logs(&conn, "", Some("INFO"), 1, 10).unwrap();
        assert_eq!(total, 1);
        assert_eq!(entries[0].level, "INFO");

        // "all" 不过滤
        let (_entries, total) = search_logs(&conn, "", Some("all"), 1, 10).unwrap();
        assert_eq!(total, 3);

        // None 不过滤
        let (_entries, total) = search_logs(&conn, "", None, 1, 10).unwrap();
        assert_eq!(total, 3);
    }

    #[test]
    fn test_check_failed_warn_and_error() {
        let conn = init_memory_db().unwrap();

        // 模拟 poll.rs 中的两种失败场景
        write_log_key(&conn, "WARN", "check.failed",
            r#"{"owner":"o","repo":"r","error":"rate limited"}"#);
        write_log_key(&conn, "ERROR", "check.failed",
            r#"{"owner":"o","repo":"r","error":"timeout after retry"}"#);

        let (warns, _) = search_logs(&conn, "", Some("WARN"), 1, 10).unwrap();
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].level, "WARN");
        assert!(warns[0].rendered_message.as_deref().unwrap().contains("rate limited"));

        let (errors, _) = search_logs(&conn, "", Some("ERROR"), 1, 10).unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].level, "ERROR");
        assert!(errors[0].rendered_message.as_deref().unwrap().contains("timeout"));
    }

    // ── 错误状态码 → 日志级别映射矩阵 ─────────────────────
    //
    // poll.rs 中 check 失败时的日志级别判定（内联闭包，出现两处）：
    //     let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 {
    //         "WARN"
    //     } else {
    //         "ERROR"
    //     };
    //
    // 语义：临时性错误（网络 0、认证 401、限流 429、5xx 服务端）记 WARN，
    //       永久性错误（404 不存在、422 参数等 4xx）记 ERROR。
    // 该判定目前内联无法直接测试，这里把规则作为不变量复刻并锁住，
    // 重构时若改动判定逻辑，此处必须同步——否则用户可见的日志级别会变。

    /// 复刻 poll.rs 中的状态码→级别判定。保持同步。
    fn check_failure_log_level(status: u16) -> &'static str {
        if matches!(status, 0 | 401 | 403 | 429) || status >= 500 {
            "WARN"
        } else {
            "ERROR"
        }
    }

    #[test]
    fn test_failure_log_level_matrix() {
        // 临时性错误 → WARN
        assert_eq!(check_failure_log_level(0), "WARN", "网络错误(0) 应 WARN");
        assert_eq!(check_failure_log_level(401), "WARN", "未授权(401) 应 WARN");
        assert_eq!(check_failure_log_level(403), "WARN", "禁止访问(403) 应 WARN");
        assert_eq!(check_failure_log_level(429), "WARN", "限流(429) 应 WARN");
        assert_eq!(check_failure_log_level(500), "WARN", "服务端错误(500) 应 WARN");
        assert_eq!(check_failure_log_level(502), "WARN", "网关错误(502) 应 WARN");
        assert_eq!(check_failure_log_level(503), "WARN", "服务不可用(503) 应 WARN");

        // 永久性错误 → ERROR
        assert_eq!(check_failure_log_level(400), "ERROR", "请求错误(400) 应 ERROR");
        assert_eq!(check_failure_log_level(404), "ERROR", "仓库不存在(404) 应 ERROR");
        assert_eq!(check_failure_log_level(422), "ERROR", "参数错误(422) 应 ERROR");

        // 边界：499（最后一个 4xx）应 ERROR，500（第一个 5xx）应 WARN
        assert_eq!(check_failure_log_level(499), "ERROR");
        assert_eq!(check_failure_log_level(500), "WARN");
    }

    // ── V23：日志写入静默失败 → 降级写文件 ─────────────────────

    /// 每个用例独立的临时目录，避免并行测试互相干扰。
    fn temp_log_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("relwatch-test-logs-{}-{}", std::process::id(), tag))
    }

    #[test]
    fn fallback_write_appends_line() {
        let dir = temp_log_dir("append");
        let _ = std::fs::remove_dir_all(&dir);

        fallback_write(&dir, "2026-09-11T00:00:00Z|INFO|check.auto|{}").unwrap();
        fallback_write(&dir, "2026-09-11T00:00:01Z|ERROR|ui.vue_error|{}").unwrap();

        let content = std::fs::read_to_string(dir.join("fallback.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "追加而非覆盖: {content}");
        assert!(lines[0].starts_with("2026-09-11T00:00:00Z|INFO|check.auto"));
        assert!(lines[1].contains("ui.vue_error"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fallback_write_rotates_when_over_limit() {
        let dir = temp_log_dir("rotate");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 预置一个超限文件（内容不重要，只看轮转行为）
        std::fs::write(dir.join("fallback.log"), "x".repeat(FALLBACK_MAX_BYTES as usize)).unwrap();
        fallback_write(&dir, "after-rotate").unwrap();

        assert!(dir.join("fallback.log.1").exists(), "超限后应轮转为 .1");
        let current = std::fs::read_to_string(dir.join("fallback.log")).unwrap();
        assert_eq!(current.trim_end(), "after-rotate", "新文件只含轮转后的行");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_log_degrades_to_fallback_file_when_table_missing() {
        // 无 logs 表（DB 不可用）时应降级而非 panic。
        // 旧版只断言「不 panic」——那连「降级到底做没做」都没测到（函数体全空也绿）。
        // 这里进一步断言降级文件确实拿到那一行。
        let conn = Connection::open_in_memory().unwrap();
        let dir = fallback_dir();
        let _ = std::fs::remove_dir_all(&dir);

        write_log(&conn, "ERROR", "boom");
        write_log_key(&conn, "ERROR", "check.failed", r#"{"error":"boom"}"#);

        let content = std::fs::read_to_string(dir.join("fallback.log"))
            .expect("DB 写入失败必须降级写 fallback.log（V23）");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "两次失败各降级一行: {content}");
        assert!(lines[0].ends_with("|ERROR|boom|"), "{} ", lines[0]);
        assert!(lines[1].contains("|ERROR|check.failed|"), "{}", lines[1]);
        // 降级行不得带明文凭据
        write_log(&conn, "WARN", "url (https://h/p?token=SECRETVALUE)");
        let content = std::fs::read_to_string(dir.join("fallback.log")).unwrap();
        assert!(!content.contains("SECRETVALUE"), "降级文件泄露凭据: {content}");

        // 该目录是所有用例共享的固定位置（`fallback_dir()` 在 cfg(test) 下恒定），
        // 用完即清，不给 TEMP 留测试垃圾。
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── V28：日志出口默认脱敏 ─────────────────────────────

    #[test]
    fn write_log_key_redacts_credentials_in_args_and_rendered() {
        let conn = init_memory_db().unwrap();
        crate::db::settings::set_setting(&conn, crate::db::settings::KEY_LANGUAGE, "zh-CN").unwrap();

        write_log_key(
            &conn,
            "WARN",
            "check.failed",
            &serde_json::json!({
                "owner": "Freesia",
                "repo": "",
                "error": "err.request_failed|error sending request for url (https://youtube.googleapis.com/youtube/v3/channels?id=UC1&key=AIzaSyFAKEKEY0000000000000000000000)"
            })
            .to_string(),
        );

        let logs = get_logs(&conn, 10).unwrap();
        let args = logs[0].message_args.clone().unwrap();
        let rendered = logs[0].rendered_message.clone().unwrap();
        assert!(!args.contains("AIzaSy"), "message_args 泄露: {args}");
        assert!(!rendered.contains("AIzaSy"), "rendered_message 泄露: {rendered}");
        assert!(args.contains("&key=***)"), "保留参数名与闭合括号: {args}");
        // 搜索依然能命中被脱敏后的日志
        let (_entries, total) = search_logs(&conn, "key=***", None, 1, 10).unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn write_log_redacts_credentials_in_message() {
        let conn = init_memory_db().unwrap();
        write_log(
            &conn,
            "ERROR",
            "proxy connect failed: http://admin:hunter2@10.0.0.1:3128",
        );
        let logs = get_logs(&conn, 10).unwrap();
        assert!(!logs[0].message.contains("hunter2"), "{}", logs[0].message);
        assert!(logs[0].message.contains("***:***@10.0.0.1:3128"));
    }

    /// CI 护栅（制度化）：logs 表只允许经本模块写入。
    ///
    /// 本模块的 `write_log` / `write_log_key` 是唯一的默认脱敏出口；任何绕过它
    /// 直接 `INSERT INTO logs` 的新代码都会把凭据泄露风险重新引进来。
    #[test]
    fn insert_into_logs_is_confined_to_this_module() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                // 只看生产代码：测试模块可以直写（如 Migration 18 的用例需要造历史明文行）
                let prod = match src.find("#[cfg(test)]") {
                    Some(pos) => &src[..pos],
                    None => src.as_str(),
                };
                let rel = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel != "db/logs.rs" && prod.contains("INSERT INTO logs") {
                    offenders.push(rel);
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "发现绕过脱敏出口的日志写入，请改用 db::logs::write_log/write_log_key: {offenders:?}"
        );
    }

    #[test]
    fn test_check_failed_writes_correct_level_for_each_status() {
        // 端到端验证：不同状态码对应的级别被正确写入 logs 表
        let conn = init_memory_db().unwrap();
        crate::db::settings::set_setting(
            &conn,
            crate::db::settings::KEY_LANGUAGE,
            "zh-CN",
        ).unwrap();

        let cases: [(u16, &str); 3] = [
            (503, "WARN"),
            (404, "ERROR"),
            (429, "WARN"),
        ];
        for (status, expected_level) in cases {
            let level = check_failure_log_level(status);
            assert_eq!(level, expected_level, "status {} 级别判定", status);
            write_log_key(
                &conn,
                level,
                "check.failed",
                &serde_json::json!({"owner":"o","repo":"r","error": format!("status {status}")}).to_string(),
            );
        }

        let (warns, _) = search_logs(&conn, "", Some("WARN"), 1, 10).unwrap();
        assert_eq!(warns.len(), 2, "503 和 429 应各产生 1 条 WARN，共 2 条");
        let (errors, _) = search_logs(&conn, "", Some("ERROR"), 1, 10).unwrap();
        assert_eq!(errors.len(), 1, "404 应产生 1 条 ERROR");
    }
}
