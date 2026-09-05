use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

/// 单次 AI 调用的用量记录（落 `ai_usage` 表的最小单元）。
/// `estimated`：API 未返回 usage（部分中转会剥离）时按字符数 ÷2 估算的标记，
/// 与真实统计区分展示。
///
/// 成本预留设计：`cache_hit_tokens` / `cache_miss_tokens` 分列（DeepSeek 缓存命中
/// 单价约为未命中的 1/10）+ `model` + `day` 三者已把金额核算的全部维度采齐，
/// 将来做费用展示时由前端按单价表现算即可，历史数据无需任何迁移。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CallUsage {
    pub action: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_hit_tokens: i64,
    pub cache_miss_tokens: i64,
    pub estimated: bool,
    pub duration_ms: i64,
}

impl CallUsage {
    /// 由一次成功的 chat/completions 响应构造用量记录：有 usage 用真实值，
    /// 缺失时按字符数 ÷2 估算（中文约 1 token/字、英文约 1 token/4 字符，
    /// 混合场景 ÷2 是偏保守的兜底，与前端 useAgentUsage 的估算口径一致）。
    pub fn from_outcome(
        action: &'static str,
        usage: Option<RawUsage>,
        content: &str,
        prompt_chars: usize,
        duration_ms: i64,
    ) -> Self {
        match usage {
            Some(u) => Self {
                action: action.to_string(),
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                cache_hit_tokens: u.cache_hit_tokens,
                cache_miss_tokens: u.cache_miss_tokens,
                estimated: false,
                duration_ms,
            },
            None => Self::estimate(action, prompt_chars, content.chars().count(), duration_ms),
        }
    }

    /// usage 缺失时的字符数估算兜底（estimated=true 标记）。
    /// ÷2 向上取整手写为 `(x + 1) / 2`：i64::div_ceil 仍是 unstable（#88581）。
    pub fn estimate(action: &'static str, prompt_chars: usize, completion_chars: usize, duration_ms: i64) -> Self {
        Self {
            action: action.to_string(),
            prompt_tokens: (prompt_chars as i64 + 1) / 2,
            completion_tokens: (completion_chars as i64 + 1) / 2,
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
            estimated: true,
            duration_ms,
        }
    }
}

/// 一次成功 chat/completions 响应的 token 用量（OpenAI 标准 `usage` 字段 + DeepSeek
/// 缓存命中扩展）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_hit_tokens: i64,
    pub cache_miss_tokens: i64,
}

/// 批量落一条 AI 任务的用量明细（一次任务可能产生多次调用：语言检测 + 翻译）。
/// `release_id` 提供时联动查出 `source_id`；落库失败由调用方记日志后忽略，
/// 绝不影响摘要/翻译主流程。
pub fn insert_call_usage(
    conn: &Connection,
    release_id: Option<i64>,
    model: &str,
    usages: &[CallUsage],
) -> Result<(), String> {
    if usages.is_empty() {
        return Ok(());
    }
    let source_id = match release_id {
        Some(id) => conn
            .query_row("SELECT source_id FROM releases WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
            .ok(),
        None => None,
    };
    let day = super::usage::local_today();
    let created_at = chrono::Utc::now().to_rfc3339();
    for u in usages {
        conn.execute(
            "INSERT INTO ai_usage (release_id, source_id, action, model, prompt_tokens,
                completion_tokens, cache_hit_tokens, cache_miss_tokens, estimated, duration_ms,
                day, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                release_id,
                source_id,
                u.action,
                model,
                u.prompt_tokens,
                u.completion_tokens,
                u.cache_hit_tokens,
                u.cache_miss_tokens,
                u.estimated as i64,
                u.duration_ms,
                day,
                created_at,
            ],
        )
        .map_err(|e| format!("写入 ai_usage 失败: {}", e))?;
    }
    Ok(())
}

/// 逐日聚合行（热力图数据源）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiUsageDaily {
    pub day: String,
    pub calls: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_hit_tokens: i64,
    pub cache_miss_tokens: i64,
}

/// 按监控源聚合行（表格 / 饼图数据源）。`label` 为监控源本身的标识
/// `owner/repo`（description 是用户备注，不作为统计维度的显示名）；
/// `source_id` 为 None 表示无源调用（连接测试），前端以「无源」聚合展示。
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiUsageSourceRow {
    pub source_id: Option<i64>,
    pub label: Option<String>,
    pub source_type: Option<String>,
    pub calls: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_hit_tokens: i64,
    pub cache_miss_tokens: i64,
}

/// 按操作类型聚合行（饼图「按操作」维度：摘要 / 翻译 / 语言检测 / 连接测试）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiUsageActionRow {
    pub action: String,
    pub calls: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// 一次查询返回的全部聚合结果（热力图 + 表格 + 饼图共享同一份筛选条件）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct AiUsageStats {
    pub daily: Vec<AiUsageDaily>,
    pub by_source: Vec<AiUsageSourceRow>,
    pub by_action: Vec<AiUsageActionRow>,
}

/// WHERE 片段：可选监控源过滤 + 可选最近 N 天窗口（按本地日期，含今天）。
/// 参数顺序固定为 (source_id, min_day)，两个 NULL 表示不过滤。
fn filtered_where() -> &'static str {
    "WHERE (?1 IS NULL OR source_id = ?1) AND (?2 IS NULL OR day >= ?2)"
}

fn min_day(days: Option<u32>) -> Option<String> {
    days.map(|n| {
        (chrono::Local::now() - chrono::Duration::days(n.saturating_sub(1) as i64))
            .format("%Y-%m-%d")
            .to_string()
    })
}

/// 聚合查询：一次返回逐日 / 按源 / 按操作类型三组聚合，供统计弹窗一次拉全。
pub fn get_ai_usage_stats(
    conn: &Connection,
    source_id: Option<i64>,
    days: Option<u32>,
) -> Result<AiUsageStats, String> {
    let min_day = min_day(days);

    let mut daily = Vec::new();
    {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT day, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens),
                        SUM(cache_hit_tokens), SUM(cache_miss_tokens)
                 FROM ai_usage {} GROUP BY day ORDER BY day",
                filtered_where()
            ))
            .map_err(|e| format!("查询 AI 用量统计失败: {}", e))?;
        let rows = stmt
            .query_map(params![source_id, min_day], |r| {
                Ok(AiUsageDaily {
                    day: r.get(0)?,
                    calls: r.get(1)?,
                    prompt_tokens: r.get(2)?,
                    completion_tokens: r.get(3)?,
                    cache_hit_tokens: r.get(4)?,
                    cache_miss_tokens: r.get(5)?,
                })
            })
            .map_err(|e| format!("查询 AI 用量统计失败: {}", e))?;
        for row in rows {
            daily.push(row.map_err(|e| format!("读取 AI 用量统计失败: {}", e))?);
        }
    }

    let mut by_source = Vec::new();
    {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT ai_usage.source_id,
                        s.owner || '/' || s.repo,
                        s.source_type,
                        COUNT(*), SUM(ai_usage.prompt_tokens), SUM(ai_usage.completion_tokens),
                        SUM(ai_usage.cache_hit_tokens), SUM(ai_usage.cache_miss_tokens)
                 FROM ai_usage LEFT JOIN sources s ON s.id = ai_usage.source_id
                 {} GROUP BY ai_usage.source_id
                 ORDER BY SUM(ai_usage.prompt_tokens + ai_usage.completion_tokens) DESC",
                filtered_where()
            ))
            .map_err(|e| format!("查询 AI 用量统计失败: {}", e))?;
        let rows = stmt
            .query_map(params![source_id, min_day], |r| {
                Ok(AiUsageSourceRow {
                    source_id: r.get(0)?,
                    label: r.get(1)?,
                    source_type: r.get(2)?,
                    calls: r.get(3)?,
                    prompt_tokens: r.get(4)?,
                    completion_tokens: r.get(5)?,
                    cache_hit_tokens: r.get(6)?,
                    cache_miss_tokens: r.get(7)?,
                })
            })
            .map_err(|e| format!("查询 AI 用量统计失败: {}", e))?;
        for row in rows {
            by_source.push(row.map_err(|e| format!("读取 AI 用量统计失败: {}", e))?);
        }
    }

    let mut by_action = Vec::new();
    {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT action, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens)
                 FROM ai_usage {} GROUP BY action ORDER BY COUNT(*) DESC, action ASC",
                filtered_where()
            ))
            .map_err(|e| format!("查询 AI 用量统计失败: {}", e))?;
        let rows = stmt
            .query_map(params![source_id, min_day], |r| {
                Ok(AiUsageActionRow {
                    action: r.get(0)?,
                    calls: r.get(1)?,
                    prompt_tokens: r.get(2)?,
                    completion_tokens: r.get(3)?,
                })
            })
            .map_err(|e| format!("查询 AI 用量统计失败: {}", e))?;
        for row in rows {
            by_action.push(row.map_err(|e| format!("读取 AI 用量统计失败: {}", e))?);
        }
    }

    Ok(AiUsageStats {
        daily,
        by_source,
        by_action,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    fn setup_release(conn: &Connection) -> i64 {
        // description 有意给非空值：统计 label 应取 owner/repo 本名而非备注
        let sid = crate::db::sources::add_source(conn, "github", "octocat", "hello", "用户备注名").unwrap();
        crate::db::releases::insert_release(
            conn,
            sid,
            "v1",
            "v1",
            "https://github.com/octocat/hello/releases/tag/v1",
            "2024-01-01T00:00:00Z",
            false,
            Some("body"),
        )
        .unwrap()
    }

    #[test]
    fn test_insert_and_stats_roundtrip() {
        let conn = init_memory_db().unwrap();
        let rid = setup_release(&conn);

        // 一条 release：detect + translate 两次调用（真实 usage）
        insert_call_usage(
            &conn,
            Some(rid),
            "deepseek-chat",
            &[
                CallUsage::from_outcome("detect_language", Some(RawUsage { prompt_tokens: 100, completion_tokens: 2, cache_hit_tokens: 0, cache_miss_tokens: 100 }), "中文", 0, 500),
                CallUsage::from_outcome("translate", Some(RawUsage { prompt_tokens: 1000, completion_tokens: 900, cache_hit_tokens: 400, cache_miss_tokens: 600 }), "译文", 0, 8000),
            ],
        )
        .unwrap();
        // 一条无源调用（连接测试，usage 缺失 → 估算）
        insert_call_usage(
            &conn,
            None,
            "deepseek-chat",
            &[CallUsage::from_outcome("test", None, "Hi", 2, 300)],
        )
        .unwrap();

        let stats = get_ai_usage_stats(&conn, None, None).unwrap();
        assert_eq!(stats.daily.len(), 1, "同一本地日的记录应聚到一天");
        let d = &stats.daily[0];
        assert_eq!(d.calls, 3);
        assert_eq!(d.prompt_tokens, 100 + 1000 + 1, "估算 prompt = chars/2 向上取整");
        assert_eq!(d.completion_tokens, 2 + 900 + 1);
        assert_eq!(d.cache_hit_tokens, 400);
        assert_eq!(d.cache_miss_tokens, 100 + 600);

        // 按源：有源一组（detect+translate），无源一组（test），按 token 合计降序
        assert_eq!(stats.by_source.len(), 2);
        assert_eq!(stats.by_source[0].label.as_deref(), Some("octocat/hello"));
        assert_eq!(stats.by_source[0].calls, 2);
        assert!(stats.by_source[1].source_id.is_none(), "连接测试应归入无源组");

        // 按操作：三种 action 各一行（同计数按字典序，稳定输出）
        let actions: Vec<&str> = stats.by_action.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(actions, vec!["detect_language", "test", "translate"]);

        // 按源过滤
        let sid: i64 = conn
            .query_row("SELECT source_id FROM releases WHERE id = ?1", [rid], |r| r.get(0))
            .unwrap();
        let filtered = get_ai_usage_stats(&conn, Some(sid), None).unwrap();
        assert_eq!(filtered.daily[0].calls, 2, "按源过滤后不应含无源调用");
        assert_eq!(filtered.by_source.len(), 1);
    }

    #[test]
    fn test_get_ai_usage_stats_days_window() {
        let conn = init_memory_db().unwrap();
        let rid = setup_release(&conn);
        // 手工插入一条 10 天前的旧记录
        let old_day = (chrono::Local::now() - chrono::Duration::days(10))
            .format("%Y-%m-%d")
            .to_string();
        conn.execute(
            "INSERT INTO ai_usage (release_id, source_id, action, model, prompt_tokens,
                completion_tokens, cache_hit_tokens, cache_miss_tokens, estimated, duration_ms,
                day, created_at)
             VALUES (?1, NULL, 'translate', 'm', 5, 5, 0, 0, 0, 0, ?2, '')",
            params![rid, old_day],
        )
        .unwrap();
        insert_call_usage(&conn, Some(rid), "m", &[CallUsage::estimate("summary", 10, 4, 100)]).unwrap();

        let all = get_ai_usage_stats(&conn, None, None).unwrap();
        assert_eq!(all.daily.len(), 2, "无窗口应含两天");

        let recent = get_ai_usage_stats(&conn, None, Some(7)).unwrap();
        assert_eq!(recent.daily.len(), 1, "7 天窗口应排除 10 天前的记录");
        assert_eq!(recent.daily[0].calls, 1);
    }

    #[test]
    fn test_insert_empty_usages_noop() {
        let conn = init_memory_db().unwrap();
        insert_call_usage(&conn, None, "m", &[]).unwrap();
        assert!(get_ai_usage_stats(&conn, None, None).unwrap().daily.is_empty());
    }

    #[test]
    fn test_estimate_flags_and_rounding() {
        let u = CallUsage::estimate("test", 3, 3, 50);
        assert!(u.estimated);
        assert_eq!(u.prompt_tokens, 2, "3 字符 ÷2 向上取整 = 2");
        assert_eq!(u.completion_tokens, 2);

        let real = CallUsage::from_outcome("translate", Some(RawUsage::default()), "x", 0, 1);
        assert!(!real.estimated, "有 usage 时不应标记估算");
    }
}
