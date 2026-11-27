use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::settings::{get_setting_str, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE};
use crate::domain::release_policy::{bump_between, importance_ge, is_release_due, parse_semver};
use crate::error::{DbError, DbResult};

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
    /// 用户旗标：0 = 未标记，1-6 = 预设颜色（红/橙/黄/绿/蓝/紫），语义由用户自行赋予。
    pub flag: i64,
    /// 相对同 source 上一版本（按 published_at）的 semver 变化类型：major/minor/patch。
    /// 无 semver tag 的源（YouTube/B 站等）或无法比较（相等/回落）时为 NULL。
    pub version_bump: Option<String>,
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
) -> DbResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    // releases 与 notification_state 在同一事务内写入，避免遗留无状态行
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO releases (source_id, tag_name, release_name, html_url, published_at, prerelease, body, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![source_id, tag_name, release_name, html_url, published_at, prerelease as i64, body, now],
    )?;

    if tx.changes() == 0 {
        return Ok(0);
    }

    let release_id = tx.last_insert_rowid();

    if release_id > 0 {
        tx.execute(
            "INSERT OR IGNORE INTO notification_state (release_id, status, created_at, updated_at)
             VALUES (?1, 'pending', ?2, ?2)",
            params![release_id, now],
        )?;
    }

    tx.commit()?;
    Ok(release_id)
}

pub fn recompute_version_bumps(conn: &Connection, source_id: i64) -> DbResult<()> {
    let mut stmt = conn.prepare(
        "SELECT id, tag_name, version_bump FROM releases
         WHERE source_id = ?1
         ORDER BY published_at ASC, id ASC",
    )?;
    let rows: Vec<(i64, String, Option<String>)> = stmt
        .query_map(params![source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut prev: Option<(u64, u64, u64)> = None;
    for (id, tag, existing) in rows {
        let cur = parse_semver(&tag);
        let bump = match (prev, cur) {
            (Some(p), Some(c)) => bump_between(p, c),
            _ => None,
        };
        if existing.as_deref() != bump {
            conn.execute(
                "UPDATE releases SET version_bump = ?1 WHERE id = ?2",
                params![bump, id],
            )?;
        }
        if cur.is_some() {
            prev = cur;
        }
    }
    Ok(())
}

pub fn set_release_flag(conn: &Connection, release_id: i64, flag: i64) -> DbResult<()> {
    if !(0..=6).contains(&flag) {
        return Err(DbError::validation_args(
            "err.release_flag_invalid",
            [flag.to_string()],
        ));
    }
    conn.execute(
        "UPDATE releases SET flag = ?1 WHERE id = ?2",
        params![flag, release_id],
    )?;
    Ok(())
}

pub fn batch_set_release_flag(conn: &Connection, release_ids: &[i64], flag: i64) -> DbResult<u32> {
    if !(0..=6).contains(&flag) {
        return Err(DbError::validation_args(
            "err.release_flag_invalid",
            [flag.to_string()],
        ));
    }
    let ids = super::dedup_ids(release_ids);
    if ids.is_empty() {
        return Ok(0);
    }
    let mut total = 0usize;
    for chunk in ids.chunks(super::BATCH_CHUNK_SIZE) {
        let placeholders = super::in_placeholders(chunk.len(), 2);
        let sql = format!("UPDATE releases SET flag = ?1 WHERE id IN ({placeholders})");
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(chunk.len() + 1);
        params_vec.push(Box::new(flag));
        for &id in chunk {
            params_vec.push(Box::new(id));
        }
        let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        total += conn.execute(&sql, params_ref.as_slice())?;
    }
    Ok(total as u32)
}

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

pub fn get_releases_without_summary(
    conn: &Connection,
    excluded_types: &[&str],
) -> DbResult<Vec<(i64, Option<String>)>> {
    let (exclude_sql, params) = exclusion_clause(excluded_types);
    let mut stmt = conn.prepare(&format!(
        "SELECT r.id, r.body FROM releases r
         JOIN sources s ON s.id = r.source_id
         WHERE r.ai_summary IS NULL AND r.body IS NOT NULL AND r.body != ''
           AND (r.retry_count IS NULL OR r.retry_count < 5){}",
        exclude_sql
    ))?;

    let releases = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(releases)
}

pub fn get_releases_without_translation(
    conn: &Connection,
    excluded_types: &[&str],
) -> DbResult<Vec<(i64, Option<String>)>> {
    let (exclude_sql, params) = exclusion_clause(excluded_types);
    let mut stmt = conn.prepare(&format!(
        "SELECT r.id, r.body FROM releases r
         JOIN sources s ON s.id = r.source_id
         WHERE r.body_translated IS NULL AND r.body IS NOT NULL AND r.body != ''
           AND (r.translate_retry_count IS NULL OR r.translate_retry_count < 5){}",
        exclude_sql
    ))?;
    let releases = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(releases)
}

pub fn ai_ineligible_release_ids(
    conn: &Connection,
    ids: &[i64],
    ineligible_types: &[&str],
) -> DbResult<std::collections::HashSet<i64>> {
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
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| row.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

pub fn set_body_translated(
    conn: &Connection,
    release_id: i64,
    translated: &str,
) -> DbResult<()> {
    conn.execute(
        "UPDATE releases SET body_translated = ?1, translate_retry_count = 0 WHERE id = ?2",
        rusqlite::params![translated, release_id],
    )?;
    Ok(())
}

pub fn set_release_body_and_metadata(
    conn: &Connection,
    release_id: i64,
    body: Option<&str>,
    extra_metadata: Option<&str>,
) -> DbResult<()> {
    conn.execute(
        "UPDATE releases SET body = ?1, extra_metadata = ?2 WHERE id = ?3",
        rusqlite::params![body, extra_metadata, release_id],
    )?;
    Ok(())
}

pub fn update_release_metadata(
    conn: &Connection,
    source_id: i64,
    tag_name: &str,
    body: Option<&str>,
    extra_metadata: Option<&str>,
) -> DbResult<()> {
    conn.execute(
        "UPDATE releases SET body = ?1, extra_metadata = ?2 WHERE source_id = ?3 AND tag_name = ?4",
        rusqlite::params![body, extra_metadata, source_id, tag_name],
    )?;
    Ok(())
}

pub fn increment_translate_retry_count(conn: &Connection, release_id: i64) -> DbResult<()> {
    conn.execute(
        "UPDATE releases SET translate_retry_count = COALESCE(translate_retry_count, 0) + 1 WHERE id = ?1",
        rusqlite::params![release_id],
    )?;
    Ok(())
}

pub fn set_ai_summary(
    conn: &Connection,
    release_id: i64,
    summary: &str,
    importance: &str,
) -> DbResult<()> {
    conn.execute(
        "UPDATE releases SET ai_summary = ?1, ai_importance = ?2, retry_count = 0 WHERE id = ?3",
        rusqlite::params![summary, importance, release_id],
    )?;
    Ok(())
}

pub fn increment_retry_count(conn: &Connection, release_id: i64) -> DbResult<()> {
    conn.execute(
        "UPDATE releases SET retry_count = COALESCE(retry_count, 0) + 1 WHERE id = ?1",
        rusqlite::params![release_id],
    )?;
    Ok(())
}

pub fn delete_release(conn: &Connection, release_id: i64) -> DbResult<()> {
    conn.execute("DELETE FROM releases WHERE id = ?1", params![release_id])?;
    Ok(())
}

pub fn batch_delete_releases(conn: &Connection, release_ids: &[i64]) -> DbResult<u32> {
    let ids = super::dedup_ids(release_ids);
    if ids.is_empty() {
        return Ok(0);
    }
    let mut total = 0usize;
    for chunk in ids.chunks(super::BATCH_CHUNK_SIZE) {
        let placeholders = super::in_placeholders(chunk.len(), 1);
        let sql = format!("DELETE FROM releases WHERE id IN ({placeholders})");
        let params_ref: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        total += conn.execute(&sql, params_ref.as_slice())?;
    }
    Ok(total as u32)
}

pub fn set_notification_state(
    conn: &Connection,
    release_id: i64,
    status: &str,
    snooze_until: Option<&str>,
) -> DbResult<()> {
    match status {
        "pending" | "snoozed" | "clicked" | "ignored" => {}
        _ => {
            return Err(DbError::validation_args(
                "err.invalid_notification_status",
                [status],
            ));
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notification_state (release_id, status, snooze_until, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(release_id) DO UPDATE SET status = ?2, snooze_until = ?3,
           last_notified_at = CASE WHEN ?2 IN ('snoozed', 'pending') THEN NULL ELSE last_notified_at END,
           notify_failures = CASE WHEN ?2 IN ('snoozed', 'pending') THEN 0 ELSE notify_failures END,
           updated_at = ?4",
        params![release_id, status, snooze_until, now],
    )?;
    Ok(())
}

pub fn batch_set_notification_state(
    conn: &Connection,
    release_ids: &[i64],
    status: &str,
    snooze_until: Option<&str>,
) -> DbResult<u32> {
    match status {
        "pending" | "snoozed" | "clicked" | "ignored" => {}
        _ => {
            return Err(DbError::validation_args(
                "err.invalid_notification_status",
                [status],
            ));
        }
    }
    let ids = super::dedup_ids(release_ids);
    if ids.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut total = 0usize;
    for chunk in ids.chunks(super::BATCH_CHUNK_SIZE) {
        let placeholders = super::in_placeholders(chunk.len(), 4);
        let sql = format!(
            "INSERT INTO notification_state (release_id, status, snooze_until, created_at, updated_at)
             SELECT id, ?1, ?2, ?3, ?3 FROM releases WHERE id IN ({placeholders})
             ON CONFLICT(release_id) DO UPDATE SET status = ?1, snooze_until = ?2,
               last_notified_at = CASE WHEN ?1 IN ('snoozed', 'pending') THEN NULL ELSE last_notified_at END,
               notify_failures = CASE WHEN ?1 IN ('snoozed', 'pending') THEN 0 ELSE notify_failures END,
               updated_at = ?3"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(chunk.len() + 3);
        params_vec.push(Box::new(status.to_string()));
        params_vec.push(Box::new(snooze_until.map(|s| s.to_string())));
        params_vec.push(Box::new(now.clone()));
        for &id in chunk {
            params_vec.push(Box::new(id));
        }
        let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        total += conn.execute(&sql, params_ref.as_slice())?;
    }
    Ok(total as u32)
}

pub fn mark_source_releases_read(conn: &Connection, source_id: i64) -> DbResult<u32> {
    let mut stmt = conn.prepare(
        "SELECT r.id, COALESCE(ns.status, 'pending'), ns.snooze_until
         FROM releases r
         LEFT JOIN notification_state ns ON r.id = ns.release_id
         WHERE r.source_id = ?1 AND COALESCE(ns.status, 'pending') IN ('pending', 'snoozed')",
    )?;
    let due_ids: Vec<i64> = stmt
        .query_map(params![source_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, status, snooze_until)| is_release_due(status, snooze_until.as_deref()))
        .map(|(id, _, _)| id)
        .collect();
    drop(stmt);
    batch_set_notification_state(conn, &due_ids, "clicked", None)
}

pub fn set_last_notified_at(conn: &Connection, release_id: i64) -> DbResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notification_state (release_id, status, created_at, updated_at, last_notified_at)
         VALUES (?1, 'pending', ?2, ?2, ?2)
         ON CONFLICT(release_id) DO UPDATE SET last_notified_at = ?2, updated_at = ?2",
        rusqlite::params![release_id, now],
    )?;
    Ok(())
}

pub fn record_notify_failure(conn: &Connection, release_id: i64) -> DbResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notification_state (release_id, status, created_at, updated_at, notify_failures)
         VALUES (?1, 'pending', ?2, ?2, 1)
         ON CONFLICT(release_id) DO UPDATE SET notify_failures = notify_failures + 1, updated_at = ?2",
        rusqlite::params![release_id, now],
    )?;
    Ok(conn.query_row(
        "SELECT notify_failures FROM notification_state WHERE release_id = ?1",
        rusqlite::params![release_id],
        |r| r.get(0),
    )?)
}

pub fn clear_last_notified_at(conn: &Connection, release_id: i64) -> DbResult<()> {
    conn.execute(
        "UPDATE notification_state SET last_notified_at = NULL, updated_at = ?2 WHERE release_id = ?1",
        rusqlite::params![release_id, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// 必须用 julianday() 归一化时间再比较：各适配器写入的 ISO-8601 后缀格式不统一，直接按字符串比较会误判。
pub fn has_newer_release(conn: &Connection, source_id: i64, published_at: &str) -> DbResult<bool> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM releases
         WHERE source_id = ?1 AND julianday(published_at) > julianday(?2)",
    )?;
    let count: i64 = stmt.query_row(rusqlite::params![source_id, published_at], |row| row.get(0))?;
    Ok(count > 0)
}

fn release_select_from(body_expr: &str, tr_expr: &str) -> String {
    format!(
        "SELECT r.id, r.source_id, s.source_type, s.owner, s.repo,
                r.tag_name, r.release_name, r.html_url, r.published_at,
                r.prerelease, {body_expr} AS body, r.detected_at,
                COALESCE(ns.status, 'pending'), ns.snooze_until,
                r.ai_summary, r.ai_importance, {tr_expr} AS body_translated, r.extra_metadata,
                s.description, r.flag, r.version_bump
         FROM releases r
         JOIN sources s ON r.source_id = s.id
         LEFT JOIN notification_state ns ON r.id = ns.release_id"
    )
}

fn row_to_release_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReleaseInfo> {
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
        flag: row.get(19)?,
        version_bump: row.get(20)?,
    })
}

pub fn get_release(conn: &Connection, id: i64) -> DbResult<Option<ReleaseInfo>> {
    let sql = format!("{} WHERE r.id = ?1", release_select_from("r.body", "r.body_translated"));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(params![id], row_to_release_info)?;
    match rows.next() {
        Some(Ok(release)) => Ok(Some(release)),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

pub const BODY_EXCERPT_CHARS: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyProjection {
    Full,
    Excerpt(i64),
}

fn query_releases(
    conn: &Connection,
    projection: BodyProjection,
    tier1_types: &[&str],
) -> DbResult<Vec<ReleaseInfo>> {
    let (body_expr, tr_expr, use_tier1) = match projection {
        BodyProjection::Full => ("r.body".to_string(), "r.body_translated".to_string(), false),
        BodyProjection::Excerpt(_) if tier1_types.is_empty() => (
            "substr(r.body, 1, ?)".to_string(),
            "substr(r.body_translated, 1, ?)".to_string(),
            false,
        ),
        BodyProjection::Excerpt(_) => {
            let placeholders = vec!["?"; tier1_types.len()].join(", ");
            let expr = format!(
                "CASE WHEN s.source_type IN ({placeholders}) THEN r.body ELSE substr(r.body, 1, ?) END"
            );
            let tr = format!(
                "CASE WHEN s.source_type IN ({placeholders}) THEN r.body_translated ELSE substr(r.body_translated, 1, ?) END"
            );
            (expr, tr, true)
        }
    };

    let sql = format!(
        "{}
         ORDER BY CASE
             WHEN r.published_at LIKE '1970%' THEN r.detected_at
             ELSE r.published_at
         END DESC, r.id DESC",
        release_select_from(&body_expr, &tr_expr)
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let BodyProjection::Excerpt(n) = projection {
        if use_tier1 {
            for t in tier1_types {
                params_vec.push(Box::new((*t).to_string()));
            }
        }
        params_vec.push(Box::new(n));
        if use_tier1 {
            for t in tier1_types {
                params_vec.push(Box::new((*t).to_string()));
            }
        }
        params_vec.push(Box::new(n));
    }
    let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let releases = stmt
        .query_map(params_ref.as_slice(), row_to_release_info)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(releases)
}

pub fn get_releases_with_state(conn: &Connection) -> DbResult<Vec<ReleaseInfo>> {
    query_releases(conn, BodyProjection::Full, &[])
}

pub fn get_release_catalog(
    conn: &Connection,
    tier1_types: &[&str],
) -> DbResult<Vec<ReleaseInfo>> {
    query_releases(conn, BodyProjection::Excerpt(BODY_EXCERPT_CHARS), tier1_types)
}

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct ReleaseSearchBody {
    pub id: i64,
    pub body: Option<String>,
    pub body_translated: Option<String>,
}

pub fn get_release_search_bodies(
    conn: &Connection,
    before_id: i64,
    max_chars: i64,
    tier1_types: &[&str],
) -> DbResult<Vec<ReleaseSearchBody>> {
    let type_clause = if tier1_types.is_empty() {
        String::new()
    } else {
        let placeholders = vec!["?"; tier1_types.len()].join(", ");
        format!("AND s.source_type NOT IN ({placeholders})")
    };
    let sql = format!(
        "SELECT r.id, r.body, r.body_translated
         FROM releases r
         JOIN sources s ON s.id = r.source_id
         WHERE r.id < ?
           {type_clause}
           AND (COALESCE(r.body, '') <> '' OR COALESCE(r.body_translated, '') <> '')
         ORDER BY r.id DESC"
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(before_id)];
    for t in tier1_types {
        params_vec.push(Box::new((*t).to_string()));
    }
    let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params_ref.as_slice())?;

    let mut out: Vec<ReleaseSearchBody> = Vec::new();
    let mut used: i64 = 0;
    while let Some(row) = rows.next()? {
        let body: Option<String> = row.get(1)?;
        let translated: Option<String> = row.get(2)?;
        let cost = body.as_deref().map_or(0, |s| s.chars().count() as i64)
            + translated.as_deref().map_or(0, |s| s.chars().count() as i64);
        if !out.is_empty() && used + cost > max_chars {
            break;
        }
        used += cost;
        out.push(ReleaseSearchBody {
            id: row.get(0)?,
            body,
            body_translated: translated,
        });
        if used >= max_chars {
            break;
        }
    }
    Ok(out)
}

pub fn get_release_bodies_by_ids(
    conn: &Connection,
    ids: &[i64],
    tier1_types: &[&str],
) -> DbResult<Vec<ReleaseSearchBody>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let id_ph = vec!["?"; ids.len()].join(", ");
    let type_clause = if tier1_types.is_empty() {
        String::new()
    } else {
        let type_ph = vec!["?"; tier1_types.len()].join(", ");
        format!("AND s.source_type NOT IN ({type_ph})")
    };
    let sql = format!(
        "SELECT r.id, r.body, r.body_translated
         FROM releases r
         JOIN sources s ON s.id = r.source_id
         WHERE r.id IN ({id_ph})
           {type_clause}
         ORDER BY r.id ASC"
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for id in ids {
        params_vec.push(Box::new(*id));
    }
    for t in tier1_types {
        params_vec.push(Box::new((*t).to_string()));
    }
    let params_ref: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(ReleaseSearchBody {
                id: row.get(0)?,
                body: row.get(1)?,
                body_translated: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn query_unread_releases(conn: &Connection, only_notified_missing: bool) -> DbResult<Vec<ReleaseInfo>> {
    let notified_filter = if only_notified_missing { " AND ns.last_notified_at IS NULL" } else { "" };
    let sql = format!(
        "{}
         WHERE COALESCE(ns.status, 'pending') IN ('pending', 'snoozed'){}
         ORDER BY r.detected_at DESC",
        release_select_from("r.body", "r.body_translated"),
        notified_filter,
    );

    let mut stmt = conn.prepare(&sql)?;
    let releases = stmt
        .query_map([], row_to_release_info)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(releases
        .into_iter()
        .filter(|r| is_release_due(&r.notification_status, r.snooze_until.as_deref()))
        .collect())
}

pub fn has_due_unread_release(conn: &Connection) -> DbResult<bool> {
    let has_pending: bool = conn
        .prepare(
            "SELECT 1
             FROM notification_state ns
             JOIN releases r ON r.id = ns.release_id
             JOIN sources s ON r.source_id = s.id
             WHERE ns.status = 'pending'
               AND s.muted = 0
             LIMIT 1",
        )?
        .exists([])?;
    if has_pending {
        return Ok(true);
    }

    let mut stmt = conn.prepare(
        "SELECT ns.snooze_until
         FROM notification_state ns
         JOIN releases r ON r.id = ns.release_id
         JOIN sources s ON r.source_id = s.id
         WHERE ns.status = 'snoozed'
           AND s.muted = 0",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let snooze_until: Option<String> = row.get(0)?;
        if is_release_due("snoozed", snooze_until.as_deref()) {
            return Ok(true);
        }
    }

    Ok(conn
        .prepare(
            "SELECT 1
             FROM releases r
             JOIN sources s ON r.source_id = s.id
             WHERE s.muted = 0
               AND NOT EXISTS (SELECT 1 FROM notification_state ns WHERE ns.release_id = r.id)
             LIMIT 1",
        )?
        .exists([])?)
}

#[cfg(test)]
pub fn get_unread_releases(conn: &Connection) -> DbResult<Vec<ReleaseInfo>> {
    query_unread_releases(conn, false)
}

pub fn get_pending_releases(conn: &Connection) -> DbResult<Vec<ReleaseInfo>> {
    let min_importance = get_setting_str(conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE)
        .unwrap_or_else(|_| DEFAULT_DEEPSEEK_MIN_IMPORTANCE.to_string());

    let pending: Vec<ReleaseInfo> = query_unread_releases(conn, true)?
        .into_iter()
        .filter(|r| {
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

    /// 读 notification_state.notify_failures（通知失败重试计数）。
    fn notify_failures(conn: &Connection, release_id: i64) -> i64 {
        conn.query_row(
            "SELECT notify_failures FROM notification_state WHERE release_id = ?1",
            rusqlite::params![release_id],
            |r| r.get(0),
        )
        .unwrap()
    }

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
    fn test_has_newer_release_compares_instant_not_string() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        insert_release(&conn, sid, "v1", "R", "https://x", "2026-08-20T11:00:17Z", false, None).unwrap();

        // 同一时刻的另两种写法：都不是「更新」（字典序会把 ...17.000Z 判成更新）
        assert!(!has_newer_release(&conn, sid, "2026-08-20T11:00:17.000Z").unwrap());
        assert!(!has_newer_release(&conn, sid, "2026-08-20T11:00:17+00:00").unwrap());
        // 早 1 毫秒 → 库里那条更新；晚 1 秒 → 库里那条更旧
        assert!(has_newer_release(&conn, sid, "2026-08-20T11:00:16.999Z").unwrap());
        assert!(!has_newer_release(&conn, sid, "2026-08-20T11:00:18Z").unwrap());
    }

    #[test]
    fn test_has_newer_release_subsecond_across_formats() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "bilibili", "1", "", "").unwrap();
        insert_release(&conn, sid, "BV1", "T", "https://x", "2026-08-20T11:00:17.565Z", false, None).unwrap();

        // 库里那条晚 565 毫秒：用整秒写法查询也必须判为「有更新版本」
        // （字典序会判成不更新，导致重复通知）
        assert!(has_newer_release(&conn, sid, "2026-08-20T11:00:17Z").unwrap());
        assert!(!has_newer_release(&conn, sid, "2026-08-20T11:00:17.565Z").unwrap());
        // 只查本源
        let other = sources::add_source(&conn, "github", "x", "y", "").unwrap();
        assert!(!has_newer_release(&conn, other, "2026-08-20T11:00:17Z").unwrap());
    }

    #[test]
    fn test_update_release_metadata_refreshes_existing() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "bilibili", "12345678", "", "").unwrap();
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

    /// 通知发送失败：清标记后该 release 重新进入待通知集合，超上限后不再重试。
    ///
    /// 这条链路是「先标记后发送」不丢通知的关键补偿（见 `record_notify_failure` 注释）。
    #[test]
    fn test_notify_failure_clears_mark_for_retry() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        // 发送前标记 → 不再待通知
        set_last_notified_at(&conn, rid).unwrap();
        assert_eq!(get_pending_releases(&conn).unwrap().len(), 0);

        // 第 1、2 次失败：计数未达上限 → 调用方清标记 → 重新待通知
        for expected in [1, 2] {
            assert_eq!(record_notify_failure(&conn, rid).unwrap(), expected);
            clear_last_notified_at(&conn, rid).unwrap();
            assert_eq!(
                get_pending_releases(&conn).unwrap().len(),
                1,
                "清标记后应重新进入待通知（下一轮重发）"
            );
            set_last_notified_at(&conn, rid).unwrap();
        }

        // 第 3 次失败：达上限，调用方不清标记 → 不再重试
        assert_eq!(record_notify_failure(&conn, rid).unwrap(), 3);
        assert_eq!(
            get_pending_releases(&conn).unwrap().len(),
            0,
            "超上限后应保持已标记（不再重试，避免每轮刷屏）"
        );
        // release 本身未丢：仍是 pending 状态，在未读列表里可见
        assert_eq!(get_unread_releases(&conn).unwrap().len(), 1);
    }

    /// 重置为待通知（重新标记未读 / 稍后提醒）时重试预算归零；标记本身不碰计数器。
    #[test]
    fn test_reset_to_pending_resets_notify_failures() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "test", "repo", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        set_last_notified_at(&conn, rid).unwrap();
        record_notify_failure(&conn, rid).unwrap();
        record_notify_failure(&conn, rid).unwrap();
        assert_eq!(notify_failures(&conn, rid), 2);

        set_notification_state(&conn, rid, "pending", None).unwrap();
        assert_eq!(notify_failures(&conn, rid), 0, "重置为待通知应清零重试预算");

        // 发送前标记不碰计数器：否则每轮标记都会重置预算，重试就没上限了
        record_notify_failure(&conn, rid).unwrap();
        set_last_notified_at(&conn, rid).unwrap();
        assert_eq!(notify_failures(&conn, rid), 1, "set_last_notified_at 不应清零计数");
    }

    #[test]
    fn test_set_last_notified_at_upserts_when_state_missing() {
        // state 行缺失（历史脏数据 / 插入失败遗留）时也必须落标记，
        // 否则 release 每轮都被重复通知
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
        // 两条写入同事务：成功时必带 state 行
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
        // 已通知的 release 从 clicked 改回 pending 时：last_notified_at 应被清空，
        // release 应重新出现在 pending 列表中
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
        // 已忽略的 release 改回 pending 时：last_notified_at 应被清空
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
        let bl = sources::add_source(conn, "bilibili", "12345678", "", "").unwrap();
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

    /// github / youtube 两源用例的便捷 seed（内部复用 seed_gh_yt_bili_releases）。
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

    // ── version_bump：semver 解析与变化类型 ──

    #[test]
    fn test_parse_semver_extracts_three_segments() {
        assert_eq!(parse_semver("v1.16.0"), Some((1, 16, 0)));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("release-1.2.3-beta.1"), Some((1, 2, 3)));
        // 两段补 0；四段取前三段
        assert_eq!(parse_semver("v1.16"), Some((1, 16, 0)));
        assert_eq!(parse_semver("1.2.3.4"), Some((1, 2, 3)));
    }

    #[test]
    fn test_parse_semver_rejects_non_versions() {
        // 单段数字（视频 id / BV 号 / build 号）不算版本
        assert_eq!(parse_semver("BV11a2c3d4e5f"), None);
        assert_eq!(parse_semver("dQw4w9WgXcQ"), None);
        assert_eq!(parse_semver("build123"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn test_bump_between_major_minor_patch() {
        assert_eq!(bump_between((1, 15, 9), (2, 0, 0)), Some("major"));
        assert_eq!(bump_between((1, 15, 9), (1, 16, 0)), Some("minor"));
        assert_eq!(bump_between((1, 16, 0), (1, 16, 1)), Some("patch"));
        // 相等或回落不算变化
        assert_eq!(bump_between((1, 16, 0), (1, 16, 0)), None);
        assert_eq!(bump_between((2, 0, 0), (1, 99, 99)), None);
    }

    #[test]
    fn test_insert_release_computes_version_bump_chain() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        // 乱序插入（先新后旧，等价 save_entries_generic 历史模式的进库顺序）。
        // 注意：insert_release 不自动维护 version_bump（逐条全链重算会退化为 O(N²)），
        // 改由批量保存入口（save_entries_generic / insert_new_models）在循环收尾时统一
        // recompute 一次。此测试模拟该收尾语义：插入完成后调用一次
        // recompute_version_bumps，验证全链推导结果。
        insert_release(&conn, sid, "v1.2.1", "R3", "https://x", "2024-01-03T00:00:00Z", false, None).unwrap();
        insert_release(&conn, sid, "v1.2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();
        insert_release(&conn, sid, "v1.0.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        recompute_version_bumps(&conn, sid).unwrap(); // 批量收尾（save 循环退出后统一重算）

        let releases = get_releases_with_state(&conn).unwrap();
        let bump_of = |tag: &str| {
            releases
                .iter()
                .find(|r| r.tag_name == tag)
                .and_then(|r| r.version_bump.clone())
        };
        // 最早版本无基线 → NULL；v1.2.0 相对 v1.0.0 是 minor；v1.2.1 相对 v1.2.0 是 patch
        assert_eq!(bump_of("v1.0.0"), None);
        assert_eq!(bump_of("v1.2.0").as_deref(), Some("minor"));
        assert_eq!(bump_of("v1.2.1").as_deref(), Some("patch"));
    }

    #[test]
    fn test_version_bump_null_for_non_semver_source() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "bilibili", "12345678", "", "").unwrap();
        insert_release(&conn, sid, "BV11a2c3d4e5f", "V1", "https://b", "2024-01-01T00:00:00Z", false, None).unwrap();
        insert_release(&conn, sid, "BV12a2c3d4e5f", "V2", "https://b", "2024-01-02T00:00:00Z", false, None).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert!(
            releases.iter().all(|r| r.version_bump.is_none()),
            "非 semver tag（BV 号）不应产生 version_bump"
        );
    }

    #[test]
    fn test_set_release_flag_stores_and_validates() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let rid = insert_release(&conn, sid, "v1.0.0", "R", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();

        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].flag, 0, "默认未标记");

        set_release_flag(&conn, rid, 3).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].flag, 3);

        set_release_flag(&conn, rid, 0).unwrap();
        let releases = get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].flag, 0);

        assert!(set_release_flag(&conn, rid, 7).is_err(), "越界旗标应报错");
        assert!(set_release_flag(&conn, rid, -1).is_err(), "负数旗标应报错");
    }

    // ── 目录投影（get_release_catalog）与正文分块（get_release_search_bodies）──

    /// 目录必须回全库：第 201 条及更早的版本同样可见（防 LIMIT 回归）。
    #[test]
    fn test_release_catalog_returns_all_rows_without_limit() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        for i in 0..250 {
            insert_release(
                &conn,
                sid,
                &format!("v0.0.{i}"),
                "R",
                "https://x",
                &format!("2024-01-01T00:{:02}:{:02}Z", i / 60, i % 60),
                false,
                None,
            )
            .unwrap();
        }
        let catalog = get_release_catalog(&conn, &["youtube", "bilibili"]).unwrap();
        assert_eq!(catalog.len(), 250, "目录必须回全库，不再截断到 200");
    }

    /// 目录正文是预览投影：Tier2 源（github）截断，Tier1 源（youtube）保留全文。
    /// 用中文正文验证 `substr` 按**字符**而非字节截断。
    #[test]
    fn test_release_catalog_excerpts_tier2_body_but_keeps_tier1_full() {
        let conn = init_memory_db().unwrap();
        let gh = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let yt = sources::add_source(&conn, "youtube", "UCabc", "", "").unwrap();
        let long_zh = "中".repeat(1000);

        insert_release(&conn, gh, "v1.0.0", "R", "https://x", "2024-01-01T00:00:00Z", false, Some(&long_zh)).unwrap();
        insert_release(&conn, yt, "vid1", "V", "https://y", "2024-01-02T00:00:00Z", false, Some(&long_zh)).unwrap();

        let catalog = get_release_catalog(&conn, &["youtube", "bilibili"]).unwrap();
        let gh_row = catalog.iter().find(|r| r.source_type == "github").unwrap();
        let yt_row = catalog.iter().find(|r| r.source_type == "youtube").unwrap();

        assert_eq!(
            gh_row.body.as_deref().map(|s| s.chars().count()),
            Some(BODY_EXCERPT_CHARS as usize),
            "Tier2 源正文应截断到预览长度（按字符）"
        );
        assert_eq!(
            yt_row.body.as_deref().map(|s| s.chars().count()),
            Some(1000),
            "Tier1 源（视频）正文是内容载体，目录必须回全文"
        );

        // 全文投影不受影响：内部读取仍是完整正文
        let full = get_releases_with_state(&conn).unwrap();
        assert!(full.iter().all(|r| r.body.as_deref().map(|s| s.chars().count()) == Some(1000)));
    }

    /// 正文分块：跳过 Tier1 源、跳过空正文、**按 id 降序（新的在前）**、遵守字符预算。
    #[test]
    fn test_release_search_bodies_cursor_budget_and_filters() {
        let conn = init_memory_db().unwrap();
        let gh = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let yt = sources::add_source(&conn, "youtube", "UCabc", "", "").unwrap();

        let body = |n: usize| "x".repeat(n);
        let a = insert_release(&conn, gh, "v1", "R", "u", "2024-01-01T00:00:00Z", false, Some(&body(100))).unwrap();
        // 无正文的 Tier2 条目应被跳过
        insert_release(&conn, gh, "v2", "R", "u", "2024-01-02T00:00:00Z", false, None).unwrap();
        // Tier1 源的正文已在目录里，不应重复下发
        insert_release(&conn, yt, "vid", "V", "u", "2024-01-03T00:00:00Z", false, Some(&body(100))).unwrap();
        let b = insert_release(&conn, gh, "v3", "R", "u", "2024-01-04T00:00:00Z", false, Some(&body(100))).unwrap();

        // 首次调用用「大于任何真实 id」的游标：先回最新的 b
        let first = get_release_search_bodies(&conn, i64::MAX, 150, &["youtube", "bilibili"]).unwrap();
        assert_eq!(first.iter().map(|r| r.id).collect::<Vec<_>>(), vec![b], "预算 150 只装得下最新一条");

        let rest = get_release_search_bodies(&conn, b, 10_000, &["youtube", "bilibili"]).unwrap();
        assert_eq!(rest.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a], "游标向下跳过无正文与 Tier1 条目");

        assert!(get_release_search_bodies(&conn, a, 10_000, &["youtube", "bilibili"]).unwrap().is_empty());
    }

    /// 方向回归：水位装不下的必须是更早的正文（新版本优先入索引）。
    #[test]
    fn test_release_search_bodies_prefers_newest() {
        let conn = init_memory_db().unwrap();
        let gh = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let body = |n: usize| "x".repeat(n);
        let old = insert_release(&conn, gh, "v1", "R", "u", "2024-01-01T00:00:00Z", false, Some(&body(100))).unwrap();
        let new = insert_release(&conn, gh, "v2", "R", "u", "2024-01-02T00:00:00Z", false, Some(&body(100))).unwrap();
        let newest = insert_release(&conn, gh, "v3", "R", "u", "2024-01-03T00:00:00Z", false, Some(&body(100))).unwrap();

        // 预算只够一条：必须是 id 最大（最新入库）的那条
        let chunk = get_release_search_bodies(&conn, i64::MAX, 120, &["youtube", "bilibili"]).unwrap();
        assert_eq!(chunk.iter().map(|r| r.id).collect::<Vec<_>>(), vec![newest]);
        assert!(newest > new && new > old);
    }

    /// 单条正文超过预算时也必须回该条，否则游标永远无法前进。
    #[test]
    fn test_release_search_bodies_always_returns_at_least_one_row() {
        let conn = init_memory_db().unwrap();
        let gh = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let big = "y".repeat(5000);
        let rid = insert_release(&conn, gh, "v1", "R", "u", "2024-01-01T00:00:00Z", false, Some(&big)).unwrap();

        let chunk = get_release_search_bodies(&conn, i64::MAX, 100, &["youtube", "bilibili"]).unwrap();
        assert_eq!(chunk.len(), 1);
        assert_eq!(chunk[0].id, rid);
        assert_eq!(chunk[0].body.as_deref().map(|s| s.len()), Some(5000));
    }

    /// 按 id 批量取正文：用于刷新已变化的行（翻译落库等）。
    #[test]
    fn test_release_bodies_by_ids_refreshes_changed_rows() {
        let conn = init_memory_db().unwrap();
        let gh = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let yt = sources::add_source(&conn, "youtube", "UCabc", "", "").unwrap();
        let a = insert_release(&conn, gh, "v1", "R", "u", "2024-01-01T00:00:00Z", false, Some("body a")).unwrap();
        let b = insert_release(&conn, gh, "v2", "R", "u", "2024-01-02T00:00:00Z", false, Some("body b")).unwrap();
        let v = insert_release(&conn, yt, "vid", "V", "u", "2024-01-03T00:00:00Z", false, Some("video")).unwrap();

        assert!(get_release_bodies_by_ids(&conn, &[], &["youtube", "bilibili"]).unwrap().is_empty());

        // 乱序传入也按 id 升序返回；Tier1 源被排除
        let got = get_release_bodies_by_ids(&conn, &[b, v, a], &["youtube", "bilibili"]).unwrap();
        assert_eq!(got.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a, b]);

        // 翻译落库后按 id 重取能拿到新文本
        set_body_translated(&conn, a, "译文 a").unwrap();
        let refreshed = get_release_bodies_by_ids(&conn, &[a], &["youtube", "bilibili"]).unwrap();
        assert_eq!(refreshed[0].body_translated.as_deref(), Some("译文 a"));
    }

    #[test]
    fn test_batch_set_notification_state_dedups_and_skips_missing() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let r1 = insert_release(&conn, sid, "v1", "R1", "u1", "2024-01-01T00:00:00Z", false, None).unwrap();
        let r2 = insert_release(&conn, sid, "v2", "R2", "u2", "2024-01-02T00:00:00Z", false, None).unwrap();

        let updated = batch_set_notification_state(&conn, &[r1, r1, r2, 9999], "clicked", None).unwrap();
        assert_eq!(updated, 2);
        assert_eq!(get_release(&conn, r1).unwrap().unwrap().notification_status, "clicked");
        assert_eq!(get_release(&conn, r2).unwrap().unwrap().notification_status, "clicked");

        assert!(batch_set_notification_state(&conn, &[r1], "bogus", None).is_err());
        assert_eq!(batch_set_notification_state(&conn, &[], "ignored", None).unwrap(), 0);
    }

    #[test]
    fn test_batch_set_release_flag_and_delete() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let r1 = insert_release(&conn, sid, "v1", "R1", "u1", "2024-01-01T00:00:00Z", false, None).unwrap();
        let r2 = insert_release(&conn, sid, "v2", "R2", "u2", "2024-01-02T00:00:00Z", false, None).unwrap();
        let r3 = insert_release(&conn, sid, "v3", "R3", "u3", "2024-01-03T00:00:00Z", false, None).unwrap();

        assert_eq!(batch_set_release_flag(&conn, &[r1, r2, r2, 9999], 3).unwrap(), 2);
        assert_eq!(get_release(&conn, r1).unwrap().unwrap().flag, 3);
        assert_eq!(get_release(&conn, r2).unwrap().unwrap().flag, 3);
        assert_eq!(get_release(&conn, r3).unwrap().unwrap().flag, 0);
        assert!(batch_set_release_flag(&conn, &[r1], 7).is_err());

        assert_eq!(batch_delete_releases(&conn, &[r1, r2, 9999]).unwrap(), 2);
        assert!(get_release(&conn, r1).unwrap().is_none());
        assert!(get_release(&conn, r2).unwrap().is_none());
        assert!(get_release(&conn, r3).unwrap().is_some());
    }

    #[test]
    fn test_mark_source_releases_read_respects_future_snooze_and_source_scope() {
        let conn = init_memory_db().unwrap();
        let s1 = sources::add_source(&conn, "github", "o", "r1", "").unwrap();
        let s2 = sources::add_source(&conn, "github", "o", "r2", "").unwrap();

        let r_pending = insert_release(&conn, s1, "v1", "R1", "u1", "2024-01-01T00:00:00Z", false, None).unwrap();
        let r_due_snooze = insert_release(&conn, s1, "v2", "R2", "u2", "2024-01-02T00:00:00Z", false, None).unwrap();
        let r_future_snooze = insert_release(&conn, s1, "v3", "R3", "u3", "2024-01-03T00:00:00Z", false, None).unwrap();
        let r_ignored = insert_release(&conn, s1, "v4", "R4", "u4", "2024-01-04T00:00:00Z", false, None).unwrap();
        let r_other_source = insert_release(&conn, s2, "v1", "O1", "u5", "2024-01-05T00:00:00Z", false, None).unwrap();

        let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let future = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        set_notification_state(&conn, r_due_snooze, "snoozed", Some(&past)).unwrap();
        set_notification_state(&conn, r_future_snooze, "snoozed", Some(&future)).unwrap();
        set_notification_state(&conn, r_ignored, "ignored", None).unwrap();

        let updated = mark_source_releases_read(&conn, s1).unwrap();
        assert_eq!(updated, 2);
        assert_eq!(get_release(&conn, r_pending).unwrap().unwrap().notification_status, "clicked");
        assert_eq!(get_release(&conn, r_due_snooze).unwrap().unwrap().notification_status, "clicked");
        assert_eq!(get_release(&conn, r_future_snooze).unwrap().unwrap().notification_status, "snoozed");
        assert_eq!(get_release(&conn, r_ignored).unwrap().unwrap().notification_status, "ignored");
        assert_eq!(get_release(&conn, r_other_source).unwrap().unwrap().notification_status, "pending");
    }
}
