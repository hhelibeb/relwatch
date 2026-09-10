use rusqlite::Connection;

use crate::db::releases;

/// 保存循环的统一条目视图。
///
/// 各适配器把各自的 JSON/强类型条目投影为该结构后交给 [`save_entries_generic`]，
/// "排序 → 去重 → max_count 早退" 的语义收敛到一处，任何修正只需改一个地方。
pub struct SaveEntry {
    /// 唯一标识（GitHub tag_name / YouTube video_id / B 站 bvid），用于去重。
    pub tag: String,
    /// 展示名（release_name / 视频标题）。
    pub name: String,
    pub html_url: String,
    /// RFC3339 发布时间，保存循环按它降序处理。
    pub published: String,
    pub prerelease: bool,
    pub body: Option<String>,
    /// 适配器附加元数据（播放量/封面/时长等），插入成功与去重命中时都会刷写。
    pub metadata: Option<String>,
}

/// 通用保存循环：按 published 降序逐条 insert，新条目计入 saved 并触发
/// `on_inserted`；去重命中触发 `on_duplicate`。插入错误记录日志（H-1），
/// 不再被当作去重命中吞掉。
///
/// 语义与历史实现逐字节对齐（youtube/bilibili/github 三份 save 的原行为）：
/// - 普通模式（max_count=1）遇到已入库记录立即返回空
/// - 历史模式（max_count>1）跳过已存在记录继续找更新内容
/// - `max_count=0` 表示不设上限
pub fn save_entries_generic(
    conn: &Connection,
    source_id: i64,
    entries: &[SaveEntry],
    max_count: usize,
    mut on_inserted: impl FnMut(&Connection, i64, &SaveEntry),
    mut on_duplicate: impl FnMut(&Connection, i64, &SaveEntry),
) -> Vec<(i64, Option<String>)> {
    let mut sorted: Vec<&SaveEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| b.published.cmp(&a.published));

    let mut saved = Vec::new();
    let inserted_any = std::cell::Cell::new(false);
    // 循环以任意方式结束（耗尽 / max_count 早退 / 普通模式遇已存在）后统一收尾：
    // 本轮确有新插入时对该 source 全链重算一次 version_bump。
    // 原实现是每条 insert_release 内部各自全链重算（历史模式首拉 N 条 → O(N²)）；
    // 改为批量结束一次，语义最终态等价。
    let finalize = || {
        if inserted_any.get() {
            // 重算失败仅记日志、不回滚已提交的插入：version_bump 是派生列（可重算），
            // 留 NULL 不影响 release 本身。相比旧实现（recompute 在 insert_release 事务内，
            // 失败连带回滚整条 release），此处失败不会丢数据，属有意取舍。
            //
            // 「补算」的边界（勿高估自愈）：本收尾由 inserted_any 门控，只有该 source
            // 本轮**确有新插入**才重算，全去重命中的轮次不补。因此重算失败、或
            // 「插入已提交、重算前崩溃」留下的 NULL，会保留到该 source 下次出现新版本
            // （届时全链重算覆盖该源全部行），期间版本类型筛选（major/minor/patch）
            // 会漏掉这些行。更强的自愈需启动期对 version_bump IS NULL 的行一次性重算
            // （当前未做）。
            if let Err(e) = releases::recompute_version_bumps(conn, source_id) {
                log::error!(
                    "recompute_version_bumps failed (source_id={}): {}",
                    source_id,
                    e
                );
            }
        }
    };
    for entry in sorted {
        match releases::insert_release(
            conn,
            source_id,
            &entry.tag,
            &entry.name,
            &entry.html_url,
            &entry.published,
            entry.prerelease,
            entry.body.as_deref(),
        ) {
            Ok(id) if id > 0 => {
                inserted_any.set(true);
                on_inserted(conn, id, entry);
                saved.push((id, entry.body.clone()));
                if max_count > 0 && saved.len() >= max_count {
                    finalize();
                    return saved;
                }
                continue;
            }
            // 已入库（去重命中，UNIQUE(source_id, tag_name)）：交给适配器刷新元数据
            Ok(0) => {
                on_duplicate(conn, source_id, entry);
            }
            // 理论不可达（insert_release 返回值非负）；防御性按去重命中处理保持原语义
            Ok(_) => {
                on_duplicate(conn, source_id, entry);
            }
            // 真正的 DB 错误：不再吞成「去重命中」（H-1）。记录日志，普通模式
            // 与去重命中同样中断本轮（保持原流程控制），但不再触发 on_duplicate
            // 把故障误当已存在去刷写元数据，便于从日志定位根因。
            Err(e) => {
                log::error!("insert_release failed (source_id={}, tag={}): {}", source_id, entry.tag, e);
            }
        }
        // 已入库且普通模式（max_count=1）时，说明不是新内容，停止
        if max_count == 1 {
            finalize();
            return vec![];
        }
        // 历史模式：已存在的跳过，继续找更新的新内容
    }
    finalize();
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;
    use crate::db::sources;

    fn entry(tag: &str, published: &str) -> SaveEntry {
        SaveEntry {
            tag: tag.to_string(),
            name: tag.to_string(),
            html_url: format!("https://x/{}", tag),
            published: published.to_string(),
            prerelease: false,
            body: None,
            metadata: None,
        }
    }

    fn version_bump_of(conn: &rusqlite::Connection, tag: &str) -> Option<String> {
        releases::get_releases_with_state(conn)
            .unwrap()
            .into_iter()
            .find(|r| r.tag_name == tag)
            .and_then(|r| r.version_bump)
    }

    /// 历史模式首拉（乱序 entries，函数内按 published 降序排序）：批量插入结束后
    /// 应恰好触发一次全链重算，version_bump 与逐条重算语义一致。
    #[test]
    fn test_batch_save_recomputes_version_bump_once() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        // 故意乱序：先给旧版本，再给新版本（真实 API 通常已降序，此处验证排序收敛）
        let entries = vec![
            entry("v1.0.0", "2024-01-01T00:00:00Z"),
            entry("v1.2.0", "2024-01-02T00:00:00Z"),
            entry("v1.2.1", "2024-01-03T00:00:00Z"),
            entry("v2.0.0", "2024-02-01T00:00:00Z"),
        ];
        let saved = save_entries_generic(&conn, sid, &entries, 0, |_, _, _| {}, |_, _, _| {});
        assert_eq!(saved.len(), 4);

        // 最早无基线 → NULL；v1.2.0=minor；v1.2.1=patch；v2.0.0=major
        assert_eq!(version_bump_of(&conn, "v1.0.0"), None);
        assert_eq!(version_bump_of(&conn, "v1.2.0").as_deref(), Some("minor"));
        assert_eq!(version_bump_of(&conn, "v1.2.1").as_deref(), Some("patch"));
        assert_eq!(version_bump_of(&conn, "v2.0.0").as_deref(), Some("major"));
    }

    /// 无新增（全部去重命中）时不触发重算也不报错，链上值保持上一轮结果。
    #[test]
    fn test_batch_save_all_duplicates_keeps_existing_bumps() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let entries = vec![
            entry("v1.0.0", "2024-01-01T00:00:00Z"),
            entry("v1.1.0", "2024-01-02T00:00:00Z"),
        ];
        save_entries_generic(&conn, sid, &entries, 0, |_, _, _| {}, |_, _, _| {});
        assert_eq!(version_bump_of(&conn, "v1.1.0").as_deref(), Some("minor"));

        // 第二轮完全相同 → 全部去重命中，inserted_any=false → 跳过重算（幂等）
        let saved = save_entries_generic(&conn, sid, &entries, 0, |_, _, _| {}, |_, _, _| {});
        assert!(saved.is_empty());
        assert_eq!(version_bump_of(&conn, "v1.1.0").as_deref(), Some("minor"));
    }

    /// 历史模式 max_count 早退：先插较新的两条就返回，收尾重算仍应正确（较新两条有基线）。
    #[test]
    fn test_batch_save_max_count_early_return_recomputes() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let entries = vec![
            entry("v1.0.0", "2024-01-01T00:00:00Z"),
            entry("v1.1.0", "2024-01-02T00:00:00Z"),
            entry("v1.2.0", "2024-01-03T00:00:00Z"),
        ];
        // max_count=2：只插 v1.2.0、v1.1.0 两条（降序先插新的）；早退出口也要正确收尾。
        // 库中此时仅这两条：升序重算后 v1.1.0 无更早基线 → NULL，v1.2.0 相对 v1.1.0 → minor
        let saved = save_entries_generic(&conn, sid, &entries, 2, |_, _, _| {}, |_, _, _| {});
        assert_eq!(saved.len(), 2);
        assert_eq!(version_bump_of(&conn, "v1.1.0"), None);
        assert_eq!(version_bump_of(&conn, "v1.2.0").as_deref(), Some("minor"));
    }
}
