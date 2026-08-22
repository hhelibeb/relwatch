use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::settings::{get_setting_str, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE};

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct ReleaseInfo {
    pub id: i64,
    pub source_id: i64,
    pub source_type: String,
    pub owner: String,
    pub repo: String,
    pub tag_name: String,
    pub release_name: String,
    pub html_url: String,
    pub published_at: String,
    pub prerelease: bool,
    pub body: Option<String>,
    pub detected_at: String,
    pub notification_status: String,
    pub snooze_until: Option<String>,
    pub ai_summary: Option<String>,
    pub ai_importance: Option<String>,
    pub body_translated: Option<String>,
    pub extra_metadata: Option<String>,
    /// 所属源的描述（YouTube 源存频道名），前端用于展示可读名称。
    pub source_description: Option<String>,
}

#[allow(clippy::too_many_arguments)]
/// 插入一条 release；已存在（UNIQUE(source_id, tag_name) 去重命中）返回 0。
///
/// releases 与 notification_state 两条写入在**同一事务**内完成（H-1 修复）：
/// 此前无事务时第二条 INSERT 失败会让 release 缺失 state 行，而查询层
/// `COALESCE(ns.status, 'pending')` 仍会选中它、`set_last_notified_at` 纯 UPDATE
/// 又落空，导致该 release 每轮轮询都被重复通知。
pub fn insert_release(
    conn: &Connection,
    source_id: i64,
    tag_name: &str,
    release_name: &str,
    html_url: &str,
    published_at: &str,
    prerelease: bool,
    body: Option<&str>,
) -> Result<i64, String> {
    let now = chrono::Utc::now().to_rfc3339();
    // conn 为 &Connection：用 unchecked_transaction（rusqlite 对 &self 的 safe 变体），
    // 本函数内顺序执行、无并发借用，等价于独占事务。
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO releases (source_id, tag_name, release_name, html_url, published_at, prerelease, body, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![source_id, tag_name, release_name, html_url, published_at, prerelease as i64, body, now],
    )
    .map_err(|e| e.to_string())?;

    if tx.changes() == 0 {
        // 去重命中：不提交（无事可写），返回 0
        return Ok(0);
    }

    let release_id = tx.last_insert_rowid();

    if release_id > 0 {
        tx.execute(
            "INSERT OR IGNORE INTO notification_state (release_id, status, created_at, updated_at)
             VALUES (?1, 'pending', ?2, ?2)",
            params![release_id, now],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(release_id)
}

/// 根据排除集合动态构建 `NOT IN (...)` 条件子句与参数。
/// 排除集合为空时不附加条件（不排除任何源类型）。
fn exclusion_clause<'a>(
    excluded_types: &'a [&'a str],
) -> (String, Vec<&'a dyn rusqlite::ToSql>) {
    if excluded_types.is_empty() {
        return (String::new(), Vec::new());
    }
    let placeholders = vec!["?"; excluded_types.len()].join(",");
    let sql = format!(" AND s.source_type NOT IN ({})", placeholders);
    let params: Vec<&dyn rusqlite::ToSql> =
        excluded_types.iter().map(|t| t as &dyn rusqlite::ToSql).collect();
    (sql, params)
}

/// Returns (id, body) tuples for releases where AI summary generation is missing.
/// Used by the poll cycle to retry failed summaries.
///
/// `excluded_types`：不参与 AI 摘要的源类型集合（如 youtube/bilibili），
/// 由 poll 编排层从 `list_adapters()` 的能力声明动态收集，
/// 新增源类型声明 `ai_eligible=false` 后自动生效，不在此硬编码具体类型。
pub fn get_releases_without_summary(
    conn: &Connection,
    excluded_types: &[&str],
) -> Result<Vec<(i64, Option<String>)>, String> {
    let (exclude_sql, params) = exclusion_clause(excluded_types);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.id, r.body FROM releases r
             JOIN sources s ON s.id = r.source_id
             WHERE r.ai_summary IS NULL AND r.body IS NOT NULL AND r.body != ''
               AND (r.retry_count IS NULL OR r.retry_count < 5){}",
            exclude_sql
        ))
        .map_err(|e| e.to_string())?;

    let releases = stmt
        .query_map(rusqlite::params_from_iter(params), |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(releases)
}

/// 返回需要翻译但尚未翻译的 (id, body) 列表。
/// 条件：body 非空、body_translated 为空、翻译重试次数 < 5。
///
/// `excluded_types`：不参与 AI 翻译的源类型集合（如 youtube/bilibili），
/// 由 poll 编排层从 `list_adapters()` 的能力声明动态收集，与摘要路径共用同一份能力声明。
pub fn get_releases_without_translation(
    conn: &Connection,
    excluded_types: &[&str],
) -> Result<Vec<(i64, Option<String>)>, String> {
    let (exclude_sql, params) = exclusion_clause(excluded_types);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.id, r.body FROM releases r
             JOIN sources s ON s.id = r.source_id
             WHERE r.body_translated IS NULL AND r.body IS NOT NULL AND r.body != ''
               AND (r.translate_retry_count IS NULL OR r.translate_retry_count < 5){}",
            exclude_sql
        ))
        .map_err(|e| e.to_string())?;
    let releases = stmt
        .query_map(rusqlite::params_from_iter(params), |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(releases)
}

/// 返回给定 release id 中属于“不参与 AI 摘要/翻译”源类型（如 youtube）的 id 集合。
///
/// 类型集合由调用方（poll 编排层）从 `source::list_adapters()` 能力声明收集，
/// 新增源类型声明 `ai_eligible=false` 后自动生效，此处不硬编码具体类型。
pub fn ai_ineligible_release_ids(
    conn: &Connection,
    ids: &[i64],
    ineligible_types: &[&str],
) -> Result<std::collections::HashSet<i64>, String> {
    use std::collections::HashSet;
    if ids.is_empty() || ineligible_types.is_empty() {
        return Ok(HashSet::new());
    }
    let id_placeholders = vec!["?"; ids.len()].join(",");
    let type_placeholders = vec!["?"; ineligible_types.len()].join(",");
    let sql = format!(
        "SELECT r.id FROM releases r JOIN sources s ON s.id = r.source_id
         WHERE s.source_type IN ({}) AND r.id IN ({})",
        type_placeholders, id_placeholders
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
    params.extend(ineligible_types.iter().map(|t| t as &dyn rusqlite::ToSql));
    params.extend(ids.iter().map(|id| id as &dyn rusqlite::ToSql));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn set_body_translated(
    conn: &Connection,
    release_id: i64,
    translated: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE releases SET body_translated = ?1, translate_retry_count = 0 WHERE id = ?2",
        rusqlite::params![translated, release_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置 release 的 body 和 extra_metadata。
///
/// 用于 HuggingFace 源：insert_release 时 body 传空，插入成功后异步拉取 README
/// 回填 body，同时写入模型元数据 JSON（pipeline_tag/downloads/likes 等）。
pub fn set_release_body_and_metadata(
    conn: &Connection,
    release_id: i64,
    body: Option<&str>,
    extra_metadata: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE releases SET body = ?1, extra_metadata = ?2 WHERE id = ?3",
        rusqlite::params![body, extra_metadata, release_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新已存在 release 的 body + extra_metadata（按 source_id + tag_name 定位）。
///
/// 用于视频源（YouTube/B 站）轮询去重命中时刷新封面/时长/播放量等元数据：
/// insert_release 对已存在条目返回 0（拿不到 id），需按业务键定位更新；
/// 新增条目仍走 set_release_body_and_metadata。
pub fn update_release_metadata(
    conn: &Connection,
    source_id: i64,
    tag_name: &str,
    body: Option<&str>,
    extra_metadata: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE releases SET body = ?1, extra_metadata = ?2 WHERE source_id = ?3 AND tag_name = ?4",
        rusqlite::params![body, extra_metadata, source_id, tag_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn increment_translate_retry_count(conn: &Connection, release_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE releases SET translate_retry_count = COALESCE(translate_retry_count, 0) + 1 WHERE id = ?1",
        rusqlite::params![release_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_ai_summary(
    conn: &Connection,
    release_id: i64,
    summary: &str,
    importance: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE releases SET ai_summary = ?1, ai_importance = ?2, retry_count = 0 WHERE id = ?3",
        rusqlite::params![summary, importance, release_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn increment_retry_count(conn: &Connection, release_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE releases SET retry_count = COALESCE(retry_count, 0) + 1 WHERE id = ?1",
        rusqlite::params![release_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_release(conn: &Connection, release_id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM releases WHERE id = ?1", params![release_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_notification_state(
    conn: &Connection,
    release_id: i64,
    status: &str,
    snooze_until: Option<&str>,
) -> Result<(), String> {
    match status {
        "pending" | "snoozed" | "clicked" | "ignored" => {}
        _ => return Err(format!("无效的通知状态值: {}", status)),
    }
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notification_state (release_id, status, snooze_until, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(release_id) DO UPDATE SET status = ?2, snooze_until = ?3,
           last_notified_at = CASE WHEN ?2 IN ('snoozed', 'pending') THEN NULL ELSE last_notified_at END,
           updated_at = ?4",
        params![release_id, status, snooze_until, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 标记 release 已通知（H-1 修复）：改为 **upsert**，不再依赖 state 行已存在。
///
/// 此前是纯 UPDATE：当 notification_state 行缺失（历史脏数据 / 插入失败遗留）时
/// 影响 0 行却返回 Ok，`get_pending_releases` 里 `last_notified_at IS NULL` 条件
/// 永远命中，release 每轮都被重复通知。upsert 保证任何情况下都能落标记。
pub fn set_last_notified_at(conn: &Connection, release_id: i64) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notification_state (release_id, status, created_at, updated_at, last_notified_at)
         VALUES (?1, 'pending', ?2, ?2, ?2)
         ON CONFLICT(release_id) DO UPDATE SET last_notified_at = ?2, updated_at = ?2",
        rusqlite::params![release_id, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 检查某个 source 是否已有比指定 published_at 更新的版本。
/// 用于判断新保存的版本是否真的是全局最新。
pub fn has_newer_release(conn: &Connection, source_id: i64, published_at: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM releases WHERE source_id = ?1 AND published_at > ?2",
        )
        .map_err(|e| e.to_string())?;
    let count: i64 = stmt
        .query_row(rusqlite::params![source_id, published_at], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

pub fn get_release(conn: &Connection, id: i64) -> Result<Option<ReleaseInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.source_id, s.source_type, s.owner, s.repo,
                    r.tag_name, r.release_name, r.html_url, r.published_at,
                    r.prerelease, r.body, r.detected_at,
                    COALESCE(ns.status, 'pending'), ns.snooze_until,
                    r.ai_summary, r.ai_importance, r.body_translated, r.extra_metadata,
                s.description
             FROM releases r
             JOIN sources s ON r.source_id = s.id
             LEFT JOIN notification_state ns ON r.id = ns.release_id
             WHERE r.id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(ReleaseInfo {
                id: row.get(0)?,
                source_id: row.get(1)?,
                source_type: row.get(2)?,
                owner: row.get(3)?,
                repo: row.get(4)?,
                tag_name: row.get(5)?,
                release_name: row.get(6)?,
                html_url: row.get(7)?,
                published_at: row.get(8)?,
                prerelease: row.get::<_, i64>(9)? != 0,
                body: row.get(10)?,
                detected_at: row.get(11)?,
                notification_status: row.get(12)?,
                snooze_until: row.get(13)?,
                ai_summary: row.get(14)?,
                ai_importance: row.get(15)?,
                body_translated: row.get(16)?,
                extra_metadata: row.get(17)?,
                source_description: row.get(18)?,
            })
        })
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok(release)) => Ok(Some(release)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

pub fn get_releases_with_state(conn: &Connection) -> Result<Vec<ReleaseInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.source_id, s.source_type, s.owner, s.repo,
                    r.tag_name, r.release_name, r.html_url, r.published_at,
                    r.prerelease, r.body, r.detected_at,
                    COALESCE(ns.status, 'pending'), ns.snooze_until,
                    r.ai_summary, r.ai_importance, r.body_translated, r.extra_metadata,
                s.description
             FROM releases r
             JOIN sources s ON r.source_id = s.id
             LEFT JOIN notification_state ns ON r.id = ns.release_id
             ORDER BY CASE
                 -- published_at 异常（0 时间戳/1970 脏数据）时按检测时间兑底，
                 -- 避免被 LIMIT 截断后完全不可见（如 bilibili pub_ts 解析失败历史数据）
                 WHEN r.published_at LIKE '1970%' THEN r.detected_at
                 ELSE r.published_at
             END DESC
             LIMIT 200",
        )
        .map_err(|e| e.to_string())?;

    let releases = stmt
        .query_map([], |row| {
            Ok(ReleaseInfo {
                id: row.get(0)?,
                source_id: row.get(1)?,
                source_type: row.get(2)?,
                owner: row.get(3)?,
                repo: row.get(4)?,
                tag_name: row.get(5)?,
                release_name: row.get(6)?,
                html_url: row.get(7)?,
                published_at: row.get(8)?,
                prerelease: row.get::<_, i64>(9)? != 0,
                body: row.get(10)?,
                detected_at: row.get(11)?,
                notification_status: row.get(12)?,
                snooze_until: row.get(13)?,
                ai_summary: row.get(14)?,
                ai_importance: row.get(15)?,
                body_translated: row.get(16)?,
                extra_metadata: row.get(17)?,
                source_description: row.get(18)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(releases)
}

/// 大→3, 中→2, 小→1. Returns true if `a` >= `b` in importance.
/// Unknown/malformed values are treated as "小" (lowest).
fn importance_ge(a: &str, b: &str) -> bool {
    let a_val = match a {
        "大" => 3,
        "中" => 2,
        _ => 1,
    };
    let b_val = match b {
        "大" => 3,
        "中" => 2,
        _ => 1,
    };
    a_val >= b_val
}

fn is_release_due(status: &str, snooze_until: Option<&str>) -> bool {
    if status == "snoozed" {
        if let Some(until) = snooze_until {
            if !until.is_empty() {
                if let Ok(until_time) = chrono::DateTime::parse_from_rfc3339(until) {
                    if until_time >= chrono::Utc::now() {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn query_unread_releases(conn: &Connection, only_notified_missing: bool) -> Result<Vec<ReleaseInfo>, String> {
    let notified_filter = if only_notified_missing { " AND ns.last_notified_at IS NULL" } else { "" };
    let sql = format!(
        "SELECT r.id, r.source_id, s.source_type, s.owner, s.repo,
                r.tag_name, r.release_name, r.html_url, r.published_at,
                r.prerelease, r.body, r.detected_at,
                COALESCE(ns.status, 'pending'), ns.snooze_until,
                r.ai_summary, r.ai_importance, r.body_translated, r.extra_metadata,
                s.description
         FROM releases r
         JOIN sources s ON r.source_id = s.id
         LEFT JOIN notification_state ns ON r.id = ns.release_id
         WHERE COALESCE(ns.status, 'pending') IN ('pending', 'snoozed'){}
         ORDER BY r.detected_at DESC",
        notified_filter,
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    let releases = stmt
        .query_map([], |row| {
            Ok(ReleaseInfo {
                id: row.get(0)?,
                source_id: row.get(1)?,
                source_type: row.get(2)?,
                owner: row.get(3)?,
                repo: row.get(4)?,
                tag_name: row.get(5)?,
                release_name: row.get(6)?,
                html_url: row.get(7)?,
                published_at: row.get(8)?,
                prerelease: row.get::<_, i64>(9)? != 0,
                body: row.get(10)?,
                detected_at: row.get(11)?,
                notification_status: row.get(12)?,
                snooze_until: row.get(13)?,
                ai_summary: row.get(14)?,
                ai_importance: row.get(15)?,
                body_translated: row.get(16)?,
                extra_metadata: row.get(17)?,
                source_description: row.get(18)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(releases
        .into_iter()
        .filter(|r| is_release_due(&r.notification_status, r.snooze_until.as_deref()))
        .collect())
}

/// 返回当前仍处于未读状态的版本。
///
/// 该查询用于 UI/托盘红点等“未读”语义：只看通知状态是否仍为 pending，
/// 或 snoozed 且已到提醒时间；不会因为已经发送过系统通知而排除。
pub fn get_unread_releases(conn: &Connection) -> Result<Vec<ReleaseInfo>, String> {
    query_unread_releases(conn, false)
}

pub fn get_pending_releases(conn: &Connection) -> Result<Vec<ReleaseInfo>, String> {
    let min_importance = get_setting_str(conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE)
        .unwrap_or_else(|_| DEFAULT_DEEPSEEK_MIN_IMPORTANCE.to_string());

    let pending: Vec<ReleaseInfo> = query_unread_releases(conn, true)?
        .into_iter()
        .filter(|r| {
            // `ai_importance IS NULL` 始终通知
            if let Some(ref imp) = r.ai_importance {
                if !importance_ge(imp, &min_importance) {
                    return false;
                }
            }
            true
        })
        .collect();

    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sources;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_release_insert() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid > 0);
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].notification_status, "pending");
    }

    #[test]
    fn test_notification_state_ignored() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "t", "r", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        set_notification_state(&conn, rid, "ignored", None).unwrap();
        assert_eq!(get_pending_releases(&conn).unwrap().len(), 0);
    }

    #[test]
    fn test_insert_release_duplicate() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid1 = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid1 > 0);
        let rid2 = insert_release(&conn, sid, "v1.0", "R2", "https://y", "2024-02-02T00:00:00Z", false, None).unwrap();
        assert_eq!(rid2, 0);
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].release_name, "R1");
    }

    #[test]
    fn test_update_release_metadata_refreshes_existing() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "bilibili", "476599099", "", "").unwrap();
        // 首次插入
        let rid = insert_release(&conn, sid, "BV1xx", "T", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid > 0);
        // 轮询去重命中（返回 0）：按 source_id + tag_name 刷新元数据
        let dup = insert_release(&conn, sid, "BV1xx", "T", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert_eq!(dup, 0);
        update_release_metadata(&conn, sid, "BV1xx", Some("简介"), Some(r#"{"kind":"video","view_count":123456}"#)).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].body.as_deref(), Some("简介"));
        assert_eq!(
            releases[0].extra_metadata.as_deref(),
            Some(r#"{"kind":"video","view_count":123456}"#)
        );
        // 其它 source_id 的同名 tag 不受影响
        let sid2 = sources::add_source(&conn, "bilibili", "888888", "", "").unwrap();
        update_release_metadata(&conn, sid2, "BV1xx", Some("别的"), None).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].body.as_deref(), Some("简介"), "其它源的同名 tag 不应被误更新");
    }

    #[test]
    fn test_insert_release_prerelease_and_body() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", true, Some("release body")).unwrap();
        assert!(rid > 0);
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert!(releases[0].prerelease);
        assert_eq!(releases[0].body.as_deref(), Some("release body"));
    }

    #[test]
    fn test_pending_releases_snooze_boundaries() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();

        let rid1 = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid1 > 0);

        let rid2 = insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();
        conn.execute("DELETE FROM notification_state WHERE release_id = ?1", rusqlite::params![rid2]).unwrap();

        let rid3 = insert_release(&conn, sid, "v3.0", "R3", "https://x", "2024-01-03T00:00:00Z", false, None).unwrap();
        set_notification_state(&conn, rid3, "snoozed", Some("")).unwrap();

        let rid4 = insert_release(&conn, sid, "v4.0", "R4", "https://x", "2024-01-04T00:00:00Z", false, None).unwrap();
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        set_notification_state(&conn, rid4, "snoozed", Some(&past.to_rfc3339())).unwrap();

        let rid5 = insert_release(&conn, sid, "v5.0", "R5", "https://x", "2024-01-05T00:00:00Z", false, None).unwrap();
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        set_notification_state(&conn, rid5, "snoozed", Some(&future.to_rfc3339())).unwrap();

        let rid6 = insert_release(&conn, sid, "v6.0", "R6", "https://x", "2024-01-06T00:00:00Z", false, None).unwrap();
        set_notification_state(&conn, rid6, "ignored", None).unwrap();

        let pending = get_pending_releases(&conn).unwrap();
        let pending_ids: Vec<i64> = pending.iter().map(|r| r.id).collect();

        assert!(pending_ids.contains(&rid1), "pending status should appear");
        assert!(pending_ids.contains(&rid2), "COALESCE NULL should appear as pending");
        assert!(pending_ids.contains(&rid3), "snoozed with empty snooze_until should appear");
        assert!(pending_ids.contains(&rid4), "snoozed with expired snooze_until should appear");
        assert!(!pending_ids.contains(&rid5), "snoozed with future snooze_until should not appear");
        assert!(!pending_ids.contains(&rid6), "ignored should not appear");
    }

    #[test]
    fn test_set_last_notified_at_upserts_when_state_missing() {
        // H-1 修复验证：state 行缺失（历史脏数据/插入失败遗留）时，
        // set_last_notified_at 必须 upsert 落标记，否则 release 每轮都被重复通知
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid > 0);

        // 模拟 state 行缺失（如旧版本 insert 第二语句失败遗留）
        conn.execute("DELETE FROM notification_state WHERE release_id = ?1", rusqlite::params![rid]).unwrap();
        assert_eq!(get_pending_releases(&conn).unwrap().len(), 1, "COALESCE 应把缺失行视为 pending");

        // 标记已通知：upsert 应补建 state 行并写入 last_notified_at
        set_last_notified_at(&conn, rid).unwrap();
        assert_eq!(
            get_pending_releases(&conn).unwrap().len(),
            0,
            "upsert 后不应再被选为待通知（重复通知循环应被切断）"
        );
    }

    #[test]
    fn test_insert_release_transaction_creates_state_row() {
        // H-1 修复验证：insert_release 两语句在同一事务内，成功时必带 state 行
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid > 0);
        let state_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notification_state WHERE release_id = ?1",
                rusqlite::params![rid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state_count, 1, "事务内应保证 release 与 state 行同时存在");
    }

    #[test]
    fn test_set_notification_state_upsert() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "pending");

        set_notification_state(&conn, rid, "ignored", None).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "ignored");

        let future = chrono::Utc::now() + chrono::Duration::hours(2);
        set_notification_state(&conn, rid, "snoozed", Some(&future.to_rfc3339())).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "snoozed");
        assert!(releases[0].snooze_until.is_some());

        set_notification_state(&conn, rid, "pending", None).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "pending");
        assert!(releases[0].snooze_until.is_none());
    }

    #[test]
    fn test_get_releases_coalesce_null() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        conn.execute("DELETE FROM notification_state WHERE release_id = ?1", rusqlite::params![rid]).unwrap();

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].notification_status, "pending");
        assert!(releases[0].snooze_until.is_none());
    }

    #[test]
    fn test_ai_summary_store_and_retrieve() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, Some("body")).unwrap();
        assert!(rid > 0);

        set_ai_summary(&conn, rid, "该版本新增了重要功能", "大").unwrap();

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].ai_summary.as_deref(), Some("该版本新增了重要功能"));
        assert_eq!(releases[0].ai_importance.as_deref(), Some("大"));

        let pending = get_pending_releases(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].ai_summary.as_deref(), Some("该版本新增了重要功能"));
        assert_eq!(pending[0].ai_importance.as_deref(), Some("大"));
    }

    #[test]
    fn test_unread_releases_include_notified_pending() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        set_last_notified_at(&conn, rid).unwrap();

        assert_eq!(
            get_pending_releases(&conn).unwrap().len(),
            0,
            "notified release should not be selected for another notification"
        );
        assert_eq!(
            get_unread_releases(&conn).unwrap().len(),
            1,
            "notified but unclicked release should still count as unread for badge"
        );
    }

    #[test]
    fn test_unread_releases_respect_snooze_until() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();

        let expired_id = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        set_notification_state(&conn, expired_id, "snoozed", Some(&past.to_rfc3339())).unwrap();

        let future_id = insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        set_notification_state(&conn, future_id, "snoozed", Some(&future.to_rfc3339())).unwrap();

        let unread_ids: Vec<i64> = get_unread_releases(&conn).unwrap().into_iter().map(|r| r.id).collect();

        assert!(unread_ids.contains(&expired_id), "expired snooze should count as unread");
        assert!(!unread_ids.contains(&future_id), "future snooze should not count as unread yet");
    }

    #[test]
    fn test_pending_after_notified_then_reset() {
        // Bug #3 修复验证：将已通知的 release 从 clicked 改回 pending，
        // last_notified_at 应被清空，release 应重新出现在 pending 列表中
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(
            &conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None,
        ).unwrap();

        // 初始状态：pending，应在列表中
        assert_eq!(get_pending_releases(&conn).unwrap().len(), 1);

        // 模拟通知发送：设置 last_notified_at
        set_last_notified_at(&conn, rid).unwrap();

        // 用户点击通知：标记为 clicked
        set_notification_state(&conn, rid, "clicked", None).unwrap();
        assert_eq!(get_pending_releases(&conn).unwrap().len(), 0);

        // 用户重新标记为 pending：应清空 last_notified_at 并重新出现在列表中
        set_notification_state(&conn, rid, "pending", None).unwrap();
        assert_eq!(
            get_pending_releases(&conn).unwrap().len(),
            1,
            "reset to pending should clear last_notified_at and re-include in pending"
        );
    }

    #[test]
    fn test_pending_after_ignored_then_reset() {
        // Bug #3 修复验证：将已忽略的 release 改回 pending，
        // last_notified_at 应被清空
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(
            &conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None,
        ).unwrap();

        // 模拟通知发送 + 忽略
        set_last_notified_at(&conn, rid).unwrap();
        set_notification_state(&conn, rid, "ignored", None).unwrap();
        assert_eq!(get_pending_releases(&conn).unwrap().len(), 0);

        // 重新标记为 pending：应可重新通知
        set_notification_state(&conn, rid, "pending", None).unwrap();
        assert_eq!(
            get_pending_releases(&conn).unwrap().len(),
            1,
            "reset from ignored to pending should clear last_notified_at"
        );
    }

    #[test]
    fn test_ai_summary_null_by_default() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid > 0);

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert!(releases[0].ai_summary.is_none());
        assert!(releases[0].ai_importance.is_none());
        assert!(releases[0].body_translated.is_none());
    }

    #[test]
    fn test_body_translated_store_and_retrieve() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, Some("release body")).unwrap();
        assert!(rid > 0);

        // 有 body 且未翻译 → 出现在待翻译列表
        let pending = get_releases_without_translation(&conn, &[]).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, rid);

        set_body_translated(&conn, rid, "发布说明译文").unwrap();

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].body_translated.as_deref(), Some("发布说明译文"));
        // 翻译完成后不再出现在待翻译列表
        assert!(get_releases_without_translation(&conn, &[]).unwrap().is_empty());
    }

    #[test]
    fn test_get_releases_without_translation_skips_empty_body() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        // body 为空 → 不应进入待翻译列表
        let _rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(get_releases_without_translation(&conn, &[]).unwrap().is_empty());

        // body 非空 → 进入待翻译列表
        let rid2 = insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, Some("body")).unwrap();
        let pending = get_releases_without_translation(&conn, &[]).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, rid2);
    }

    #[test]
    fn test_translate_retry_count_excludes_after_limit() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, Some("body")).unwrap();

        // 重试 5 次后不再出现在待翻译列表
        for _ in 0..5 {
            increment_translate_retry_count(&conn, rid).unwrap();
        }
        assert!(get_releases_without_translation(&conn, &[]).unwrap().is_empty());

        // set_body_translated 会重置 retry_count
        set_body_translated(&conn, rid, "译文").unwrap();
        let count: i64 = conn
            .query_row("SELECT translate_retry_count FROM releases WHERE id=?1", rusqlite::params![rid], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_importance_ge_da_ge_da() {
        assert!(importance_ge("大", "大"));
    }

    #[test]
    fn test_importance_ge_da_ge_zhong() {
        assert!(importance_ge("大", "中"));
    }

    #[test]
    fn test_importance_ge_da_ge_xiao() {
        assert!(importance_ge("大", "小"));
    }

    #[test]
    fn test_importance_ge_zhong_lt_da() {
        assert!(!importance_ge("中", "大"));
    }

    #[test]
    fn test_importance_ge_zhong_ge_zhong() {
        assert!(importance_ge("中", "中"));
    }

    #[test]
    fn test_importance_ge_zhong_ge_xiao() {
        assert!(importance_ge("中", "小"));
    }

    #[test]
    fn test_importance_ge_xiao_lt_da() {
        assert!(!importance_ge("小", "大"));
    }

    #[test]
    fn test_importance_ge_xiao_lt_zhong() {
        assert!(!importance_ge("小", "中"));
    }

    #[test]
    fn test_importance_ge_xiao_ge_xiao() {
        assert!(importance_ge("小", "小"));
    }

    #[test]
    fn test_importance_ge_unknown_falls_back_to_xiao() {
        assert!(importance_ge("未知", "小"));
        assert!(!importance_ge("未知", "中"));
    }

    // ── ai_eligible=false 源类型跳过 AI 摘要/翻译（youtube + bilibili）──

    /// 同时插入 github / youtube / bilibili 源各一条带 body 的 release，
    /// 验证摘要/翻译候选列表排除 youtube 与 bilibili 源。
    fn seed_gh_yt_bili_releases(conn: &rusqlite::Connection) -> (i64, i64, i64) {
        let gh = sources::add_source(conn, "github", "o", "r", "").unwrap();
        let yt = sources::add_source(conn, "youtube", "UCabc123", "", "").unwrap();
        let bl = sources::add_source(conn, "bilibili", "476599099", "", "").unwrap();
        let gh_id = insert_release(conn, gh, "v1", "R", "https://x", "2024-01-01T00:00:00Z", false, Some("gh body")).unwrap();
        let yt_id = insert_release(conn, yt, "vid1", "V", "https://y", "2024-01-02T00:00:00Z", false, Some("yt body")).unwrap();
        let bl_id = insert_release(conn, bl, "BV1a1b2c3d4e5f", "V", "https://b", "2024-01-03T00:00:00Z", false, Some("bili body")).unwrap();
        (gh_id, yt_id, bl_id)
    }

    #[test]
    fn test_get_releases_without_summary_excludes_ineligible_types() {
        let conn = init_memory_db().unwrap();
        let (gh_id, _yt_id, _bl_id) = seed_gh_yt_bili_releases(&conn);
        // 与 poll 编排层一致：排除集合由 list_adapters 的 ai_eligible() 动态收集
        let excluded = ["youtube", "bilibili"];
        let pending = get_releases_without_summary(&conn, &excluded).unwrap();
        assert_eq!(pending.len(), 1, "youtube/bilibili 源不应进入待摘要列表");
        assert_eq!(pending[0].0, gh_id);
    }

    #[test]
    fn test_get_releases_without_translation_excludes_ineligible_types() {
        let conn = init_memory_db().unwrap();
        let (gh_id, _yt_id, _bl_id) = seed_gh_yt_bili_releases(&conn);
        let excluded = ["youtube", "bilibili"];
        let pending = get_releases_without_translation(&conn, &excluded).unwrap();
        assert_eq!(pending.len(), 1, "youtube/bilibili 源不应进入待翻译列表");
        assert_eq!(pending[0].0, gh_id);
    }

    #[test]
    fn test_get_releases_without_summary_empty_exclusion_keeps_all() {
        let conn = init_memory_db().unwrap();
        let (gh_id, yt_id, bl_id) = seed_gh_yt_bili_releases(&conn);
        // 排除集合为空（未收集到能力声明）时不附加 NOT IN 条件，全部进入候选
        let pending = get_releases_without_summary(&conn, &[]).unwrap();
        let ids: Vec<i64> = pending.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&gh_id) && ids.contains(&yt_id) && ids.contains(&bl_id));
    }

    /// 旧辅助：只关心 github/youtube 的测试仍可用（内部复用新 seed）。
    fn seed_gh_and_yt_releases(conn: &rusqlite::Connection) -> (i64, i64) {
        let (gh_id, yt_id, _) = seed_gh_yt_bili_releases(conn);
        (gh_id, yt_id)
    }

    #[test]
    fn test_ai_ineligible_release_ids_returns_only_ineligible() {
        let conn = init_memory_db().unwrap();
        let (gh_id, yt_id) = seed_gh_and_yt_releases(&conn);
        let ids = ai_ineligible_release_ids(&conn, &[gh_id, yt_id], &["youtube"]).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&yt_id), "只应包含 youtube 源的 release id");
    }

    #[test]
    fn test_ai_ineligible_release_ids_empty_input() {
        let conn = init_memory_db().unwrap();
        assert!(ai_ineligible_release_ids(&conn, &[], &["youtube"])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_ai_ineligible_release_ids_empty_types_skips() {
        let conn = init_memory_db().unwrap();
        let (gh_id, _) = seed_gh_and_yt_releases(&conn);
        // 无排除类型 → 直接返回空集（不生成无效 IN () SQL）
        assert!(ai_ineligible_release_ids(&conn, &[gh_id], &[]).unwrap().is_empty());
    }

    #[test]
    fn test_ai_ineligible_release_ids_other_type_returns_only_matched() {
        let conn = init_memory_db().unwrap();
        let (gh_id, yt_id) = seed_gh_and_yt_releases(&conn);
        // 传入的排除类型为 github 时，只返回 github 源 id
        let ids = ai_ineligible_release_ids(&conn, &[gh_id, yt_id], &["github"]).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&gh_id));
    }

    #[test]
    fn test_ai_ineligible_release_ids_non_ineligible_only_empty() {
        let conn = init_memory_db().unwrap();
        let (gh_id, _) = seed_gh_and_yt_releases(&conn);
        let ids = ai_ineligible_release_ids(&conn, &[gh_id], &["youtube"]).unwrap();
        assert!(ids.is_empty(), "github 源不应被标记为排除类型");
    }
}
