use std::sync::Arc;
use serde_json::json;
use tauri::Manager;
use tauri_specta::Event;

use crate::db;
use crate::deepseek;
use crate::error::SourceError;
use crate::http;
use crate::source;
use crate::types::{AppState, PollResult};
use super::ai_jobs::{filter_ai_eligible, schedule_translations_background};
use super::notify_flow::collect_pending_and_notify;
use super::settings::{compute_fetch_plan, load_poll_settings, PollSettings};
use super::{acquire_lock, MAX_CONCURRENCY, SOURCE_FETCH_TIMEOUT_SECS};

/// 将新保存的 releases 中非最新版本标记为"已读"。
///
/// - 如果库中已有比最新 release 更新的版本 → 全部标记已读
/// - 否则跳过最新一条，其余标记已读
pub(super) fn mark_older_as_read(conn: &rusqlite::Connection, source_id: i64, saved: &[(i64, Option<String>)]) {
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
/// 抽成独立函数，便于测试直接跑真实代码路径（两条调用路径共用）。
pub(super) fn post_save_mark_record(
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


/// 单源检查的结果：本轮新增 release id 与 save 产物（供后续 AI 摘要/翻译）。
pub(super) struct CheckOutcome {
    pub(super) new_ids: Vec<i64>,
    pub(super) saved: Vec<(i64, Option<String>)>,
}

/// 单源检查上下文：依赖与配置的集合，两条调用路径（手动/定时）各自构造一次。
pub(super) struct CheckCtx<'a> {
    pub(super) db_pool: &'a r2d2::Pool<crate::db::SqliteConnectionManager>,
    pub(super) adapter: &'a dyn source::SourceAdapter,
    pub(super) client: &'a reqwest::Client,
    pub(super) token: Option<&'a str>,
    pub(super) fetch_history: bool,
    pub(super) fetch_history_count: usize,
    /// 成功日志键（"check.manual" / "check.auto"）。
    pub(super) log_key: &'static str,
}

/// 单源检查核心链路：fetch（超时保护）→ save → post_save + 日志。
/// 手动检查（check_single_source）与定时轮询（poll_all_sources_async）两条路径
/// 共用此实现，差异仅 CheckCtx.log_key；
/// 失败统一记为 check.failed（临时性错误 WARN / 其余 ERROR）并返回 Err。
pub(super) async fn check_one_source(ctx: &CheckCtx<'_>, source: &db::sources::Source) -> Result<CheckOutcome, String> {
    let is_first_query = source.last_checked_at.is_none()
        || source.last_checked_at.as_deref() == Some("");
    // 日志展示标识：YouTube 源用频道名替代 channel_id（repo 置空），避免 `owner/` 残废格式
    let (log_owner, log_repo) = source::source_log_ident(
        &source.source_type,
        &source.owner,
        &source.repo,
        source.description.as_deref(),
    );
    // YouTube 源开启历史拉取时，每次检查都按 fetch_history_count 拉取历史：
    // save 阶段按 UNIQUE(source_id, tag_name) 去重跳过已存在条目，因此
    // 重新配置 Data API Key 后无需删除源即可补拉历史视频。RSS 模式单页拉取，不受影响。
    // 拉取历史/预发布这类开关支持**按源覆盖**（`sources.config` JSON）：未配置则沿用全局。
    let fetch_history = db::sources::config_flag(source.config.as_deref(), "fetch_history")
        .unwrap_or(ctx.fetch_history);
    let history_query = fetch_history && (is_first_query || ctx.adapter.always_fetch_history());
    let (max_count, per_page, needs_pagination) = compute_fetch_plan(fetch_history, history_query, ctx.fetch_history_count);

    let fetch_result = tokio::time::timeout(
        std::time::Duration::from_secs(SOURCE_FETCH_TIMEOUT_SECS),
        async {
            if needs_pagination {
                ctx.adapter.fetch_all(ctx.client, source, max_count, ctx.token).await
            } else {
                ctx.adapter.fetch(ctx.client, source, per_page, ctx.token).await
            }
        },
    )
    .await
    .unwrap_or_else(|_| Err(SourceError::source_timeout(SOURCE_FETCH_TIMEOUT_SECS)));

    match fetch_result {
        Ok(releases) => {
            let saved = match tokio::time::timeout(
                std::time::Duration::from_secs(SOURCE_FETCH_TIMEOUT_SECS),
                ctx.adapter.save(ctx.db_pool, source, &releases, max_count.unwrap_or(usize::MAX), ctx.client),
            )
            .await
            {
                Ok(saved) => saved,
                Err(_) => {
                    let err = SourceError::save_timeout(SOURCE_FETCH_TIMEOUT_SECS);
                    return Err(record_check_failure(ctx.db_pool, source.id, log_owner, log_repo, &err).await);
                }
            };
            // 抓到的每一条都是新内容、且吃满单页 → feed 窗口被上游截断：比它更旧的新版本没随
            // 本次响应返回，本轮结果仍不完整（如间隔内发布数超过单页容量）。fetch/save 本身
            // 确实成功，故源健康保持 ok，但补一条 WARN 让缺口在日志里可见。
            // 首次检查必然全为新内容，不在此列（那不是截断，是建库）。
            let window_truncated = !is_first_query
                && !history_query
                && per_page > 0
                && releases.len() >= per_page
                && saved.len() == releases.len();
            // save 之后的同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
            let db_pool_blk = ctx.db_pool.clone();
            let source_id = source.id;
            let owner = log_owner;
            let repo = log_repo;
            let log_key = ctx.log_key;
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
                    log_key,
                    &json!({"owner": &owner, "repo": &repo, "count": new_count}).to_string(),
                );
                if window_truncated {
                    db::logs::write_log_key(
                        &conn,
                        "WARN",
                        "check.partial",
                        &json!({"owner": &owner, "repo": &repo, "count": new_count}).to_string(),
                    );
                }
                (ids, saved)
            })
            .await;
            match result {
                Ok((new_ids, saved)) => Ok(CheckOutcome { new_ids, saved }),
                Err(e) => {
                    log::error!("{} post-save spawn_blocking panic: {}", log_key, e);
                    Ok(CheckOutcome { new_ids: vec![], saved: vec![] })
                }
            }
        }
        Err(err) => {
            let msg = record_check_failure(ctx.db_pool, source.id, log_owner, log_repo, &err).await;
            Err(msg)
        }
    }
}

/// task panic / 被取消时的失败留痕：源健康状态与日志页各记一条。
///
/// 与 [`record_check_failure`] 同口径双写，但错误文本只能是固定的
/// `err.check_task_panic`——`JoinError` 只说明任务异常终止，源内的错误详情在 panic
/// 中已经丢了。只写 `sources` 表的话，用户会在源列表看到红字却在日志页查不到任何
/// 记录，而其他所有失败路径都是双写。
///
/// 同步 SQLite 调用收笼进 `spawn_blocking`（与 `apply_mass_failure_guard` 及
/// huggingface.rs 的同类说明一致）：这条路径虽罕见，但它跑在整轮并发检查的收尾处，
/// 直接阻塞的正是 tokio worker。
pub(super) async fn record_task_panic(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    source_id: i64,
) {
    let pool = db_pool.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(conn) = pool.get() else { return };
        let _ = db::sources::record_check_failure(&conn, source_id, "err.check_task_panic");
        db::logs::write_log_key(
            &conn,
            "ERROR",
            "err.check_task_panic",
            &json!({"id": source_id}).to_string(),
        );
    })
    .await;
}

/// 记录一次检查失败（源健康状态 + 日志），返回**脱敏后**的错误文本供调用方返回前端。
///
/// 凭据脱敏放在这里而非各出口：适配器错误文本会回显完整 URL（如 `…&key=AIzaSy…`），
/// 而该文本有**三个去向**——① logs（`write_log_key` 也默认脱敏，此处是二道保险）；
/// ② `sources.last_check_message`；③ 返回给前端进 toast（用户截图报障即外泄）。
///
/// `status=0` 表示无 HTTP 状态（超时/发送失败），与认证、限流、5xx 同样按临时性记 WARN。
pub(super) async fn record_check_failure(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    source_id: i64,
    owner: String,
    repo: String,
    err: &SourceError,
) -> String {
    let msg = crate::redact::redact(&err.to_ipc_string());
    let level = db::logs::failure_log_level(err.status());
    let uncounted = err.is_uncounted_failure();
    let log_key_failed = if uncounted { "check.failed_uncounted" } else { "check.failed" };
    let pool = db_pool.clone();
    let msg_for_log = msg.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(conn) = pool.get() {
            let _ = if uncounted {
                db::sources::record_check_failure_uncounted(&conn, source_id, &msg_for_log)
            } else {
                db::sources::record_check_failure(&conn, source_id, &msg_for_log)
            };
            db::logs::write_log_key(
                &conn,
                level,
                log_key_failed,
                &json!({"owner": &owner, "repo": &repo, "error": &msg_for_log}).to_string(),
            );
        }
    })
    .await;
    msg
}

pub async fn check_single_source(app: tauri::AppHandle, id: i64) -> Result<PollResult, String> {
    let _guard = acquire_lock()?;

    let (source_obj, settings);
    {
        let state = app.state::<AppState>();
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        let sources = db::sources::list_sources(&conn)?;
        let source = sources
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("err.source_not_found")?;
        settings = load_poll_settings(&conn)?;
        source_obj = source;
    }

    // 日志展示标识：YouTube 源用频道名替代 channel_id（repo 置空），避免 `owner/` 残废格式
    let (log_owner, log_repo) = source::source_log_ident(
        &source_obj.source_type,
        &source_obj.owner,
        &source_obj.repo,
        source_obj.description.as_deref(),
    );
    // token 按适配器声明的 auth_kind 选取（YouTube → Data API Key，GitHub → PAT，
    // HF → None），无需逐个 source_type 特判。
    let adapter = match source::get_adapter(&source_obj.source_type) {
        Ok(a) => a,
        Err(e) => return Err(e.to_ipc_string()),
    };
    let token = settings.credentials.token_for(adapter.as_ref());

    // 从 AppHandle 提取依赖，注入提纯后的核心函数
    let (db_pool, deepseek_semaphore) = {
        let state = app.state::<AppState>();
        (state.db.clone(), state.deepseek_semaphore.clone())
    };
    let emitter: std::sync::Arc<dyn crate::types::Emitter> =
        std::sync::Arc::new(app.clone());

    // client 不携带 default Authorization——github token 由 adapter 按请求设置，
    // 避免 HF 请求泄露 GitHub Token（见 http::HttpClientConfig::set_default_auth）
    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url: &settings.proxy.url,
        proxy_mode: &settings.proxy.mode,
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

    // 单源检查核心链路（fetch→save→post_save→日志）与轮询路径共用 check_one_source，
    // 超时/错误分级/去重语义两处保持一致，修单源超时只改一处。
    let ctx = CheckCtx {
        db_pool: &db_pool,
        adapter: adapter.as_ref(),
        client: &client,
        token,
        fetch_history: settings.fetch_history,
        fetch_history_count: settings.fetch_history_count,
        log_key: "check.manual",
    };
    let (new_ids, saved) = match check_one_source(&ctx, &source_obj).await {
        Ok(outcome) => (outcome.new_ids, outcome.saved),
        Err(msg) => return Err(msg),
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
    // 手动单源检查与轮询语义一致：摘要同步（通知带重要度），译文后台批
    // （fire-and-forget）——不阻塞手动检查返回，译文完成后经
    // release-state-changed 刷新可见。
    match filter_ai_eligible(&db_pool, &saved).await {
        Ok(ai_saved) if !ai_saved.is_empty() => {
            deepseek::generate_summaries_for_new(&db_pool, &deepseek_semaphore, &ai_saved).await;
        }
        Ok(_) => {}
        Err(e) => log::error!("err.filter_ai_eligible|{}", e),
    }
    schedule_translations_background(&db_pool, &deepseek_semaphore, &emitter);

    let (_, new_releases) = collect_pending_and_notify(&db_pool, &*emitter, &new_ids, false).await;

    let _ = crate::events::PollCompleted.emit(&app);

    Ok(PollResult {
        new_releases: new_releases
            .into_iter()
            .filter(|r| r.notification_status == "pending")
            .collect(),
    })
}


pub(super) async fn poll_all_sources_async(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    sources: &[db::sources::Source],
    settings: &PollSettings,
) -> (Vec<i64>, Vec<(i64, Option<String>)>) {
    // client 不携带 default Authorization——github token 由 adapter 按请求设置，
    // 避免 HF 请求泄露 GitHub Token（见 http::HttpClientConfig::set_default_auth）
    let client = match http::build_http_client(http::HttpClientConfig {
        proxy_url: &settings.proxy.url,
        proxy_mode: &settings.proxy.mode,
        bearer_token: None,
        ..Default::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            if let Ok(conn) = db_pool.get() {
                for source in sources {
                    // 环境层面（代理/证书）问题：所有源一起挂，不是源本身坏了 → 不计入断路器
                    let _ = db::sources::record_check_failure_uncounted(&conn, source.id, &e);
                }
                db::logs::write_log_key(&conn, "ERROR", "check.http_client_error", &json!({"error": &e}).to_string());
            }
            return (vec![], vec![]);
        }
    };

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENCY));
    // 带上 source_id：task panic 时 JoinError 里没有源信息，而每个源都必须有人认领。
    let mut handles: Vec<(i64, tokio::task::JoinHandle<TaskOutcome>)> = Vec::new();
    let pool = db_pool.clone();
    let fetch_history = settings.fetch_history;
    let fetch_history_count = settings.fetch_history_count;

    for source in sources {
        let sem = semaphore.clone();
        let client = client.clone();
        let source_id = source.id;
        let source = source.clone();
        let db_pool = pool.clone();
        // token 按适配器声明的 auth_kind 选取（YouTube → Data API Key，
        // GitHub → PAT，HF → None）。spawn 需要 'static，先克隆为 owned。
        let adapter = match source::get_adapter(&source.source_type) {
            Ok(a) => a,
            Err(e) => {
                log::error!("{}", e);
                continue;
            }
        };
        let token = settings.credentials.token_for(adapter.as_ref()).map(str::to_string);

        handles.push((source_id, tokio::spawn(async move {
            // 背压信号量用 graceful 等待获取：失败若直接 panic，会被 handle.await 静默吞掉
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("err.sem_closed|{}", e);
                    // 未真正发起检查：不计入 attempted（不参与整轮失败比例判定）
                    return TaskOutcome { source_id, failed: false, checked: false, new_ids: vec![], saved: vec![] };
                }
            };
            // 单源检查核心链路（fetch→save→post_save→日志）与手动检查共用
            // check_one_source；失败已记 check.failed，此处仅标记本轮失败。
            let ctx = CheckCtx {
                db_pool: &db_pool,
                adapter: adapter.as_ref(),
                client: &client,
                token: token.as_deref(),
                fetch_history,
                fetch_history_count,
                log_key: "check.auto",
            };
            match check_one_source(&ctx, &source).await {
                Ok(outcome) => TaskOutcome {
                    source_id,
                    failed: false,
                    checked: true,
                    new_ids: outcome.new_ids,
                    saved: outcome.saved,
                },
                Err(_) => TaskOutcome {
                    source_id,
                    failed: true,
                    checked: true,
                    new_ids: vec![],
                    saved: vec![],
                },
            }
        })));
    }

    let mut all_new_ids = Vec::new();
    let mut all_saved = Vec::new();
    let mut outcomes = Vec::new();
    for (source_id, handle) in handles {
        match handle.await {
            Ok(outcome) => {
                all_new_ids.extend(outcome.new_ids.iter().copied());
                all_saved.extend(outcome.saved.iter().cloned());
                outcomes.push(outcome);
            }
            // task panic / 被取消时只剩 JoinError。丢弃它等于该源本轮既不记成功也不记
            // 失败：last_check_status 停在上一轮的 ok，UI 健康灯常绿但内容已停更。
            Err(e) => {
                log::error!("err.check_task_panic|{}|{}", source_id, e);
                record_task_panic(db_pool, source_id).await;
                outcomes.push(TaskOutcome {
                    source_id,
                    failed: true,
                    checked: true,
                    new_ids: vec![],
                    saved: vec![],
                });
            }
        }
    }

    // 整轮失败判定必须在整轮结果齐了之后才能做
    apply_mass_failure_guard(db_pool, &outcomes).await;

    (all_new_ids, all_saved)
}

/// 本轮单个源的结局（用于整轮失败比例判定）。
pub(super) struct TaskOutcome {
    pub(super) source_id: i64,
    pub(super) failed: bool,
    /// 是否真的发起了检查（信号量关闭等情况下为 false，不参与比例判定）。
    pub(super) checked: bool,
    pub(super) new_ids: Vec<i64>,
    pub(super) saved: Vec<(i64, Option<String>)>,
}

/// 同时失败源数占已检查源数的最小百分比：达到即判定为网络层抖动。
pub(super) const MASS_FAILURE_PERCENT: usize = 50;
/// 触发整轮失败判定的最小源数：单源用户的失败就是单源问题，不该被本规则豁免。
pub(super) const MASS_FAILURE_MIN_SOURCES: usize = 2;

/// 本轮 ≥[`MASS_FAILURE_PERCENT`]% 的源同时失败时，判定为网络/代理层抖动，
/// 回退本轮给每个失败源累加的 `consecutive_failures`。
///
/// 依据：整轮同时失败通常是链路抖一下全挂（实测 10 个源在同一秒报同一个
/// `err.source_timeout|300`），不是这些源同时坏；照旧累加的话，连续 3 轮抖动就
/// 会把所有源逐个自动禁掉，而用户看到的现象只是「这些源怎么不更新了」，需手动
/// 逐个重开。
///
/// 实现上「先累加、整轮汇总后回退」而非「检查时就决定不累加」：失败记录发生在单源
/// 检查内部，早于整轮汇总，而每轮每源最多累加 1，回退 1 就等价于本轮不计入。
pub(super) async fn apply_mass_failure_guard(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    outcomes: &[TaskOutcome],
) {
    let attempted = outcomes.iter().filter(|o| o.checked).count();
    let failed_ids: Vec<i64> = outcomes
        .iter()
        .filter(|o| o.checked && o.failed)
        .map(|o| o.source_id)
        .collect();
    if attempted < MASS_FAILURE_MIN_SOURCES || failed_ids.len() * 100 < attempted * MASS_FAILURE_PERCENT {
        return;
    }

    let failed = failed_ids.len();
    let pool = db_pool.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(conn) = pool.get() else { return };
        for id in &failed_ids {
            let _ = db::sources::decrement_consecutive_failures(&conn, *id);
        }
        db::logs::write_log_key(
            &conn,
            "WARN",
            "check.round_failed",
            &json!({"failed": failed, "attempted": attempted}).to_string(),
        );
    })
    .await;
}

