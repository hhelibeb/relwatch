use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    let _ = conn.execute(
        "INSERT INTO logs (level, message, created_at) VALUES (?1, ?2, ?3)",
        params![level, message, now],
    );
}

pub fn write_log_key(conn: &Connection, level: &str, key: &str, args: &str) {
    let now = chrono::Utc::now().to_rfc3339();

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
    let args_value: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
    let rendered = crate::i18n::render(key, &args_value, &locale);

    let _ = conn.execute(
        "INSERT INTO logs (level, message, message_key, message_args, rendered_message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![level, key, key, args, rendered, now],
    );
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
