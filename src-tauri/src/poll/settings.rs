use crate::db;
use crate::db::settings::{KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT};
use crate::source;

/// 根据 fetch_history 配置计算分页参数。
///
/// 返回 `(max_count, per_page, needs_pagination)`：
/// - `max_count`: `None` 表示不设上限（全量分页），`Some(n)` 表示最多拉取 n 条
/// - `per_page`: 每次 API 调用的页面大小
/// - `needs_pagination`: 是否需要翻页（true = 多次 API 调用）
pub(super) fn compute_fetch_plan(
    fetch_history: bool,
    is_first_query: bool,
    fetch_history_count: usize,
) -> (Option<usize>, usize, bool) {
    if fetch_history && is_first_query && (fetch_history_count == 0 || fetch_history_count > 100) {
        // 0 = 全量分页（None），>100 = 受限分页（前端限制 max=100，此分支作为安全兜底）
        let max_count = if fetch_history_count == 0 { None } else { Some(fetch_history_count) };
        (max_count, 100, true)
    } else if fetch_history && is_first_query {
        // 小数量，单次 API 调用即可
        (Some(fetch_history_count), (fetch_history_count + 5).clamp(10, 100), false)
    } else {
        // 增量模式不设写入上限：`fetch_history_count` 按设置项文案（「历史版本数量」）只
        // 约束首次/历史拉取，拿它兼任每轮写入上限会把同一周期内第二条起的新版本全部挡在
        // 库外（配合旧的「首条去重命中即停止」更会形成永不回补的缺口）。单页 10 条已是
        // 每轮成本上界，无需再截断。
        (None, 10, false)
    }
}

// ---- 轮询设置 ----------------

pub(super) struct PollSettings {
    pub(super) proxy: db::settings::ProxyConfig,
    pub(super) credentials: source::SourceCredentials,
    pub(super) fetch_history: bool,
    pub(super) fetch_history_count: usize,
}

impl Default for PollSettings {
    fn default() -> Self {
        Self {
            proxy: db::settings::ProxyConfig::default(),
            credentials: source::SourceCredentials::default(),
            fetch_history: false,
            fetch_history_count: 1,
        }
    }
}

pub(super) fn load_poll_settings(conn: &rusqlite::Connection) -> Result<PollSettings, String> {
    let proxy = db::settings::load_proxy_config(conn)?;
    let credentials = source::SourceCredentials::load(conn);
    let fetch_history =
        db::settings::get_setting_bool(conn, KEY_FETCH_HISTORY, false).unwrap_or(false);
    let fetch_history_count =
        db::settings::get_setting_i64(conn, KEY_FETCH_HISTORY_COUNT, 1)
            .unwrap_or(1)
            .max(0) as usize;
    Ok(PollSettings {
        proxy,
        credentials,
        fetch_history,
        fetch_history_count,
    })
}

