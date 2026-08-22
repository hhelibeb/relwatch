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
                on_inserted(conn, id, entry);
                saved.push((id, entry.body.clone()));
                if max_count > 0 && saved.len() >= max_count {
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
            return vec![];
        }
        // 历史模式：已存在的跳过，继续找更新的新内容
    }
    saved
}
