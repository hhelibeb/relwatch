use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri::Emitter;

/// 连续失败超过此次数后自动禁用监控源，防止无限重试
const MAX_CONSECUTIVE_FAILURES: i64 = 3;

use crate::db;
use crate::db::settings::{
    KEY_POLL_INTERVAL, KEY_PROXY_URL, KEY_PROXY_MODE, KEY_LOG_RETENTION, KEY_GITHUB_TOKEN, KEY_NEXT_POLL_AT,
    KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT, KEY_YOUTUBE_API_KEY, KEY_BILIBILI_COOKIE,
};
use crate::crypto;
use crate::http;
use crate::deepseek;
use crate::source;
use serde_json::json;
use crate::types::{AppState, PollResult, ReleaseNotifyParams};

static POLL_LOCK: AtomicBool = AtomicBool::new(false);
static POLL_RUNNING: AtomicBool = AtomicBool::new(true);
const MAX_CONCURRENCY: usize = 10;
/// 单源网络拉取超时（秒）：约束每个源的 fetch（含适配器内部重试）总耗时，
/// 防止某源接口异常（如 B 站 offset 死循环的兜底）无限挂起、占用信号量 permit
/// 拖住整轮轮询（F5）。超时按临时故障处理：记 check.failed 后跳过本轮，下轮重试。
const SOURCE_FETCH_TIMEOUT_SECS: u64 = 300;

pub fn is_poll_running() -> bool {
    POLL_RUNNING.load(Ordering::Relaxed)
}

pub fn stop_poll() {
    POLL_RUNNING.store(false, Ordering::SeqCst);
}

pub(crate) struct PollGuard;
impl Drop for PollGuard {
    fn drop(&mut self) {
        POLL_LOCK.store(false, Ordering::Release);
    }
}

pub(crate) fn acquire_lock() -> Result<PollGuard, String> {
    if POLL_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err("err.poll_in_progress".to_string());
    }
    Ok(PollGuard)
}

fn get_github_token(conn: &rusqlite::Connection) -> Option<String> {
    let encrypted = db::settings::get_setting(conn, KEY_GITHUB_TOKEN).ok()??;
    if encrypted.is_empty() {
        return None;
    }
    let (plain, new_v2) = crypto::decrypt_with_migration(&encrypted)?;
    if let Some(new_val) = new_v2 {
        if let Err(e) = db::settings::set_setting(conn, KEY_GITHUB_TOKEN, &new_val) {
            log::warn!("迁移 v1→v2 GitHub Token 回写失败: {}", e);
        }
    }
    Some(plain)
}

/// 读取解密后的 YouTube Data API Key（含 v1→v2 加密迁移回写）。
fn get_youtube_api_key(conn: &rusqlite::Connection) -> Option<String> {
    let encrypted = db::settings::get_setting(conn, KEY_YOUTUBE_API_KEY).ok()??;
    if encrypted.is_empty() {
        return None;
    }
    let (plain, new_v2) = crypto::decrypt_with_migration(&encrypted)?;
    if let Some(new_val) = new_v2 {
        if let Err(e) = db::settings::set_setting(conn, KEY_YOUTUBE_API_KEY, &new_val) {
            log::warn!("迁移 v1→v2 YouTube API Key 回写失败: {}", e);
        }
    }
    Some(plain)
}

/// 读取解密后的 B 站登录 Cookie（SESSDATA，含 v1→v2 加密迁移回写）。
fn get_bilibili_cookie(conn: &rusqlite::Connection) -> Option<String> {
    let encrypted = db::settings::get_setting(conn, KEY_BILIBILI_COOKIE).ok()??;
    if encrypted.is_empty() {
        return None;
    }
    let (plain, new_v2) = crypto::decrypt_with_migration(&encrypted)?;
    if let Some(new_val) = new_v2 {
        if let Err(e) = db::settings::set_setting(conn, KEY_BILIBILI_COOKIE, &new_val) {
            log::warn!("迁移 v1→v2 B 站 Cookie 回写失败: {}", e);
        }
    }
    Some(plain)
}

/// 根据 fetch_history 配置计算分页参数。
///
/// 返回 `(max_count, per_page, needs_pagination)`：
/// - `max_count`: `None` 表示不设上限（全量分页），`Some(n)` 表示最多拉取 n 条
/// - `per_page`: 每次 API 调用的页面大小
/// - `needs_pagination`: 是否需要翻页（true = 多次 API 调用）
fn compute_fetch_plan(
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
        // 增量模式：max_count=0（“拉取全部”）时 save 不截断（None），
        // 避免 fetch_history_count=0 导致 save 阶段只保存 1 条。
        let max_count = if fetch_history_count == 0 { None } else { Some(fetch_history_count) };
        (max_count, 10, false)
    }
}

// ---- 轮询设置 ----------------

/// 轮询操作所需的所有设置项，一次性从 DB 读取。
struct PollSettings {
    proxy_url: String,
    proxy_mode: String,
    github_token: Option<String>,
    youtube_api_key: Option<String>,
    bilibili_cookie: Option<String>,
    fetch_history: bool,
    fetch_history_count: usize,
}

impl Default for PollSettings {
    fn default() -> Self {
        Self {
            proxy_url: String::new(),
            proxy_mode: "none".to_string(),
            github_token: None,
            youtube_api_key: None,
            bilibili_cookie: None,
            fetch_history: false,
            fetch_history_count: 1,
        }
    }
}

/// 从 DB 加载轮询设置，返回 Result（在 ? 可用的上下文中使用）。
fn load_poll_settings(conn: &rusqlite::Connection) -> Result<PollSettings, String> {
    let proxy_url = db::settings::get_setting(conn, KEY_PROXY_URL)?.unwrap_or_default();
    let proxy_mode = db::settings::get_setting(conn, KEY_PROXY_MODE)?.unwrap_or_else(|| {
        if proxy_url.is_empty() {
            "none".to_string()
        } else {
            "custom".to_string()
        }
    });
    let github_token = get_github_token(conn);
    let youtube_api_key = get_youtube_api_key(conn);
    let bilibili_cookie = get_bilibili_cookie(conn);
    let fetch_history =
        db::settings::get_setting_bool(conn, KEY_FETCH_HISTORY, false).unwrap_or(false);
    let fetch_history_count =
        db::settings::get_setting_i64(conn, KEY_FETCH_HISTORY_COUNT, 1)
            .unwrap_or(1)
            .max(0) as usize;
    Ok(PollSettings {
        proxy_url,
        proxy_mode,
        github_token,
        youtube_api_key,
        bilibili_cookie,
        fetch_history,
        fetch_history_count,
    })
}

/// 将新保存的 releases 中非最新版本标记为"已读"。
///
/// - 如果库中已有比最新 release 更新的版本 → 全部标记已读
/// - 否则跳过最新一条，其余标记已读
fn mark_older_as_read(conn: &rusqlite::Connection, source_id: i64, saved: &[(i64, Option<String>)]) {
    if saved.is_empty() {
        return;
    }
    let latest_id = saved[0].0;
    let has_newer = db::releases::get_release(conn, latest_id)
        .ok()
        .flatten()
        .and_then(|r| db::releases::has_newer_release(conn, source_id, &r.published_at).ok())
        .unwrap_or(false);
    if has_newer {
        for (id, _) in saved {
            let _ = db::releases::set_notification_state(conn, *id, "clicked", None);
        }
    } else if saved.len() > 1 {
        for (id, _) in saved.iter().skip(1) {
            let _ = db::releases::set_notification_state(conn, *id, "clicked", None);
        }
    }
}

/// Post-save 事务性步骤：mark_older_as_read → record_check_success → 返回 (ids, saved)。
/// 从 `poll_all_sources_async` 和 `check_single_source` 的 spawn_blocking 闭包中提取，
/// 便于直接测试真实代码路径，消除 `simulate_fetch_save_mark_record` 等价副本。
fn post_save_mark_record(
    conn: &rusqlite::Connection,
    source_id: i64,
    saved: &[(i64, Option<String>)],
) -> (Vec<i64>, Vec<(i64, Option<String>)>) {
    if !saved.is_empty() {
        mark_older_as_read(conn, source_id, saved);
    }
    let ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();
    let new_count = ids.len();
    let _ = db::sources::record_check_success(conn, source_id, new_count);
    (ids, saved.to_vec())
}

/// 收集不参与 AI 摘要/翻译的源类型（如 youtube/bilibili）。
///
/// 由 `list_adapters()` 的能力声明 `ai_eligible()=false` 动态枚举，
/// 新增源类型在适配器登记后自动生效，即时路径与重试路径共用同一份能力声明，
/// 避免 SQL 硬编码排除集合遗漏新类型（如 bilibili）。
fn ai_excluded_types() -> Vec<&'static str> {
    source::list_adapters()
        .iter()
        .filter(|a| !a.ai_eligible())
        .map(|a| a.source_type())
        .collect()
}

/// 过滤掉无需 AI 摘要/翻译的源（youtube/bilibili）的 saved 条目。
/// 用户明确要求此类源不生成 DeepSeek 摘要，这里在调用
/// generate_summaries_for_new / generate_translations_for_new 前做过滤。
///
/// fail-closed：DB 错误或 spawn panic 返回 Err，由调用方跳过本轮 AI 生成并记日志，
/// 绝不因过滤失败而把不参与 AI 的条目放行给 DeepSeek。
async fn filter_ai_eligible(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    saved: &[(i64, Option<String>)],
) -> Result<Vec<(i64, Option<String>)>, String> {
    if saved.is_empty() {
        return Ok(vec![]);
    }
    // 按注册表能力枚举不参与 AI 摘要/翻译的源类型（当前：youtube/bilibili），
    // 新增源类型在 list_adapters 登记并声明 ai_eligible=false 后自动生效，
    // 无需在 DB 层或此处逐个 source_type 特判。
    let ineligible_types: Vec<&'static str> = ai_excluded_types();
    if ineligible_types.is_empty() {
        return Ok(saved.to_vec());
    }
    let ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();
    let pool = db_pool.clone();
    let excluded_ids: std::collections::HashSet<i64> = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_lock|{}", e))?;
        db::releases::ai_ineligible_release_ids(&conn, &ids, &ineligible_types)
    })
    .await
    .map_err(|e| format!("err.filter_ai_eligible_panic|{}", e))??;
    Ok(saved
        .iter()
        .filter(|(id, _)| !excluded_ids.contains(id))
        .cloned()
        .collect())
}

pub async fn trigger_poll(app: tauri::AppHandle) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;
    do_trigger_poll_async(app).await
}

/// 两个 do_*_poll_async 函数的公共核心：拉取所有源 → AI 摘要 → 通知 → 重试失败摘要
#[allow(clippy::too_many_arguments)]
async fn do_poll_core(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &dyn crate::types::Emitter,
    sources: &[db::sources::Source],
    proxy_url: &str,
    proxy_mode: &str,
    github_token: Option<&str>,
    youtube_api_key: Option<&str>,
    bilibili_cookie: Option<&str>,
    fetch_history: bool,
    fetch_history_count: usize,
    is_manual: bool,
) -> (Vec<i64>, Vec<db::releases::ReleaseInfo>) {
    let (all_new_ids, all_saved) = poll_all_sources_async(
        db_pool, sources, proxy_url, proxy_mode, github_token, youtube_api_key, bilibili_cookie,
        fetch_history, fetch_history_count,
    )
    .await;

    // 跳过 ai_eligible=false 的源（youtube/bilibili）后生成摘要与译文。
    // fail-closed：过滤失败（DB 错误/panic）时跳过本轮 AI 并记日志，
    // 宁可下轮重试也不放行 youtube 条目给 DeepSeek。
    match filter_ai_eligible(db_pool, &all_saved).await {
        Ok(ai_saved) if !ai_saved.is_empty() => {
            deepseek::generate_summaries_for_new(db_pool, deepseek_semaphore, &ai_saved).await;
            deepseek::generate_translations_for_new(db_pool, deepseek_semaphore, &ai_saved, false).await;
        }
        Ok(_) => {}
        Err(e) => log::error!("err.filter_ai_eligible|{}", e),
    }

    let (_, new_releases) = collect_pending_and_notify(db_pool, emitter, &all_new_ids, is_manual).await;

    // 重试之前失败的 AI 摘要 / 译文：两份失败名单读取合并到一次 spawn_blocking，
    // 避免在 async fn 内同步 DB 调用阻塞 tokio worker（与 Phase 2 的 spawn_blocking 改造一致）。
    // 重试查询与即时路径共用同一份能力声明（ai_excluded_types），
    // 保证 youtube/bilibili 等 ai_eligible=false 的源绝不进入 DeepSeek 重试队列。
    let retry_pool = db_pool.clone();
    let excluded_types = ai_excluded_types();
    let (retry_releases, retry_translations) =
        tokio::task::spawn_blocking(move || {
            match retry_pool.get() {
                Ok(conn) => (
                    db::releases::get_releases_without_summary(&conn, &excluded_types).unwrap_or_default(),
                    db::releases::get_releases_without_translation(&conn, &excluded_types).unwrap_or_default(),
                ),
                Err(e) => {
                    log::error!("err.db_lock|{}", e);
                    (Vec::new(), Vec::new())
                }
            }
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("do_poll_core retry spawn_blocking panic: {}", e);
            (Vec::new(), Vec::new())
        });
    if !retry_releases.is_empty() {
        log::info!("正在重试 {} 个之前失败的 AI 摘要", retry_releases.len());
        deepseek::generate_summaries_for_new(db_pool, deepseek_semaphore, &retry_releases).await;
    }
    if !retry_translations.is_empty() {
        log::info!("正在重试 {} 个之前失败的 AI 译文", retry_translations.len());
        deepseek::generate_translations_for_new(db_pool, deepseek_semaphore, &retry_translations, false).await;
    }

    (all_new_ids, new_releases)
}

async fn do_trigger_poll_async(app: tauri::AppHandle) -> Result<PollResult, String> {
    let (sources, proxy_url, proxy_mode, github_token, youtube_api_key, bilibili_cookie, fetch_history, fetch_history_count);

    {
        let state = app.state::<AppState>();
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        sources = db::sources::list_sources(&conn)?
            .into_iter()
            .filter(|s| s.enabled)
            .collect::<Vec<_>>();
        let settings = load_poll_settings(&conn)?;
        proxy_url = settings.proxy_url;
        proxy_mode = settings.proxy_mode;
        github_token = settings.github_token;
        youtube_api_key = settings.youtube_api_key;
        bilibili_cookie = settings.bilibili_cookie;
        fetch_history = settings.fetch_history;
        fetch_history_count = settings.fetch_history_count;
    }

    let (db_pool, deepseek_semaphore) = {
        let state = app.state::<AppState>();
        (state.db.clone(), state.deepseek_semaphore.clone())
    };
    let emitter: &dyn crate::types::Emitter = &app;
    let (_, new_releases) = do_poll_core(
        &db_pool, &deepseek_semaphore, emitter, &sources, &proxy_url, &proxy_mode,
        github_token.as_deref(), youtube_api_key.as_deref(), bilibili_cookie.as_deref(), fetch_history, fetch_history_count, true,
    ).await;

    // 手动触发特有逻辑：更新 next_poll_at
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        let now = chrono::Utc::now().timestamp();
        let interval = db::settings::get_setting(&conn, KEY_POLL_INTERVAL)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(30);
        let next = now + interval * 60;
        state.next_poll_at.store(next, Ordering::Relaxed);
        let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
    }

    let _ = app.emit("poll-completed", ());

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}

pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;

    let (source_obj, proxy_url, proxy_mode, github_token, youtube_api_key, bilibili_cookie, fetch_history, fetch_history_count);
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        let sources = db::sources::list_sources(&conn)?;
        let source = sources
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("err.source_not_found")?;
        let settings = load_poll_settings(&conn)?;
        proxy_url = settings.proxy_url;
        proxy_mode = settings.proxy_mode;
        github_token = settings.github_token;
        youtube_api_key = settings.youtube_api_key;
        bilibili_cookie = settings.bilibili_cookie;
        fetch_history = settings.fetch_history;
        fetch_history_count = settings.fetch_history_count;
        source_obj = source;
    }

    let is_first_query = source_obj.last_checked_at.is_none()
        || source_obj.last_checked_at.as_deref() == Some("");
    // 日志展示标识：YouTube 源用频道名替代 channel_id（repo 置空），避免 `owner/` 残废格式
    let (log_owner, log_repo) = db::logs::source_log_ident(
        &source_obj.source_type,
        &source_obj.owner,
        &source_obj.repo,
        source_obj.description.as_deref(),
    );
    // source 分发收敛为 trait 调用；token 按适配器声明的 auth_kind 选取
    // （YouTube → Data API Key，GitHub → PAT，HF → None），无需逐个 source_type 特判。
    let adapter = match source::get_adapter(&source_obj.source_type) {
        Ok(a) => a,
        Err((_, msg)) => return Err(msg),
    };
    let token = source::token_for(
        adapter.as_ref(),
        github_token.as_deref(),
        youtube_api_key.as_deref(),
        bilibili_cookie.as_deref(),
    );
    // YouTube 源开启历史拉取时，每次检查都按 fetch_history_count 拉取历史：
    // save 阶段按 UNIQUE(source_id, tag_name) 去重跳过已存在条目，因此
    // 重新配置 Data API Key 后无需删除源即可补拉历史视频。RSS 模式单页拉取，不受影响。
    let history_query = fetch_history && (is_first_query || adapter.always_fetch_history());
    let (max_count, per_page, needs_pagination) = compute_fetch_plan(fetch_history, history_query, fetch_history_count);

    // 从 AppHandle 提取依赖，注入提纯后的核心函数
    let (db_pool, deepseek_semaphore) = {
        let state = app.state::<AppState>();
        (state.db.clone(), state.deepseek_semaphore.clone())
    };
    let emitter: &dyn crate::types::Emitter = &app;

    // client 不携带 default Authorization——github token 由 adapter 按请求设置，
    // 避免 HF 请求泄露 GitHub Token（见 http::HttpClientConfig::set_default_auth）
    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        bearer_token: None,
        ..Default::default()
    }) {
        Ok(client) => client,
        Err(e) => {
            if let Ok(conn) = db_pool.get() {
                let _ = db::sources::record_check_failure(&conn, source_obj.id, &e);
                db::logs::write_log_key(
                    &conn,
                    "WARN",
                    "check.failed",
                    &json!({"owner": &log_owner, "repo": &log_repo, "error": &e}).to_string(),
                );
            }
            return Err(e);
        }
    };

    // 单源超时保护：与轮询路径一致，fetch（含适配器内部重试）整体约束在
    // SOURCE_FETCH_TIMEOUT_SECS 内，超时转临时故障错误走统一失败分支。
    let releases = match tokio::time::timeout(
        std::time::Duration::from_secs(SOURCE_FETCH_TIMEOUT_SECS),
        async {
            if needs_pagination {
                adapter.fetch_all(&client, &source_obj, max_count, token).await
            } else {
                adapter.fetch(&client, &source_obj, per_page, token).await
            }
        },
    )
    .await
    .unwrap_or_else(|_| Err((0, format!("err.source_timeout|{}", SOURCE_FETCH_TIMEOUT_SECS))))
    {
        Ok(releases) => releases,
        Err((status, msg)) => {
            if let Ok(conn) = db_pool.get() {
                let _ = db::sources::record_check_failure(&conn, source_obj.id, &msg);
                // 网络错误(0)、认证/限流(401/403/429)、服务端错误(5xx) 均为临时性，记为 WARN
                let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 { "WARN" } else { "ERROR" };
                db::logs::write_log_key(
                    &conn,
                    level,
                    "check.failed",
                    &json!({"owner": &log_owner, "repo": &log_repo, "error": &msg}).to_string(),
                );
            }
            return Err(msg);
        }
    };
    // save 统一走 trait，吸收 github 同步 / HF 异步三阶段差异
    let saved: Vec<(i64, Option<String>)> = adapter.save(&db_pool, &source_obj, &releases, max_count.unwrap_or(usize::MAX), &client).await;
    // save 之后的同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
    let db_pool_blk = db_pool.clone();
    let source_id = source_obj.id;
    let owner = log_owner;
    let repo = log_repo;
    let result = tokio::task::spawn_blocking(move || {
        let conn = match db_pool_blk.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("err.db_lock|{}", e);
                return (vec![], saved);
            }
        };
        let (ids, saved) = post_save_mark_record(&conn, source_id, &saved);
        let new_count = ids.len();
        db::logs::write_log_key(
            &conn,
            "INFO",
            "check.manual",
            &json!({"owner": &owner, "repo": &repo, "count": new_count}).to_string(),
        );
        (ids, saved)
    })
    .await;
    let (new_ids, saved) = match result {
        Ok((ids, saved)) => (ids, saved),
        Err(e) => {
            log::error!("check_single_source post-save spawn_blocking panic: {}", e);
            (vec![], vec![])
        }
    };

    // 检查成功后按源类型能力刷新描述（GitHub 仓库描述 / YouTube 真实频道名），
    // 统一走 adapter.verify_and_describe，无需逐个 source_type 特判。
    if adapter.refresh_description_after_check() {
        if let Ok(desc) = adapter
            .verify_and_describe(&client, &source_obj.owner, &source_obj.repo, token)
            .await
        {
            let db_pool_blk = db_pool.clone();
            let source_id = source_obj.id;
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(conn) = db_pool_blk.get() {
                    let _ = db::sources::update_source_description(&conn, source_id, &desc);
                }
            })
            .await;
        }
    }

    // 跳过 ai_eligible=false 的源（youtube/bilibili）后生成摘要与译文。
    // fail-closed：过滤失败（DB 错误/panic）时跳过本轮 AI 并记日志。
    match filter_ai_eligible(&db_pool, &saved).await {
        Ok(ai_saved) if !ai_saved.is_empty() => {
            deepseek::generate_summaries_for_new(&db_pool, &deepseek_semaphore, &ai_saved).await;
            deepseek::generate_translations_for_new(&db_pool, &deepseek_semaphore, &ai_saved, false).await;
        }
        Ok(_) => {}
        Err(e) => log::error!("err.filter_ai_eligible|{}", e),
    }

    let (_, new_releases) = collect_pending_and_notify(&db_pool, emitter, &new_ids, false).await;

    let _ = app.emit("poll-completed", ());

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}

pub fn trigger_poll_async(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = trigger_poll(app).await;
    });
}

async fn do_poll_async(app: tauri::AppHandle) {
    if POLL_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let _guard = PollGuard;

    if !is_poll_running() {
        return;
    }

    let (sources, proxy_url, proxy_mode, retention_days, github_token, youtube_api_key, bilibili_cookie, fetch_history, fetch_history_count);
    {
        let state = app.state::<AppState>();
        let conn = match state.db.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("err.db_lock|{}", e);
                return;
            }
        };
        sources = db::sources::list_sources(&conn).unwrap_or_default();
        let settings = load_poll_settings(&conn).unwrap_or_default();
        proxy_url = settings.proxy_url;
        proxy_mode = settings.proxy_mode;
        github_token = settings.github_token;
        youtube_api_key = settings.youtube_api_key;
        bilibili_cookie = settings.bilibili_cookie;
        fetch_history = settings.fetch_history;
        fetch_history_count = settings.fetch_history_count;
        retention_days = db::settings::get_setting(&conn, KEY_LOG_RETENTION)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if retention_days > 0 {
            db::logs::delete_old_logs(&conn, retention_days);
        }
    }

    // 自动禁用连续失败过多的监控源（断路器）
    let disabled_ids: Vec<i64> = {
        let state = app.state::<AppState>();
        let mut ids = Vec::new();
        if let Ok(conn) = state.db.get() {
            for source in &sources {
                if source.enabled && source.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    let _ = db::sources::update_source(&conn, source.id, false, source.poll_interval_minutes);
                    let (log_owner, log_repo) = db::logs::source_log_ident(&source.source_type, &source.owner, &source.repo, source.description.as_deref());
                    db::logs::write_log_key(
                        &conn,
                        "WARN",
                        "source.log_auto_disabled",
                        &serde_json::json!({"owner": &log_owner, "repo": &log_repo, "id": source.id}).to_string(),
                    );
                    log::warn!(
                        "自动禁用 {}/{} (id={})：连续失败 {} 次",
                        source.owner,
                        source.repo,
                        source.id,
                        source.consecutive_failures
                    );
                    let _ = app.emit("source-auto-disabled", serde_json::json!({
                        "owner": &source.owner,
                        "repo": &source.repo,
                        "id": source.id,
                        "failures": source.consecutive_failures,
                    }));
                    ids.push(source.id);
                }
            }
        }
        ids
    };

    let enabled: Vec<db::sources::Source> =
        sources.into_iter().filter(|s| s.enabled && !disabled_ids.contains(&s.id)).collect();
    if enabled.is_empty() {
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.get() {
            db::logs::write_log_key(&conn, "INFO", "check.skipped", "{}");
        }
        return;
    }

    {
        let state = app.state::<AppState>();
        let db_pool = state.db.clone();
        let semaphore = state.deepseek_semaphore.clone();
        let emitter: &dyn crate::types::Emitter = &app;
        do_poll_core(
            &db_pool, &semaphore, emitter, &enabled, &proxy_url, &proxy_mode,
            github_token.as_deref(), youtube_api_key.as_deref(), bilibili_cookie.as_deref(), fetch_history, fetch_history_count, false,
        ).await;
    }

    let _ = app.emit("poll-completed", ());
}

#[allow(clippy::too_many_arguments)]
async fn poll_all_sources_async(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    sources: &[db::sources::Source],
    proxy_url: &str,
    proxy_mode: &str,
    github_token: Option<&str>,
    youtube_api_key: Option<&str>,
    bilibili_cookie: Option<&str>,
    fetch_history: bool,
    fetch_history_count: usize,
) -> (Vec<i64>, Vec<(i64, Option<String>)>) {
    // client 不携带 default Authorization——github token 由 adapter 按请求设置，
    // 避免 HF 请求泄露 GitHub Token（见 http::HttpClientConfig::set_default_auth）
    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url,
        proxy_mode,
        bearer_token: None,
        ..Default::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            if let Ok(conn) = db_pool.get() {
                for source in sources {
                    let _ = db::sources::record_check_failure(&conn, source.id, &e);
                }
                db::logs::write_log_key(&conn, "ERROR", "check.http_client_error", &json!({"error": &e}).to_string());
            }
            return (vec![], vec![]);
        }
    };

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENCY));
    let mut handles = Vec::new();
    let pool = db_pool.clone();

    for source in sources {
        let sem = semaphore.clone();
        let client = client.clone();
        let source = source.clone();
        let db_pool = pool.clone();
        // source 分发收敛为 trait 调用；token 按适配器声明的 auth_kind 选取
        // （YouTube → Data API Key，GitHub → PAT，HF → None）。
        // spawn 需要 'static，先克隆为 owned。
        let adapter = match source::get_adapter(&source.source_type) {
            Ok(a) => a,
            Err((_, msg)) => {
                log::error!("{}", msg);
                continue;
            }
        };
        let token = source::token_for(adapter.as_ref(), github_token, youtube_api_key, bilibili_cookie)
            .map(|s| s.to_string());

        let is_first_query = source.last_checked_at.is_none()
            || source.last_checked_at.as_deref() == Some("");
        // 同 check_single_source：YouTube 源开启历史拉取时每次检查都按 count 拉历史
        // （API 模式翻页拉取 + save 去重，支持配 key 后补拉历史；RSS 模式单页不受影响）
        let history_query = fetch_history && (is_first_query || adapter.always_fetch_history());
        let (max_count, per_page, needs_pagination) = compute_fetch_plan(fetch_history, history_query, fetch_history_count);

        handles.push(tokio::spawn(async move {
            // 背压信号量获取改为 graceful：不再 expect 后被 handle.await 静默吞掉
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("err.sem_closed|{}", e);
                    return (vec![], vec![]);
                }
            };
            // 单源超时保护：fetch（含适配器内部重试）整体约束在 SOURCE_FETCH_TIMEOUT_SECS 内，
            // 超时后转成临时故障错误走统一的失败分支（记 check.failed），下轮自动重试。
            let fetch_result = tokio::time::timeout(
                std::time::Duration::from_secs(SOURCE_FETCH_TIMEOUT_SECS),
                async {
                    if needs_pagination {
                        adapter.fetch_all(&client, &source, max_count, token.as_deref()).await
                    } else {
                        adapter.fetch(&client, &source, per_page, token.as_deref()).await
                    }
                },
            )
            .await
            .unwrap_or_else(|_| Err((0, format!("err.source_timeout|{}", SOURCE_FETCH_TIMEOUT_SECS))));
            match fetch_result {
                Ok(releases) => {
                    // save 统一走 trait，吸收 github 同步 / HF 异步三阶段差异
                    let saved = adapter.save(&db_pool, &source, &releases, max_count.unwrap_or(usize::MAX), &client).await;
                    // save 之后的同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
                    let db_pool_blk = db_pool.clone();
                    let source_id = source.id;
                    let (owner, repo) = db::logs::source_log_ident(&source.source_type, &source.owner, &source.repo, source.description.as_deref());
                    let result = tokio::task::spawn_blocking(move || {
                        let conn = match db_pool_blk.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("err.db_lock|{}", e);
                                return (vec![], saved);
                            }
                        };
                        let (ids, saved) = post_save_mark_record(&conn, source_id, &saved);
                        let new_count = ids.len();
                        db::logs::write_log_key(
                            &conn,
                            "INFO",
                            "check.auto",
                            &json!({"owner": &owner, "repo": &repo, "count": new_count}).to_string(),
                        );
                        (ids, saved)
                    })
                    .await;
                    match result {
                        Ok((ids, saved)) => (ids, saved),
                        Err(e) => {
                            log::error!("poll post-save spawn_blocking panic: {}", e);
                            (vec![], vec![])
                        }
                    }
                }
                Err((status, msg)) => {
                    let db_pool_blk = db_pool.clone();
                    let source_id = source.id;
                    let (owner, repo) = db::logs::source_log_ident(&source.source_type, &source.owner, &source.repo, source.description.as_deref());
                    let level = if matches!(status, 0 | 401 | 403 | 429) || status >= 500 { "WARN" } else { "ERROR" };
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(conn) = db_pool_blk.get() {
                            let _ = db::sources::record_check_failure(&conn, source_id, &msg);
                            db::logs::write_log_key(
                                &conn,
                                level,
                                "check.failed",
                                &json!({"owner": &owner, "repo": &repo, "error": &msg}).to_string(),
                            );
                        }
                    })
                    .await;
                    (vec![], vec![])
                }
            }
        }));
    }

    let mut all_new_ids = Vec::new();
    let mut all_saved = Vec::new();
    for handle in handles {
        if let Ok((ids, saved)) = handle.await {
            all_new_ids.extend(ids);
            all_saved.extend(saved);
        }
    }

    (all_new_ids, all_saved)
}

async fn collect_pending_and_notify(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    emitter: &dyn crate::types::Emitter,
    new_ids: &[i64],
    is_manual: bool,
) -> (Vec<db::releases::ReleaseInfo>, Vec<db::releases::ReleaseInfo>) {
    // 一次性取连接：读 pending + muted，并把本轮 pending 批量标记已通知。
    // 同步 DB 调用收笼进 spawn_blocking，避免在 async 上下文阻塞 tokio worker。
    let pool = db_pool.clone();
    let (pending, muted_source_ids): (
        Vec<db::releases::ReleaseInfo>,
        std::collections::HashSet<i64>,
    ) = tokio::task::spawn_blocking(move || {
        let conn = match pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("err.db_lock|{}", e);
                return (Vec::new(), std::collections::HashSet::new());
            }
        };
        let pending = db::releases::get_pending_releases(&conn).unwrap_or_default();
        // 批量标记已通知：本轮 pending 全部置 last_notified_at，避免 N 次取连接
        for release in &pending {
            let _ = db::releases::set_last_notified_at(&conn, release.id);
        }
        let muted_source_ids = db::sources::list_muted_source_ids(&conn)
            .unwrap_or_default()
            .into_iter()
            .collect();
        (pending, muted_source_ids)
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("collect_pending spawn_blocking panic: {}", e);
        (Vec::new(), std::collections::HashSet::new())
    });

    // 派发桌面通知。Pending here means unread and eligible now, including expired snoozes.
    // 通过 Emitter trait 注入，可测上下文中替换为 NoopEmitter。
    for release in &pending {
        // 静默的源：只标记已通知，不发送桌面通知
        if muted_source_ids.contains(&release.source_id) {
            continue;
        }
        emitter.notify_release(ReleaseNotifyParams {
            release_id: release.id,
            html_url: release.html_url.clone(),
            owner: release.owner.clone(),
            repo: release.repo.clone(),
            tag: release.tag_name.clone(),
            name: release.release_name.clone(),
            importance: release.ai_importance.clone(),
        });
    }

    let new_releases: Vec<db::releases::ReleaseInfo> = pending
        .into_iter()
        .filter(|r| new_ids.contains(&r.id))
        .collect();

    // is_manual 日志 + 最终 pending 读取合并到一次 spawn_blocking
    let count = new_ids.len();
    let pool2 = db_pool.clone();
    let all_pending = tokio::task::spawn_blocking(move || {
        let conn = match pool2.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("err.db_lock|{}", e);
                return Vec::new();
            }
        };
        if is_manual {
            db::logs::write_log_key(
                &conn,
                "INFO",
                "check.manual_all_done",
                &json!({"count": count}).to_string(),
            );
        }
        db::releases::get_pending_releases(&conn).unwrap_or_default()
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("collect_pending tail spawn_blocking panic: {}", e);
        Vec::new()
    });

    (all_pending, new_releases)
}

pub fn start_poll_thread(app_handle: tauri::AppHandle, next_poll: std::sync::Arc<AtomicI64>) {
    tauri::async_runtime::spawn(async move {
        // Save the initial next_poll_at so restart can restore it
        let initial = next_poll.load(Ordering::Relaxed);
        if let Ok(conn) = app_handle.state::<AppState>().db.get() {
            let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &initial.to_string());
        }

        loop {
            let target = next_poll.load(Ordering::Relaxed);
            let now = chrono::Utc::now().timestamp();
            let sleep_secs = (target - now).max(0) as u64;

            for _ in 0..sleep_secs {
                if !is_poll_running() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if chrono::Utc::now().timestamp() >= target {
                    break;
                }
            }

            do_poll_async(app_handle.clone()).await;

            let now = chrono::Utc::now().timestamp();
            let interval = {
                let state = app_handle.state::<AppState>();
                if let Ok(conn) = state.db.get() {
                    db::settings::get_setting(&conn, KEY_POLL_INTERVAL)
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(30)
                } else {
                    30
                }
            };
            let next = now + (interval as i64) * 60;
            next_poll.store(next, Ordering::Relaxed);
            if let Ok(conn) = app_handle.state::<AppState>().db.get() {
                let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github;

    #[test]
    fn test_poll_lock_acquire_and_release() {
        POLL_LOCK.store(false, Ordering::Release);

        assert!(POLL_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok());
        assert!(POLL_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err());
        POLL_LOCK.store(false, Ordering::Release);
        assert!(POLL_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok());
        POLL_LOCK.store(false, Ordering::Release);
    }

    #[test]
    fn test_poll_guard_drop_releases_lock() {
        POLL_LOCK.store(false, Ordering::Release);

        {
            assert!(POLL_LOCK
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok());
            let _guard = PollGuard;
            assert!(POLL_LOCK.load(Ordering::Relaxed));
        }
        assert!(!POLL_LOCK.load(Ordering::Relaxed));
    }

    #[test]
    fn test_poll_running_flag() {
        POLL_RUNNING.store(true, Ordering::SeqCst);
        assert!(is_poll_running());

        stop_poll();
        assert!(!is_poll_running());

        POLL_RUNNING.store(true, Ordering::SeqCst);
    }

    use crate::db::init::init_memory_db;

    #[test]
    fn test_read_deepseek_config_defaults() {
        let conn = init_memory_db().unwrap();
        let (enabled, model, base_url, api_key, _prompt) = deepseek::read_config(&conn);
        assert!(!enabled);
        assert_eq!(model, "deepseek-v4-flash");
        assert_eq!(base_url, "https://api.deepseek.com");
        assert!(api_key.is_none());
    }

    #[test]
    fn test_read_deepseek_config_configured() {
        crate::crypto::set_test_master_key();
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_MODEL, "deepseek-v4-pro").unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, "https://custom.api").unwrap();
        let encrypted = crate::crypto::encrypt("sk-test");
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_API_KEY, &encrypted).unwrap();

        let (enabled, model, base_url, api_key, _prompt) = deepseek::read_config(&conn);
        assert!(enabled);
        assert_eq!(model, "deepseek-v4-pro");
        assert_eq!(base_url, "https://custom.api");
        assert_eq!(api_key.unwrap(), "sk-test");
    }

    use crate::http;

    #[test]
    fn test_build_http_client_empty() {
        let result = http::build_http_client(http::HttpClientConfig::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_invalid() {
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_url: "://invalid",
            proxy_mode: "custom",
            ..Default::default()
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid proxy URL"));
    }

    #[test]
    fn test_build_http_client_valid() {
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_url: "http://127.0.0.1:1080",
            proxy_mode: "custom",
            ..Default::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_none_ignores_proxy_url() {
        // mode=none 时传无效 URL 也不报错
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_url: "://invalid",
            proxy_mode: "none",
            ..Default::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_system() {
        let result = http::build_http_client(http::HttpClientConfig {
            proxy_mode: "system",
            ..Default::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_system_proxy() {
        let result = deepseek::build_client("sk-test", "", "system");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_success() {
        let result = deepseek::build_client("sk-test", "", "none");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_with_proxy() {
        let result = deepseek::build_client("sk-test", "http://127.0.0.1:1080", "custom");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_deepseek_client_invalid_key() {
        let result = deepseek::build_client("key\nwith\nnewlines", "", "none");
        assert!(result.is_err());
    }

    // Simulates the startup logic in lib.rs
    fn calc_next_poll(conn: &rusqlite::Connection) -> i64 {
        let now = chrono::Utc::now().timestamp();
        db::settings::get_setting(conn, KEY_NEXT_POLL_AT)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|&v| v > now)
            .unwrap_or(now)
    }

    #[test]
    fn test_next_poll_at_startup_restores_future() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        // Saved next_poll_at is 20 min in the future → should be used as-is
        db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now + 20 * 60).to_string()).unwrap();
        let result = calc_next_poll(&conn);
        assert_eq!(result - now, 20 * 60);
    }

    #[test]
    fn test_next_poll_at_startup_falls_back_when_expired() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        // Saved next_poll_at is 10 min in the past → expired, fallback to now → immediate check
        db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now - 10 * 60).to_string()).unwrap();
        let result = calc_next_poll(&conn);
        assert_eq!(result, now);
    }

    #[test]
    fn test_acquire_lock_succeeds_and_fails() {
        // Bug #2 相关：验证 pub(crate) acquire_lock API
        POLL_LOCK.store(false, Ordering::Release);

        // 第一次获取应成功
        let guard1 = acquire_lock();
        assert!(guard1.is_ok(), "first acquire should succeed");
        assert!(POLL_LOCK.load(Ordering::Relaxed));

        // 第二次获取应失败（锁已被持有）
        let guard2 = acquire_lock();
        assert!(guard2.is_err(), "second acquire should fail while locked");

        // 释放第一个 guard
        drop(guard1);
        assert!(!POLL_LOCK.load(Ordering::Relaxed));

        // 再次获取应成功
        let guard3 = acquire_lock();
        assert!(guard3.is_ok(), "acquire should succeed after release");

        // 清理
        drop(guard3);
        POLL_LOCK.store(false, Ordering::Release);
    }

    #[test]
    fn test_next_poll_at_startup_falls_back_when_missing() {
        let conn = init_memory_db().unwrap();
        let now = chrono::Utc::now().timestamp();
        db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "45").unwrap();
        // No saved value → fallback to now → immediate check
        let result = calc_next_poll(&conn);
        assert_eq!(result, now);
    }

    // --- compute_fetch_plan tests ---

    #[test]
    fn test_compute_plan_no_history() {
        // fetch_history=false → third branch (Some(count), 10, false)
        let (max, per_page, paginate) = compute_fetch_plan(false, true, 50);
        assert_eq!(max, Some(50));
        assert_eq!(per_page, 10);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_not_first_query() {
        // !is_first_query → third branch
        let (max, per_page, paginate) = compute_fetch_plan(true, false, 50);
        assert_eq!(max, Some(50));
        assert_eq!(per_page, 10);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_0() {
        // count=0 → first branch: None = no limit, paginate = true
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 0);
        assert_eq!(max, None);
        assert_eq!(per_page, 100);
        assert!(paginate);
    }

    #[test]
    fn test_compute_plan_count_1() {
        // count=1 → second branch: small single-page fetch
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 1);
        assert_eq!(max, Some(1));
        // per_page = clamp(1+5, 10, 100) = 10
        assert_eq!(per_page, 10);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_50() {
        // count=50 → second branch
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 50);
        assert_eq!(max, Some(50));
        // per_page = clamp(50+5, 10, 100) = 55
        assert_eq!(per_page, 55);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_95() {
        // count=95 → second branch, per_page capped at 100
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 95);
        assert_eq!(max, Some(95));
        // per_page = clamp(95+5, 10, 100) = 100
        assert_eq!(per_page, 100);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_100() {
        // count=100 → second branch (since 100 > 100 is false)
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 100);
        assert_eq!(max, Some(100));
        // per_page = clamp(100+5, 10, 100) = 100
        assert_eq!(per_page, 100);
        assert!(!paginate);
    }

    #[test]
    fn test_compute_plan_count_101() {
        // count=101 → first branch, paginated with limit (safety net)
        let (max, per_page, paginate) = compute_fetch_plan(true, true, 101);
        assert_eq!(max, Some(101));
        assert_eq!(per_page, 100);
        assert!(paginate);
    }

    // --- get_github_token tests ---

    #[test]
    fn test_get_github_token_no_key_returns_none() {
        let conn = init_memory_db().unwrap();
        assert!(get_github_token(&conn).is_none(), "未设置 token 时应返回 None");
    }

    #[test]
    fn test_get_github_token_empty_string_returns_none() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();
        db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, "").unwrap();
        assert!(get_github_token(&conn).is_none(), "空字符串 token 时应返回 None");
    }

    #[test]
    fn test_get_github_token_valid_returns_decrypted() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();
        let encrypted = crate::crypto::encrypt("ghp_test_token");
        db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, &encrypted).unwrap();
        let result = get_github_token(&conn);
        assert_eq!(result.as_deref(), Some("ghp_test_token"), "应解密返回原始 token");
    }

    // --- load_poll_settings tests ---

    #[test]
    fn test_load_poll_settings_defaults() {
        let conn = init_memory_db().unwrap();
        let settings = load_poll_settings(&conn).unwrap();
        assert_eq!(settings.proxy_url, "");
        assert_eq!(settings.proxy_mode, "none");
        assert!(settings.github_token.is_none());
        assert!(!settings.fetch_history);
        assert_eq!(settings.fetch_history_count, 1);
    }

    #[test]
    fn test_load_poll_settings_configured() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();

        db::settings::set_setting(&conn, KEY_PROXY_URL, "http://127.0.0.1:1080").unwrap();
        db::settings::set_setting(&conn, KEY_PROXY_MODE, "custom").unwrap();
        let encrypted = crate::crypto::encrypt("ghp_configured");
        db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, &encrypted).unwrap();
        db::settings::set_setting(&conn, KEY_FETCH_HISTORY, "true").unwrap();
        db::settings::set_setting(&conn, KEY_FETCH_HISTORY_COUNT, "5").unwrap();

        let settings = load_poll_settings(&conn).unwrap();
        assert_eq!(settings.proxy_url, "http://127.0.0.1:1080");
        assert_eq!(settings.proxy_mode, "custom");
        assert_eq!(settings.github_token.as_deref(), Some("ghp_configured"));
        assert!(settings.fetch_history);
        assert_eq!(settings.fetch_history_count, 5);
    }

    #[test]
    fn test_load_poll_settings_proxy_mode_fallback() {
        // proxy_url=非空但 proxy_mode 未设置 → fallback 为 "custom"
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, KEY_PROXY_URL, "http://proxy:8080").unwrap();
        let settings = load_poll_settings(&conn).unwrap();
        assert_eq!(settings.proxy_mode, "custom");
    }

    // --- mark_older_as_read tests ---

    #[test]
    fn test_mark_older_as_read_empty_saved_is_noop() {
        let conn = init_memory_db().unwrap();
        mark_older_as_read(&conn, 1, &[]);
        // 不应 panic，不应改变任何状态
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs.is_empty());
    }

    #[test]
    fn test_mark_older_as_read_skips_latest_when_no_newer() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let r1 = db::releases::insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        let r2 = db::releases::insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();

        // saved 按 published_at 降序排列（最新在前）
        let saved = vec![(r2, Some("v2.0".into())), (r1, Some("v1.0".into()))];
        mark_older_as_read(&conn, sid, &saved);

        // 最新（v2.0）应保持 pending，v1.0 应变为 clicked
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let r1_state = releases.iter().find(|r| r.id == r1).unwrap();
        let r2_state = releases.iter().find(|r| r.id == r2).unwrap();
        assert_eq!(r2_state.notification_status, "pending", "最新 release 应保持 pending");
        assert_eq!(r1_state.notification_status, "clicked", "旧 release 应标记为已读");
    }

    #[test]
    fn test_mark_older_as_read_all_when_newer_exists() {
        let conn = init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        // 先在 DB 中创建一个更新的 release
        let newer_id = db::releases::insert_release(&conn, sid, "v3.0", "R3", "https://x", "2024-01-03T00:00:00Z", false, None).unwrap();

        let r1 = db::releases::insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
        let r2 = db::releases::insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();

        // 只传 r1, r2 进来（r3 已存在于 DB）
        let saved = vec![(r2, Some("v2.0".into())), (r1, Some("v1.0".into()))];
        mark_older_as_read(&conn, sid, &saved);

        // r3 已有更新版本在 DB 中，所以 r1, r2 都应标记为 clicked
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let newer = releases.iter().find(|r| r.id == newer_id).unwrap();
        let r1_state = releases.iter().find(|r| r.id == r1).unwrap();
        let r2_state = releases.iter().find(|r| r.id == r2).unwrap();
        assert_eq!(newer.notification_status, "pending", "完全独立的更新 release 不受影响");
        assert_eq!(r1_state.notification_status, "clicked", "有更新版本时所有 saved release 都应标记");
        assert_eq!(r2_state.notification_status, "clicked", "有更新版本时所有 saved release 都应标记");
    }

    // ── source 分发测试（原 save_for_source / fetch_for_source_async 分发）──

    fn gh_release(tag: &str, date: &str, body: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "name": tag,
            "html_url": format!("https://github.com/o/r/releases/tag/{}", tag),
            "published_at": date,
            "prerelease": false,
            "body": body,
        })
    }

    /// github 入库逻辑由 `github::save_releases` 覆盖（trait 实现内部调它）。
    /// 此测试验证 max_count 行为保持。
    #[test]
    fn test_github_save_respects_max_count() {
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            gh_release("v3.0", "2024-03-01T00:00:00Z", Some("v3")),
            gh_release("v2.0", "2024-02-01T00:00:00Z", Some("v2")),
            gh_release("v1.0", "2024-01-01T00:00:00Z", Some("v1")),
        ];

        // max_count=2 应只保存最新两条
        let saved = github::save_releases(&conn, sid, &data, 2);

        assert_eq!(saved.len(), 2, "max_count=2 应只保存 2 条");
        assert_eq!(saved[0].1.as_deref(), Some("v3"));
        assert_eq!(saved[1].1.as_deref(), Some("v2"));
    }

    /// 不支持的 source_type：`get_adapter` 返回 `err.unsupported_source` 错误，
    /// 取代原 `save_for_source`/`fetch_for_source_async` 的 noop/error 分支。
    #[test]
    fn test_get_adapter_unsupported_type_errors() {
        match source::get_adapter("gitlab") {
            Ok(_) => panic!("不支持的源类型应返回错误"),
            Err((status, msg)) => {
                assert_eq!(status, 0);
                assert!(msg.contains("err.unsupported_source"), "错误信息: {}", msg);
            }
        }
    }

    /// github / huggingface 应能取得适配器。
    #[test]
    fn test_get_adapter_supported_types() {
        assert!(source::get_adapter("github").is_ok());
        assert!(source::get_adapter("huggingface").is_ok());
    }

    // ── 编排链路集成测试：fetch→save→mark_read→record ──────
    //
    // 这是 poll_all_sources_async / check_single_source 内联的核心链路。
    // 在重构抽取公共逻辑前，先把这条链路用真实的 github fetch + db 层串起来锁住行为。
    // 重构后这些测试应保持不变地通过，从而验证行为等价性。

    fn make_source(conn: &rusqlite::Connection, owner: &str, repo: &str) -> db::sources::Source {
        let id = db::sources::add_source(conn, "github", owner, repo, "").unwrap();
        db::sources::get_source(conn, id).unwrap().unwrap()
    }

    /// 模拟 poll.rs 编排主链路：
    /// 1. 从远程拉取（这里用构造的 JSON 代替 HTTP，聚焦 save→mark→record 这段）
    /// 2. github::save_releases 入库（trait 实现内部调它）
    /// 3. mark_older_as_read 标记非最新
    /// 4. record_check_success 更新源健康状态
    #[test]
    fn test_pipeline_single_new_release_stays_pending() {
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source = make_source(&conn, "o", "r");

        let fetched = vec![gh_release("v1.0", "2024-01-01T00:00:00Z", Some("body"))];
        let saved = github::save_releases(&conn, source.id, &fetched, 1);
        assert_eq!(saved.len(), 1);
        let (_ids, saved) = post_save_mark_record(&conn, source.id, &saved);
        assert_eq!(saved.len(), 1);

        // 唯一的新版本应保持 pending，可被通知
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].notification_status, "pending");

        // 源健康状态更新为 ok，new_count=1
        let s = db::sources::get_source(&conn, source.id).unwrap().unwrap();
        assert_eq!(s.last_check_status, "ok");
        assert_eq!(s.last_new_count, 1);
        assert_eq!(s.consecutive_failures, 0);
    }

    #[test]
    fn test_pipeline_multiple_new_only_latest_stays_pending() {
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source = make_source(&conn, "o", "r");

        // 首次拉取 3 个新版本（历史模式 max_count=3）
        let fetched = vec![
            gh_release("v3.0", "2024-03-01T00:00:00Z", Some("v3")),
            gh_release("v2.0", "2024-02-01T00:00:00Z", Some("v2")),
            gh_release("v1.0", "2024-01-01T00:00:00Z", Some("v1")),
        ];
        let saved = github::save_releases(&conn, source.id, &fetched, 3);
        assert_eq!(saved.len(), 3);
        let (_ids, saved) = post_save_mark_record(&conn, source.id, &saved);
        assert_eq!(saved.len(), 3);

        // 只有最新 v3.0 保持 pending，v2/v1 被标记为 clicked
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let status_of = |tag: &str| -> String {
            releases.iter().find(|r| r.tag_name == tag).unwrap().notification_status.clone()
        };
        assert_eq!(status_of("v3.0"), "pending");
        assert_eq!(status_of("v2.0"), "clicked");
        assert_eq!(status_of("v1.0"), "clicked");
    }

    #[test]
    fn test_pipeline_subsequent_poll_with_no_new_release() {
        let conn = init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source = make_source(&conn, "o", "r");

        // 第一轮：拉到 v1.0
        let fetched1 = vec![gh_release("v1.0", "2024-01-01T00:00:00Z", Some("v1"))];
        let saved = github::save_releases(&conn, source.id, &fetched1, 1);
        let (_ids, _saved) = post_save_mark_record(&conn, source.id, &saved);

        // 第二轮：同样的数据，save 返回空（已入库），但源健康状态仍应更新
        let saved2 = github::save_releases(&conn, source.id, &fetched1, 1);
        assert!(saved2.is_empty(), "重复数据不应再次保存");
        let (_ids2, _saved2) = post_save_mark_record(&conn, source.id, &saved2);

        let s = db::sources::get_source(&conn, source.id).unwrap().unwrap();
        assert_eq!(s.last_check_status, "ok");
        assert_eq!(s.last_new_count, 0, "无新版本时 last_new_count 应为 0");

        // release 数量不变，状态不变
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].notification_status, "pending");
    }

    #[test]
    fn test_pipeline_failure_records_error_without_saving() {
        let conn = init_memory_db().unwrap();
        let source = make_source(&conn, "o", "r");

        // 模拟 check 失败：不 save，直接 record_check_failure（与 poll.rs 失败分支一致）
        let _ = db::sources::record_check_failure(&conn, source.id, "err.api_error|503|Service Unavailable");

        let s = db::sources::get_source(&conn, source.id).unwrap().unwrap();
        assert_eq!(s.last_check_status, "error");
        assert_eq!(s.consecutive_failures, 1);
        assert_eq!(s.last_new_count, 0);
        assert!(s.last_check_message.as_deref().unwrap().contains("503"));

        // 无 release 被保存
        assert!(db::releases::get_releases_with_state(&conn).unwrap().is_empty());
    }

    // ── collect_pending_and_notify 编排链路注入式测试（Phase 3 / S3）──
    //
    // 直接测真实的私有 async fn collect_pending_and_notify，注入 NoopEmitter
    // + init_memory_pool，覆盖「通知派发次数 / muted 源跳过 / new_ids 过滤」状态机。
    // 这条链路原先无法被 CI 触达（S3），至此消除“测试副本”假象。

    /// 单条 pending release：通知派发 1 次，new_releases 含该条。
    #[tokio::test]
    async fn test_collect_pending_notifies_single_release() {
        let pool = crate::db::init::init_memory_pool().unwrap();
        let saved = {
            let conn = pool.get().unwrap();
            db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
            let source = make_source(&conn, "o", "r");
            github::save_releases(&conn, source.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("b"))], 1)
        };
        assert_eq!(saved.len(), 1);
        let new_ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();

        let emitter = crate::types::NoopEmitter::new();
        let (all_pending, new_releases) =
            collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;

        assert_eq!(emitter.call_count(), 1, "单条 pending 应派发 1 次通知");
        assert_eq!(new_releases.len(), 1, "new_ids 全包含 → new_releases 含该条");
        // 标记 last_notified_at 后该 release 不再未读pending（only_notified_missing=true），
        // 故 all_pending（末尾重读）应为空——这是 collect 真实行为。
        assert!(all_pending.is_empty(), "已通知的 release 应从 unread pending 列表移除");
    }

    /// 静音源：其 pending release 不派发通知，但仍计入 new_releases 过滤结果。
    #[tokio::test]
    async fn test_collect_pending_skips_muted_source() {
        let pool = crate::db::init::init_memory_pool().unwrap();
        let (id_a, id_b) = {
            let conn = pool.get().unwrap();
            db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
            let source_a = make_source(&conn, "oa", "ra");
            let source_b = make_source(&conn, "ob", "rb");
            let a = github::save_releases(&conn, source_a.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("a"))], 1);
            let b = github::save_releases(&conn, source_b.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("b"))], 1);
            assert_eq!(a.len(), 1);
            assert_eq!(b.len(), 1);
            db::sources::set_source_muted(&conn, source_a.id, true).unwrap();
            (a[0].0, b[0].0)
        };
        let new_ids = vec![id_a, id_b];

        let emitter = crate::types::NoopEmitter::new();
        let (_all_pending, new_releases) =
            collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;

        assert_eq!(emitter.call_count(), 1, "静音源不派发通知，仅 source_b 通知");
        assert_eq!(new_releases.len(), 2, "muted 不剔除 new_releases（仅跳过通知）");
    }

    /// 通知派发阶段对全部 pending 都触发；new_releases 仅按 new_ids 子集过滤。
    #[tokio::test]
    async fn test_collect_pending_new_ids_subset_filters_result() {
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id_v2 = {
            let conn = pool.get().unwrap();
            db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
            let source = make_source(&conn, "o", "r");
            // 直接 save_releases（不走 post_save_mark_record），两条都保持 pending
            let saved = github::save_releases(
                &conn,
                source.id,
                &[
                    gh_release("v2", "2024-03-01T00:00:00Z", Some("v2")),
                    gh_release("v1", "2024-02-01T00:00:00Z", Some("v1")),
                ],
                2,
            );
            assert_eq!(saved.len(), 2);
            saved[0].0
        };
        // new_ids 只含 v2 → new_releases 只返回 v2，但通知阶段对两条 pending 都派发
        let new_ids = vec![id_v2];

        let emitter = crate::types::NoopEmitter::new();
        let (_all_pending, new_releases) =
            collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;

        assert_eq!(emitter.call_count(), 2, "通知派发对全部 pending 触发，与 new_ids 无关");
        assert_eq!(new_releases.len(), 1, "new_releases 按 new_ids 子集过滤后仅含 v2");
        assert_eq!(new_releases[0].tag_name, "v2");
    }

    // ── filter_ai_eligible：跳过 ai_eligible=false 源（youtube/bilibili）的 AI 摘要/翻译 ──

    #[tokio::test]
    async fn test_filter_ai_eligible_keeps_only_eligible_types() {
        let pool = crate::db::init::init_memory_pool().unwrap();
        let (gh_id, yt_id, bl_id) = {
            let conn = pool.get().unwrap();
            let gh = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
            let yt = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
            let bl = db::sources::add_source(&conn, "bilibili", "476599099", "", "").unwrap();
            let gh_id = db::releases::insert_release(&conn, gh, "v1", "R", "https://x", "2024-01-01T00:00:00Z", false, Some("gh")).unwrap();
            let yt_id = db::releases::insert_release(&conn, yt, "vid1", "V", "https://y", "2024-01-02T00:00:00Z", false, Some("yt")).unwrap();
            let bl_id = db::releases::insert_release(&conn, bl, "BV1xx", "V", "https://b", "2024-01-03T00:00:00Z", false, Some("bili")).unwrap();
            (gh_id, yt_id, bl_id)
        };

        let saved = vec![
            (gh_id, Some("gh".into())),
            (yt_id, Some("yt".into())),
            (bl_id, Some("bili".into())),
        ];
        let filtered = filter_ai_eligible(&pool, &saved).await.unwrap();
        assert_eq!(filtered.len(), 1, "youtube/bilibili 源条目应被过滤掉");
        assert_eq!(filtered[0].0, gh_id);
    }

    #[tokio::test]
    async fn test_filter_ai_eligible_all_youtube_empty() {
        let pool = crate::db::init::init_memory_pool().unwrap();
        let yt_id = {
            let conn = pool.get().unwrap();
            let yt = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
            db::releases::insert_release(&conn, yt, "vid1", "V", "https://y", "2024-01-01T00:00:00Z", false, Some("yt")).unwrap()
        };
        let filtered = filter_ai_eligible(&pool, &[(yt_id, Some("yt".into()))])
            .await
            .unwrap();
        assert!(filtered.is_empty(), "纯 youtube 条目应全部过滤");
    }

    #[tokio::test]
    async fn test_filter_ai_eligible_empty_input() {
        let pool = crate::db::init::init_memory_pool().unwrap();
        assert!(filter_ai_eligible(&pool, &[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_filter_ai_eligible_db_error_fails_closed() {
        // 占住唯一连接使后续 get() 超时失败 → 必须返回 Err（fail-closed），
        // 绝不能降级为空集把 youtube 条目放行给 DeepSeek。
        use r2d2_sqlite::SqliteConnectionManager;
        let manager = SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_millis(5))
            .build(manager)
            .unwrap();
        let _held = pool.get().unwrap(); // 占用唯一连接
        let err = filter_ai_eligible(&pool, &[(1, Some("x".into()))])
            .await
            .expect_err("DB 连接失败应返回 Err（fail-closed）");
        assert!(
            err.contains("err.db_lock"),
            "错误应含 err.db_lock，实际: {}",
            err
        );
    }
}
