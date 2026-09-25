use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct Source {
    pub id: i64,
    pub source_type: String,
    pub owner: String,
    pub repo: String,
    pub poll_interval_minutes: i64,
    pub enabled: bool,
    pub last_checked_at: Option<String>,
    pub last_check_status: String,
    pub last_check_message: Option<String>,
    pub consecutive_failures: i64,
    pub last_new_count: i64,
    pub muted: bool,
    pub created_at: String,
    pub updated_at: String,
    pub description: Option<String>,
    /// 源级附加配置（JSON）。目前用于 YouTube 订阅内容类型（视频/直播/帖子）。
    pub config: Option<String>,
}

/// 仅测试使用的便捷封装（生产统一走 `add_source_with_config`）。
#[cfg(test)]
pub fn add_source(
    conn: &Connection,
    source_type: &str,
    owner: &str,
    repo: &str,
    description: &str,
) -> Result<i64, String> {
    add_source_with_config(conn, source_type, owner, repo, description, None)
}

/// 带源级配置的添加（config 为 JSON 字符串，如 YouTube 订阅内容类型）。
pub fn add_source_with_config(
    conn: &Connection,
    source_type: &str,
    owner: &str,
    repo: &str,
    description: &str,
    config: Option<&str>,
) -> Result<i64, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let desc = if description.is_empty() { None } else { Some(description) };
    // `poll_interval_minutes` **显式**写 0 = 未设置（跟随全局）：老库的建表默认值
    // 仍是历史遗留的 30，省略此列会让每个新源凭空带上一个 30 分钟间隔。
    conn.execute(
        "INSERT OR IGNORE INTO sources (source_type, owner, repo, poll_interval_minutes, description, config, created_at, updated_at)
         VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7)",
        params![source_type, owner, repo, desc, config, now, now],
    )
    .map_err(|e| e.to_string())?;
    if conn.changes() == 0 {
        return Ok(0);
    }
    Ok(conn.last_insert_rowid())
}

pub fn source_exists(
    conn: &Connection,
    source_type: &str,
    owner: &str,
    repo: &str,
) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(
            "SELECT 1 FROM sources
             WHERE source_type = ?1 AND lower(owner) = lower(?2) AND lower(repo) = lower(?3)
             LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    stmt.exists(params![source_type, owner, repo])
        .map_err(|e| e.to_string())
}

pub fn remove_source(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM sources WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_source(conn: &Connection, id: i64) -> Result<Option<Source>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source_type, owner, repo, poll_interval_minutes, enabled,
                    last_checked_at, last_check_status, last_check_message,
                    consecutive_failures, last_new_count, muted, created_at, updated_at,
                    description, config
             FROM sources WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(Source {
                id: row.get(0)?,
                source_type: row.get(1)?,
                owner: row.get(2)?,
                repo: row.get(3)?,
                poll_interval_minutes: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_checked_at: row.get(6)?,
                last_check_status: row.get(7)?,
                last_check_message: row.get(8)?,
                consecutive_failures: row.get(9)?,
                last_new_count: row.get(10)?,
                muted: row.get::<_, i64>(11)? != 0,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                description: row.get(14)?,
                config: row.get(15)?,
            })
        })
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok(source)) => Ok(Some(source)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

pub fn update_source(
    conn: &Connection,
    id: i64,
    enabled: bool,
    poll_interval_minutes: i64,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    if enabled {
        // 手动重新启用时重置连续失败计数，避免一次失败即再次被断路器禁用
        conn.execute(
            "UPDATE sources SET enabled = 1, poll_interval_minutes = ?1, consecutive_failures = 0, updated_at = ?2 WHERE id = ?3",
            params![poll_interval_minutes, now, id],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "UPDATE sources SET enabled = 0, poll_interval_minutes = ?1, updated_at = ?2 WHERE id = ?3",
            params![poll_interval_minutes, now, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn update_source_config(conn: &Connection, id: i64, config: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sources SET config = ?1, updated_at = ?2 WHERE id = ?3",
        params![config, chrono::Utc::now().to_rfc3339(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_source_muted(
    conn: &Connection,
    id: i64,
    muted: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE sources SET muted = ?1, updated_at = ?2 WHERE id = ?3",
        params![muted as i64, chrono::Utc::now().to_rfc3339(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_muted_source_ids(conn: &Connection) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM sources WHERE muted = 1")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(ids)
}

pub fn record_check_success(conn: &Connection, id: i64, new_count: usize) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE sources
         SET last_checked_at = ?1,
             last_check_status = 'ok',
             last_check_message = NULL,
             consecutive_failures = 0,
             last_new_count = ?2,
             updated_at = ?1
         WHERE id = ?3",
        params![now, new_count as i64, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 记录一次检查失败（源健康状态 + 累计失败数）。
///
/// `sources.last_check_message` 是**不经过** `db::logs::write_log_key` 的凭据出口
/// （V28）：错误文本常回显完整请求 URL（如 `…&key=AIzaSy…`），而该列会被源列表 /
/// 详情展示，并随备份导出。故 `record_check_failure_inner`（该列的唯一写入者，
/// 两个公开入口共用）在落库前统一脱敏，避免每个调用点各写一遍而漏掉某一处。
pub fn record_check_failure(conn: &Connection, id: i64, message: &str) -> Result<(), String> {
    record_check_failure_inner(conn, id, message, true)
}

/// 记录一次检查失败，但**不**累加 `consecutive_failures`。
///
/// 用于两类「不是源本身坏了」的失败（误伤断路器的那两类）：
/// - 上游按账号/配额/风控拒绝服务（YouTube 配额、B 站限流/风控、API key 失效）：
///   重试会自愈（配额按日重置），且常一次命中全部同类源（共用 key/cookie），
///   把它们逐个自动禁用纯属误伤。
/// - 本轮已判定为网络层抖动（≥50% 源同轮失败）时预检失败的全量源。
///
/// 状态与消息照旧落库（`last_check_status/last_check_message`），所以源列表依旧显示
/// 红点与错误文本，只是不再往断路器的计数上摞。
pub fn record_check_failure_uncounted(conn: &Connection, id: i64, message: &str) -> Result<(), String> {
    record_check_failure_inner(conn, id, message, false)
}

/// 回退一次失败累加（下限 0）。
///
/// 断路器判定需要**整轮结果**才能下结论（见 `poll::apply_mass_failure_guard`），而失败
/// 记录发生在单源检查内部、早于整轮汇总。故采用「先照常累加、整轮汇总后回退」：
/// 每轮每源最多累加 1，所以回退 1 就等于本轮不累加，不会误伤上一轮的计数。
pub fn decrement_consecutive_failures(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE sources SET consecutive_failures = MAX(consecutive_failures - 1, 0), updated_at = ?2 WHERE id = ?1",
        params![id, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn record_check_failure_inner(
    conn: &Connection,
    id: i64,
    message: &str,
    count_failure: bool,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let trimmed: String = crate::redact::redact(message).chars().take(500).collect();
    conn.execute(
        if count_failure {
            "UPDATE sources
             SET last_checked_at = ?1,
                 last_check_status = 'error',
                 last_check_message = ?2,
                 consecutive_failures = consecutive_failures + 1,
                 last_new_count = 0,
                 updated_at = ?1
             WHERE id = ?3"
        } else {
            "UPDATE sources
             SET last_checked_at = ?1,
                 last_check_status = 'error',
                 last_check_message = ?2,
                 last_new_count = 0,
                 updated_at = ?1
             WHERE id = ?3"
        },
        params![now, trimmed, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 读源级配置槽（`sources.config` JSON）里的布尔开关：`Some(true/false)` = 该源显式覆盖，
/// `None` = 未配置/解析失败/非布尔值（调用方回退全局设置）。
///
/// 目前只用于 `fetch_history` 与 `check_prereleases` 两个开关的「全局默认 + 按源覆盖」。
pub fn config_flag(config: Option<&str>, key: &str) -> Option<bool> {
    let parsed: serde_json::Value = serde_json::from_str(config?).ok()?;
    parsed.get(key)?.as_bool()
}

/// 同上，但按源 id 取行后解析（适配器只能拿到 `source_id` 时用，如 `github::save_releases`）。
pub fn source_config_flag(conn: &Connection, id: i64, key: &str) -> Option<bool> {
    let config: Option<String> = conn
        .query_row("SELECT config FROM sources WHERE id = ?1", params![id], |r| r.get(0))
        .ok()
        .flatten();
    config_flag(config.as_deref(), key)
}

pub fn update_source_description(conn: &Connection, id: i64, description: &str) -> Result<(), String> {
    let desc = if description.is_empty() { None } else { Some(description) };
    conn.execute(
        "UPDATE sources SET description = ?1, updated_at = ?2 WHERE id = ?3",
        params![desc, chrono::Utc::now().to_rfc3339(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source_type, owner, repo, poll_interval_minutes, enabled,
                    last_checked_at, last_check_status, last_check_message,
                    consecutive_failures, last_new_count, muted, created_at, updated_at,
                    description, config
             FROM sources ORDER BY id DESC",
        )
        .map_err(|e| e.to_string())?;

    let sources = stmt
        .query_map([], |row| {
            Ok(Source {
                id: row.get(0)?,
                source_type: row.get(1)?,
                owner: row.get(2)?,
                repo: row.get(3)?,
                poll_interval_minutes: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_checked_at: row.get(6)?,
                last_check_status: row.get(7)?,
                last_check_message: row.get(8)?,
                consecutive_failures: row.get(9)?,
                last_new_count: row.get(10)?,
                muted: row.get::<_, i64>(11)? != 0,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                description: row.get(14)?,
                config: row.get(15)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_remove_nonexistent_source() {
        let conn = init_memory_db().unwrap();
        // 删除不存在的 id 不应报错
        let result = remove_source(&conn, 999);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_nonexistent_source() {
        let conn = init_memory_db().unwrap();
        let result = get_source(&conn, 999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_nonexistent_source() {
        let conn = init_memory_db().unwrap();
        // 更新不存在的 id 不应报错（0 rows affected）
        let result = update_source(&conn, 999, false, 60);
        assert!(result.is_ok());
    }

    #[test]
    fn test_record_check_failure_redacts_credentials() {
        // V28：last_check_message 是独立于日志表的凭据出口，必须在唯一写入者处脱敏
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "youtube", "UC1", "", "").unwrap();
        record_check_failure(
            &conn,
            id,
            "err.request_failed|error sending request for url (https://youtube.googleapis.com/youtube/v3/channels?id=UC1&key=AIzaSyFAKEKEY0000000000000000000000)",
        )
        .unwrap();

        let s = get_source(&conn, id).unwrap().unwrap();
        let msg = s.last_check_message.unwrap();
        assert!(!msg.contains("AIzaSy"), "凭据泄露到 last_check_message: {msg}");
        assert!(msg.contains("&key=***)"), "应保留参数名与闭合括号: {msg}");
        // 诊断信息不丢
        assert!(msg.contains("youtube/v3/channels?id=UC1"));
        assert_eq!(s.last_check_status, "error");
        assert_eq!(s.consecutive_failures, 1);
    }

    #[test]
    fn test_record_check_failure_truncates_long_message() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        // 构造超过 500 字符的错误消息
        let long_msg = "x".repeat(1000);
        record_check_failure(&conn, id, &long_msg).unwrap();

        let s = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(s.last_check_status, "error");
        assert!(s.last_check_message.is_some());
        // 应被截断到 500 字符
        assert!(s.last_check_message.as_ref().unwrap().len() <= 500);
        assert_eq!(s.consecutive_failures, 1);
    }

    /// 不计入型失败：状态/消息照旧落库（源列表红点不变），但不动断路器计数。
    #[test]
    fn test_record_check_failure_uncounted_keeps_counter() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "youtube", "UCtest", "", "").unwrap();

        record_check_failure_uncounted(&conn, id, "err.youtube_api_quota|quotaExceeded").unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert_eq!(s.consecutive_failures, 0, "配额类失败不应累加连续失败计数");
        assert_eq!(s.last_check_status, "error", "状态仍应落库（否则源列表看不出异常）");
        assert!(s.last_check_message.as_deref().unwrap().contains("youtube_api_quota"));

        // 普通失败仍照旧累加，保证断路器本身不被废掉
        record_check_failure(&conn, id, "err.api_error|500|x").unwrap();
        assert_eq!(list_sources(&conn).unwrap()[0].consecutive_failures, 1);
    }

    /// 整轮回退：累加后回退 1 等于本轮不计入，且下限为 0（不会倒欠）。
    #[test]
    fn test_decrement_consecutive_failures_floors_at_zero() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        decrement_consecutive_failures(&conn, id).unwrap();
        assert_eq!(list_sources(&conn).unwrap()[0].consecutive_failures, 0, "下限应为 0");

        record_check_failure(&conn, id, "timeout").unwrap();
        record_check_failure(&conn, id, "timeout").unwrap();
        decrement_consecutive_failures(&conn, id).unwrap();
        assert_eq!(
            list_sources(&conn).unwrap()[0].consecutive_failures,
            1,
            "回退 1 只抵消本轮累加，不碰上一轮计数"
        );
    }

    /// 源级开关解析：未配置/非布尔/JSON 损坏 → None（调用方回退全局）。
    #[test]
    fn test_source_config_flag() {
        assert_eq!(config_flag(None, "fetch_history"), None);
        assert_eq!(config_flag(Some("not json"), "fetch_history"), None);
        assert_eq!(config_flag(Some("{\"videos\":true}"), "fetch_history"), None);
        assert_eq!(
            config_flag(Some("{\"videos\":true,\"fetch_history\":false}"), "fetch_history"),
            Some(false),
            "同一条 config 里 YouTube 订阅与通用开关共存"
        );

        // 按 id 读取（适配器只有 source_id 时用）
        let conn = init_memory_db().unwrap();
        let id = add_source_with_config(&conn, "github", "o", "r", "", Some("{\"check_prereleases\":true}")).unwrap();
        assert_eq!(source_config_flag(&conn, id, "check_prereleases"), Some(true));
        assert_eq!(source_config_flag(&conn, 9999, "check_prereleases"), None);
    }

    #[test]
    fn test_source_add_and_list() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "microsoft", "vscode", "Code editor").unwrap();
        assert!(id > 0);
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].owner, "microsoft");
        assert!(sources[0].enabled);
        assert_eq!(sources[0].last_check_status, "unknown");
        assert_eq!(sources[0].consecutive_failures, 0);
    }

    #[test]
    fn test_source_remove() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "a", "b", "").unwrap();
        remove_source(&conn, id).unwrap();
        assert_eq!(list_sources(&conn).unwrap().len(), 0);
    }

    #[test]
    fn test_source_update() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();
        update_source(&conn, id, false, 60).unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert!(!s.enabled);
        assert_eq!(s.poll_interval_minutes, 60);
    }

    /// 新源的间隔恒为 0（未设置/跟随全局），**不能**依赖 DDL 默认值：
    /// 老库的 `sources` 建表默认值仍是历史遗留的 30，靠默认值会让每个新源
    /// 凭空带上一个 30 分钟间隔（将来重新引入按源调度就是静默降频）。
    #[test]
    fn test_add_source_leaves_interval_unset_on_legacy_schema() {
        let conn = Connection::open_in_memory().unwrap();
        // 复刻老库（含 DEFAULT 30）的 sources DDL；description/config 是历史上
        // 由 Migration 4/11 补上的列，INSERT 会用到，故一并声明。
        conn.execute_batch(
            "CREATE TABLE sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_type TEXT NOT NULL,
                owner TEXT NOT NULL,
                repo TEXT NOT NULL,
                poll_interval_minutes INTEGER NOT NULL DEFAULT 30,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_checked_at TEXT,
                last_check_status TEXT NOT NULL DEFAULT 'unknown',
                last_check_message TEXT,
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                last_new_count INTEGER NOT NULL DEFAULT 0,
                muted INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                description TEXT,
                config TEXT,
                UNIQUE(source_type, owner, repo)
            );",
        )
        .unwrap();

        let id = add_source_with_config(&conn, "github", "o", "r", "", None).unwrap();
        assert!(id > 0);
        let iv: i64 = conn
            .query_row(
                "SELECT poll_interval_minutes FROM sources WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(iv, 0, "新源应写入 0 = 未设置，而不是落 DDL 默认值 30");
    }

    #[test]
    fn test_re_enable_resets_consecutive_failures() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        // 模拟连续失败累积
        for _ in 0..3 {
            record_check_failure(&conn, id, "fail").unwrap();
        }
        let s = &list_sources(&conn).unwrap()[0];
        assert_eq!(s.consecutive_failures, 3);

        // 断路器禁用
        update_source(&conn, id, false, 60).unwrap();

        // 用户手动重新启用 — 应重置连续失败计数
        update_source(&conn, id, true, 60).unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert!(s.enabled);
        assert_eq!(s.consecutive_failures, 0, "re-enable 后 consecutive_failures 应重置为 0");
    }

    #[test]
    fn test_source_health_success_and_failure() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        record_check_failure(&conn, id, "network failed").unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert_eq!(s.last_check_status, "error");
        assert_eq!(s.last_check_message.as_deref(), Some("network failed"));
        assert_eq!(s.consecutive_failures, 1);
        assert_eq!(s.last_new_count, 0);
        assert!(s.last_checked_at.is_some());

        record_check_success(&conn, id, 3).unwrap();
        let s = &list_sources(&conn).unwrap()[0];
        assert_eq!(s.last_check_status, "ok");
        assert!(s.last_check_message.is_none());
        assert_eq!(s.consecutive_failures, 0);
        assert_eq!(s.last_new_count, 3);
    }

    #[test]
    fn test_source_muted_default_false() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.muted, "新建源的 muted 应默认为 false");
    }

    #[test]
    fn test_source_set_muted_toggle() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "a", "b", "").unwrap();

        // 切换为静默
        set_source_muted(&conn, id, true).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.muted);

        // 取消静默
        set_source_muted(&conn, id, false).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.muted);
    }

    #[test]
    fn test_source_muted_independent_of_enabled() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "x", "y", "").unwrap();

        // 先设为静默
        set_source_muted(&conn, id, true).unwrap();

        // 暂停（enabled=false）
        update_source(&conn, id, false, 30).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.enabled);
        assert!(s.muted, "muted 应跨暂停保留");

        // 恢复（enabled=true）后仍为静默
        update_source(&conn, id, true, 30).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.enabled);
        assert!(s.muted, "muted 应在恢复后仍然保持");
    }

    #[test]
    fn test_add_source_duplicate() {
        let conn = init_memory_db().unwrap();
        let id1 = add_source(&conn, "github", "microsoft", "vscode", "Code editor").unwrap();
        assert!(id1 > 0);
        let id2 = add_source(&conn, "github", "microsoft", "vscode", "Code editor").unwrap();
        assert_eq!(id2, 0);
        let sources = list_sources(&conn).unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn test_add_source_config_default_none() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "o", "r", "").unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.config.is_none(), "普通源 config 应为 None");
    }

    #[test]
    fn test_add_source_with_config_roundtrip() {
        let conn = init_memory_db().unwrap();
        let cfg = r#"{"videos":true,"live":true,"posts":false}"#;
        let id = add_source_with_config(&conn, "youtube", "UCtest123", "", "", Some(cfg)).unwrap();
        assert!(id > 0);
        let s = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(s.config.as_deref(), Some(cfg));
        // list_sources 也应带出 config
        let listed = list_sources(&conn).unwrap();
        assert_eq!(listed[0].config.as_deref(), Some(cfg));
    }

    #[test]
    fn test_update_source_config() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "youtube", "UCtest123", "", "").unwrap();
        let cfg = r#"{"videos":false,"live":true,"posts":false}"#;
        update_source_config(&conn, id, cfg).unwrap();
        let s = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(s.config.as_deref(), Some(cfg));
    }

    #[test]
    fn test_update_source_config_nonexistent_id_is_noop() {
        let conn = init_memory_db().unwrap();
        let result = update_source_config(&conn, 999, "{}");
        assert!(result.is_ok());
    }

    // ── 断路器端到端：连续失败达阈值后触发禁用 ──────────────
    //
    // poll.rs::do_poll_async 中的断路器逻辑（MAX_CONSECUTIVE_FAILURES=3）：
    //   当 source.consecutive_failures >= 3 时，调用 update_source(conn, id, false, interval) 禁用。
    // 这里把"累积失败 → 检测阈值 → 触发禁用"的完整判定链路锁住，
    // 重构断路器时这些不变量必须保持。

    /// 复刻 poll.rs 中的断路器阈值常量，保持同步以检测漂移。
    const TEST_MAX_CONSECUTIVE_FAILURES: i64 = 3;

    #[test]
    fn test_circuit_breaker_disables_after_threshold_failures() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "o", "r", "").unwrap();

        // 累积 3 次失败（恰好达到阈值）
        for _ in 0..TEST_MAX_CONSECUTIVE_FAILURES {
            record_check_failure(&conn, id, "timeout").unwrap();
        }
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.enabled, "失败累积期间源仍应 enabled");
        assert_eq!(s.consecutive_failures, TEST_MAX_CONSECUTIVE_FAILURES);

        // 模拟断路器判定：consecutive_failures >= 阈值 → 禁用
        let should_disable = s.enabled && s.consecutive_failures >= TEST_MAX_CONSECUTIVE_FAILURES;
        assert!(should_disable, "达到阈值应触发断路器");
        if should_disable {
            update_source(&conn, id, false, s.poll_interval_minutes).unwrap();
        }

        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(!s.enabled, "断路器触发后源应被禁用");
    }

    #[test]
    fn test_circuit_breaker_does_not_trigger_below_threshold() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "o", "r", "").unwrap();

        // 只累积 2 次失败（阈值 - 1）
        for _ in 0..(TEST_MAX_CONSECUTIVE_FAILURES - 1) {
            record_check_failure(&conn, id, "timeout").unwrap();
        }
        let s = get_source(&conn, id).unwrap().unwrap();
        assert!(s.enabled);
        assert_eq!(s.consecutive_failures, TEST_MAX_CONSECUTIVE_FAILURES - 1);

        // 阈值之下不应触发
        assert!(s.consecutive_failures < TEST_MAX_CONSECUTIVE_FAILURES);
    }

    #[test]
    fn test_success_resets_failure_counter_before_circuit_breaker() {
        let conn = init_memory_db().unwrap();
        let id = add_source(&conn, "github", "o", "r", "").unwrap();

        // 累积 2 次失败（阈值 - 1），再来一次成功
        record_check_failure(&conn, id, "fail1").unwrap();
        record_check_failure(&conn, id, "fail2").unwrap();
        record_check_success(&conn, id, 0).unwrap();

        let s = get_source(&conn, id).unwrap().unwrap();
        assert_eq!(s.consecutive_failures, 0, "成功应重置失败计数");
        assert!(s.enabled, "源不应被禁用");
    }
}
