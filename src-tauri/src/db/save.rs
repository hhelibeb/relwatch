use rusqlite::Connection;

use crate::db::releases;

/// 保存循环的统一条目视图。
///
/// 各适配器把各自的 JSON/强类型条目投影为该结构后交给 [`save_entries_generic`]，
/// "排序 → 去重 → 写入上限 / 已知区域停止" 的语义收敛到一处，任何修正只需改一个地方。
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

/// 连续命中多少条已入库条目即认为已扫进已知区域，停止本轮扫描。
///
/// 「首条去重命中就停」是不能用的停止判据：一轮内发布多条时，上一轮往往已经抓走了
/// 最新那条，本轮首条即命中 → 立即停止 → 同轮较旧的新条目留在库外；下一轮仍在它
/// 前面命中那一条而再次停止，**缺口永不回补**。跨过若干已知条目才能把缺口补上，
/// 而 3 条的余量足够覆盖「已知条目穿插在中间」的正常形态，又不至于每轮全页重扫。
pub(crate) const KNOWN_HIT_STOP: usize = 3;

/// 通用保存循环：按 published 降序逐条 insert，新条目计入 saved 并触发
/// `on_inserted`；去重命中触发 `on_duplicate`。真正的 DB 错误只记日志并中断本轮。
///
/// 模式语义（youtube/bilibili/github 三份 save 共用）：
/// - `max_count` 只限制**本轮写入条数**，不参与扫描停止判断
/// - 扫描停止只看 [`KNOWN_HIT_STOP`]；上限命中时剩余较旧条目本轮不写，
///   但下一轮仍会被重新扫到（不会永久丢失）
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
    let mut known_hits: usize = 0;
    let inserted_any = std::cell::Cell::new(false);
    // 循环以任意方式结束（耗尽 / 写入上限命中 / 连续已知条目停止）后统一收尾：
    // 本轮确有新插入时对该 source 全链重算一次 version_bump
    // （逐条全链重算会退化为 O(N²)，改成批量结束一次、最终态等价）。
    let finalize = || {
        if inserted_any.get() {
            // 重算失败仅记日志、不回滚已提交的插入：version_bump 是派生列（可重算），
            // 留 NULL 不影响 release 本身，属有意取舍（失败不丢数据）。
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
                known_hits = 0;
                inserted_any.set(true);
                on_inserted(conn, id, entry);
                saved.push((id, entry.body.clone()));
                if max_count > 0 && saved.len() >= max_count {
                    finalize();
                    return saved;
                }
                continue;
            }
            // 已入库（去重命中，UNIQUE(source_id, tag_name)）：交给适配器刷新元数据。
            // 负值理论不可达，一并按去重命中处理。
            Ok(_) => {
                on_duplicate(conn, source_id, entry);
                known_hits += 1;
                if known_hits >= KNOWN_HIT_STOP {
                    break;
                }
            }
            // 真正的 DB 错误：记日志并中断本轮，不触发 on_duplicate（否则会把故障误当
            // 已存在去刷写元数据），也不清 known_hits——避免故障期间继续向更旧的条目扫描。
            Err(e) => {
                log::error!("insert_release failed (source_id={}, tag={}): {}", source_id, entry.tag, e);
                break;
            }
        }
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

    fn tags(conn: &rusqlite::Connection) -> Vec<String> {
        let mut v: Vec<String> = releases::get_releases_with_state(conn)
            .unwrap()
            .into_iter()
            .map(|r| r.tag_name)
            .collect();
        v.sort();
        v
    }

    /// 回归（一轮多条时较旧的新条目被永久丢弃）：上一轮受写入上限只抓走最新那条，
    /// 本轮首条即去重命中——旧实现在此立即返回空，v1.2.0 此后每轮都排在已知的
    /// v1.3.0 之后，永不入库。停止判据改为连续已知命中后，这类缺口能在下一轮自动补上。
    #[test]
    fn test_older_unsaved_release_below_known_newest_is_backfilled() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let window = || {
            vec![
                entry("v1.3.0", "2024-01-03T00:00:00Z"),
                entry("v1.2.0", "2024-01-02T00:00:00Z"),
            ]
        };
        // 上一轮：写入上限 1，只落最新的 v1.3.0
        assert_eq!(
            save_entries_generic(&conn, sid, &window(), 1, |_, _, _| {}, |_, _, _| {}).len(),
            1
        );
        assert_eq!(tags(&conn), vec!["v1.3.0"]);

        // 本轮：v1.3.0 去重命中（旧实现于此停止），v1.2.0 必须被补写
        let saved = save_entries_generic(&conn, sid, &window(), 0, |_, _, _| {}, |_, _, _| {});
        assert_eq!(saved.len(), 1);
        assert_eq!(tags(&conn), vec!["v1.2.0", "v1.3.0"]);
        // 补写的旧条目也要有升型基线（v1.3.0 相对 v1.2.0 = minor）
        assert_eq!(version_bump_of(&conn, "v1.2.0"), None);
        assert_eq!(version_bump_of(&conn, "v1.3.0").as_deref(), Some("minor"));
    }

    /// 写入上限不再兼任停止判据：上限命中时较旧条目本轮不写，但下一轮仍会被扫到。
    #[test]
    fn test_write_cap_does_not_permanently_hide_older_entries() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let window = vec![
            entry("v3", "2024-01-03T00:00:00Z"),
            entry("v2", "2024-01-02T00:00:00Z"),
            entry("v1", "2024-01-01T00:00:00Z"),
        ];
        save_entries_generic(&conn, sid, &window, 1, |_, _, _| {}, |_, _, _| {});
        assert_eq!(tags(&conn), vec!["v3"]);

        // 逐轮各写一条， backlog 应能排空而不是只剩 v3
        save_entries_generic(&conn, sid, &window, 1, |_, _, _| {}, |_, _, _| {});
        save_entries_generic(&conn, sid, &window, 1, |_, _, _| {}, |_, _, _| {});
        assert_eq!(tags(&conn), vec!["v1", "v2", "v3"]);
    }

    /// 连续 [`KNOWN_HIT_STOP`] 条已知即停止扫描：藏得更旧的条目不每轮重扫（成本上界）。
    #[test]
    fn test_scan_stops_after_three_consecutive_known_hits() {
        let conn = init_memory_db().unwrap();
        let sid = sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let known = vec![
            entry("v5", "2024-01-05T00:00:00Z"),
            entry("v4", "2024-01-04T00:00:00Z"),
            entry("v3", "2024-01-03T00:00:00Z"),
        ];
        save_entries_generic(&conn, sid, &known, 0, |_, _, _| {}, |_, _, _| {});

        let mut window = vec![
            entry("v5", "2024-01-05T00:00:00Z"),
            entry("v4", "2024-01-04T00:00:00Z"),
            entry("v3", "2024-01-03T00:00:00Z"),
            entry("v2", "2024-01-02T00:00:00Z"),
        ];
        let saved = save_entries_generic(&conn, sid, &window, 0, |_, _, _| {}, |_, _, _| {});
        assert!(saved.is_empty());
        assert_eq!(tags(&conn), vec!["v3", "v4", "v5"]);

        // 已知区域只剩 2 条命中时不停止：更旧的 v2 仍会被补写（余量覆盖穿插形态）
        window.remove(0);
        let saved = save_entries_generic(&conn, sid, &window, 0, |_, _, _| {}, |_, _, _| {});
        assert_eq!(saved.len(), 1);
        assert_eq!(tags(&conn), vec!["v2", "v3", "v4", "v5"]);
    }
}
