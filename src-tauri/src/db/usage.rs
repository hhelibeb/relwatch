use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;

/// 单个事件的聚合统计行：total_count 为累计次数（SUM），daily 为按天趋势。
#[derive(Debug, Serialize, Clone, Type)]
pub struct UsageStatRow {
    pub key: String,
    pub total_count: i64,
    pub last_day: String,
    pub daily: Vec<UsageDaily>,
}

/// 单日计数（按本地时区 YYYY-MM-DD 分桶）。
#[derive(Debug, Serialize, Clone, Type)]
pub struct UsageDaily {
    pub day: String,
    pub count: i64,
}

/// 当前本地日期（YYYY-MM-DD），作为按天分桶的桶键。
pub fn local_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 批量累加事件计数（UPSERT，幂等可重放）。
/// events 为 (事件 key, 本次次数) 列表，由前端 5s 节流聚合后批量上报。
pub fn record_usage(conn: &Connection, events: &[(String, u32)]) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let today = local_today();
    for (key, count) in events {
        if key.is_empty() || *count == 0 {
            continue;
        }
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)
             ON CONFLICT(key, day) DO UPDATE SET count = count + excluded.count",
            params![key, today, *count],
        )
        .map_err(|e| format!("记录使用统计失败: {}", e))?;
    }
    Ok(())
}

/// 查询统计：按 key 聚合，返回累计次数降序列表。
/// `days` 提供时仅统计最近 N 天（趋势窗口），daily 为窗口内逐日计数。
/// 表很小（几十个 key × 天数），直接全量读取后在内存聚合，避免 N+1 查询。
pub fn get_usage_stats(conn: &Connection, days: Option<u32>) -> Result<Vec<UsageStatRow>, String> {
    let mut stmt = conn
        .prepare("SELECT key, day, count FROM usage_stats")
        .map_err(|e| format!("查询使用统计失败: {}", e))?;

    // 过滤窗口：最近 N 天（含今天）
    let min_day = days
        .map(|n| {
            (chrono::Local::now() - chrono::Duration::days(n.saturating_sub(1) as i64))
                .format("%Y-%m-%d")
                .to_string()
        });

    // key -> (total, last_day, daily 有序列表)
    let mut by_key: HashMap<String, (i64, String, Vec<UsageDaily>)> = HashMap::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| format!("查询使用统计失败: {}", e))?;

    for row in rows {
        let (key, day, count) = row.map_err(|e| format!("读取使用统计行失败: {}", e))?;
        if let Some(ref min) = min_day {
            if day.as_str() < min.as_str() {
                continue;
            }
        }
        let entry = by_key.entry(key).or_insert_with(|| (0, String::new(), Vec::new()));
        entry.0 += count;
        if day > entry.1 {
            entry.1 = day.clone();
        }
        entry.2.push(UsageDaily { day, count });
    }

    // 每 key 的 daily 按天升序
    let mut result: Vec<UsageStatRow> = by_key
        .into_iter()
        .map(|(key, (total_count, last_day, mut daily))| {
            daily.sort_by(|a, b| a.day.cmp(&b.day));
            UsageStatRow {
                key,
                total_count,
                last_day,
                daily,
            }
        })
        .collect();

    // 累计次数降序，同次数按 key 字典序（稳定输出）
    result.sort_by(|a, b| b.total_count.cmp(&a.total_count).then(a.key.cmp(&b.key)));
    Ok(result)
}

/// 清空全部统计。
pub fn clear_usage_stats(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM usage_stats", [])
        .map_err(|e| format!("清空使用统计失败: {}", e))?;
    Ok(())
}

/// 按保留天数清理过期分桶（与操作日志共用 log_retention_days 设置，不单独配置）。
/// 语义与 delete_old_logs 对齐：`days <= 0` 表示不清理。
/// 保留最近 `days` 个自然日（含今天），如 days=7 保留今天往前共 7 天，
/// 删除更早的 (key, day) 行；与 get_usage_stats(days) 的窗口口径一致。
pub fn prune_old_usage_stats(conn: &Connection, days: i64) {
    if days <= 0 {
        return;
    }
    let cutoff = (chrono::Local::now() - chrono::Duration::days(days.saturating_sub(1)))
        .format("%Y-%m-%d")
        .to_string();
    let _ = conn.execute("DELETE FROM usage_stats WHERE day < ?1", params![cutoff]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_record_usage_upserts_same_day() {
        let conn = init_memory_db().unwrap();
        record_usage(&conn, &[("source.add".into(), 1)]).unwrap();
        // 同 key 同日再次写入应累加而非覆盖
        record_usage(&conn, &[("source.add".into(), 2)]).unwrap();
        let rows = get_usage_stats(&conn, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "source.add");
        assert_eq!(rows[0].total_count, 3);
        assert_eq!(rows[0].daily.len(), 1);
        assert_eq!(rows[0].daily[0].count, 3);
    }

    #[test]
    fn test_record_usage_empty_or_blank_keys() {
        let conn = init_memory_db().unwrap();
        record_usage(&conn, &[]).unwrap();
        record_usage(&conn, &[("".into(), 5)]).unwrap();
        record_usage(&conn, &[("source.check".into(), 0)]).unwrap();
        let rows = get_usage_stats(&conn, None).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_get_usage_stats_sorted_by_total_desc() {
        let conn = init_memory_db().unwrap();
        record_usage(&conn, &[("a".into(), 1)]).unwrap();
        record_usage(&conn, &[("b".into(), 5)]).unwrap();
        record_usage(&conn, &[("c".into(), 3)]).unwrap();
        let rows = get_usage_stats(&conn, None).unwrap();
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_get_usage_stats_days_window_filters_old() {
        let conn = init_memory_db().unwrap();
        // 手工插入一条"昨天"的旧数据，模拟跨天
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let today = local_today();
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)",
            params!["old.event", yesterday, 10],
        )
        .unwrap();
        record_usage(&conn, &[("today.event".into(), 2)]).unwrap();

        let all = get_usage_stats(&conn, None).unwrap();
        let all_total: i64 = all.iter().map(|r| r.total_count).sum();
        assert_eq!(all_total, 12, "无窗口时应包含全部数据");

        let recent = get_usage_stats(&conn, Some(1)).unwrap();
        let recent_total: i64 = recent.iter().map(|r| r.total_count).sum();
        assert_eq!(recent_total, 2, "窗口 1 天应只含今天的记录");
        assert_eq!(recent[0].key, "today.event");
        assert_eq!(recent[0].last_day, today);
    }

    #[test]
    fn test_prune_old_usage_stats_respects_days() {
        let conn = init_memory_db().unwrap();
        // 手工插入跨天数据：今天 / 昨天 / 8 天前
        let today = local_today();
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let old_day = (chrono::Local::now() - chrono::Duration::days(8))
            .format("%Y-%m-%d")
            .to_string();
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)",
            params!["today.event", today, 1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)",
            params!["yesterday.event", yesterday, 1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)",
            params!["old.event", old_day, 1],
        )
        .unwrap();

        // days=1：只保留今天
        prune_old_usage_stats(&conn, 1);
        let rows = get_usage_stats(&conn, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "today.event");

        // 重新写入昨天与 8 天前，days=7：保留最近 7 个自然日（今天+昨天），删 8 天前
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)",
            params!["yesterday.event", yesterday, 1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage_stats (key, day, count) VALUES (?1, ?2, ?3)",
            params!["old.event", old_day, 1],
        )
        .unwrap();
        prune_old_usage_stats(&conn, 7);
        let rows = get_usage_stats(&conn, None).unwrap();
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["today.event", "yesterday.event"]);
    }

    #[test]
    fn test_prune_old_usage_stats_zero_or_negative_noop() {
        let conn = init_memory_db().unwrap();
        record_usage(&conn, &[("source.add".into(), 1)]).unwrap();

        prune_old_usage_stats(&conn, 0);
        prune_old_usage_stats(&conn, -1);
        prune_old_usage_stats(&conn, -100);

        assert_eq!(get_usage_stats(&conn, None).unwrap().len(), 1, "days<=0 不应清理");
    }

    #[test]
    fn test_clear_usage_stats_empties_table() {
        let conn = init_memory_db().unwrap();
        record_usage(&conn, &[("source.add".into(), 1)]).unwrap();
        assert_eq!(get_usage_stats(&conn, None).unwrap().len(), 1);
        clear_usage_stats(&conn).unwrap();
        assert!(get_usage_stats(&conn, None).unwrap().is_empty());
        // 清空后仍可继续记录
        record_usage(&conn, &[("source.add".into(), 1)]).unwrap();
        assert_eq!(get_usage_stats(&conn, None).unwrap().len(), 1);
    }

    #[test]
    fn test_local_today_format() {
        let today = local_today();
        assert_eq!(today.len(), 10, "YYYY-MM-DD 应为 10 字符");
        assert!(today.chars().nth(4) == Some('-') && today.chars().nth(7) == Some('-'));
    }
}
