use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db::settings::{get_setting_str, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE};

#[derive(Debug, Serialize, Deserialize, Clone)]
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
}

#[allow(clippy::too_many_arguments)]
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
    conn.execute(
        "INSERT OR IGNORE INTO releases (source_id, tag_name, release_name, html_url, published_at, prerelease, body, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![source_id, tag_name, release_name, html_url, published_at, prerelease as i64, body, now],
    )
    .map_err(|e| e.to_string())?;

    if conn.changes() == 0 {
        return Ok(0);
    }

    let release_id = conn.last_insert_rowid();

    if release_id > 0 {
        conn.execute(
            "INSERT OR IGNORE INTO notification_state (release_id, status, created_at, updated_at)
             VALUES (?1, 'pending', ?2, ?2)",
            params![release_id, now],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(release_id)
}

/// Returns (id, body) tuples for releases where AI summary generation is missing.
/// Used by the poll cycle to retry failed summaries.
pub fn get_releases_without_summary(
    conn: &Connection,
) -> Result<Vec<(i64, Option<String>)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, body FROM releases WHERE ai_summary IS NULL AND body IS NOT NULL AND body != '' AND (retry_count IS NULL OR retry_count < 5)",
        )
        .map_err(|e| e.to_string())?;

    let releases = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(releases)
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
           last_notified_at = CASE WHEN ?2 = 'snoozed' THEN NULL ELSE last_notified_at END,
           updated_at = ?4",
        params![release_id, status, snooze_until, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_last_notified_at(conn: &Connection, release_id: i64) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE notification_state SET last_notified_at = ?1, updated_at = ?1 WHERE release_id = ?2",
        rusqlite::params![now, release_id],
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
                    r.ai_summary, r.ai_importance
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
                    r.ai_summary, r.ai_importance
             FROM releases r
             JOIN sources s ON r.source_id = s.id
             LEFT JOIN notification_state ns ON r.id = ns.release_id
             ORDER BY r.published_at DESC
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

pub fn get_pending_releases(conn: &Connection) -> Result<Vec<ReleaseInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.source_id, s.source_type, s.owner, s.repo,
                    r.tag_name, r.release_name, r.html_url, r.published_at,
                    r.prerelease, r.body, r.detected_at,
                    COALESCE(ns.status, 'pending'), ns.snooze_until,
                    r.ai_summary, r.ai_importance
             FROM releases r
             JOIN sources s ON r.source_id = s.id
             LEFT JOIN notification_state ns ON r.id = ns.release_id
             WHERE COALESCE(ns.status, 'pending') IN ('pending', 'snoozed')
               AND ns.last_notified_at IS NULL
             ORDER BY r.detected_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now();
    let min_importance = get_setting_str(conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE)
        .unwrap_or_else(|_| DEFAULT_DEEPSEEK_MIN_IMPORTANCE.to_string());

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
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let pending: Vec<ReleaseInfo> = releases
        .into_iter()
        .filter(|r| {
            if r.notification_status == "ignored" {
                return false;
            }
            if r.notification_status == "snoozed" {
                if let Some(ref until) = r.snooze_until {
                    if !until.is_empty() {
                        if let Ok(until_time) = chrono::DateTime::parse_from_rfc3339(until) {
                            if until_time >= now {
                                return false;
                            }
                        }
                    }
                }
            }
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
    fn test_ai_summary_null_by_default() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        assert!(rid > 0);

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert!(releases[0].ai_summary.is_none());
        assert!(releases[0].ai_importance.is_none());
    }
}
