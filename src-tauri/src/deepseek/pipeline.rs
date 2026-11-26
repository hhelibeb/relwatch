use rusqlite::Connection;

use crate::db;
use crate::db::ai_usage::CallUsage;
use super::calls::{call_detect_language, call_summary, call_translate, call_translate_streaming};
use super::config::{load_ai_network_config, read_config, read_translate_config};
use super::transport::{
    build_client, build_stream_client, DEEPSEEK_TIMEOUT_SECS_SUMMARY,
    DEEPSEEK_TIMEOUT_SECS_TRANSLATE, DEEPSEEK_TRANSLATE_TRUNCATE_CHARS,
};

fn persist_call_usages(conn: &Connection, release_id: i64, model: &str, usages: &[CallUsage]) {
    if usages.is_empty() {
        return;
    }
    if let Err(e) = db::ai_usage::insert_call_usage(conn, Some(release_id), model, usages) {
        log::warn!("记录 AI token 用量失败 id={}: {}", release_id, e);
    }
}

/// 复查：该 release 是否已有 AI 摘要（`run_ai_job` 的 `already_done`，摘要侧）。
pub(super) fn has_ai_summary(conn: &Connection, release_id: i64) -> bool {
    db::releases::get_release(conn, release_id)
        .ok()
        .flatten()
        .map(|r| r.ai_summary.is_some())
        .unwrap_or(false)
}

/// 复查：该 release 是否已有译文（`run_ai_job` 的 `already_done`，翻译侧）。
pub(super) fn has_translation(conn: &Connection, release_id: i64) -> bool {
    db::releases::get_release(conn, release_id)
        .ok()
        .flatten()
        .map(|r| r.body_translated.is_some())
        .unwrap_or(false)
}


/// 写 AI 任务结果日志：统一"成功/失败 × 有/无 release 行"四种组合。
/// 摘要与翻译共用。
/// - `ok=true`：`{action}已生成: {owner}/{repo} {tag}[ {detail}]`
/// - `ok=false`：`{action}生成失败: {owner}/{repo} {tag}: {detail}`
///
/// 有 release 行时用 owner/repo/tag 定位，否则退化为 `id={release_id}`。
pub(super) fn log_ai_job_result(
    conn: &Connection,
    level: &str,
    action: &str,
    release_id: i64,
    detail: &str,
    ok: bool,
) {
    let rel = db::releases::get_release(conn, release_id).ok().flatten();
    let who = match rel {
        Some(r) => format!("{}/{} {}", r.owner, r.repo, r.tag_name),
        None => format!("id={}", release_id),
    };
    let msg = if ok {
        if detail.is_empty() {
            format!("{}已生成: {}", action, who)
        } else {
            format!("{}已生成: {} {}", action, who, detail)
        }
    } else {
        format!("{}生成失败: {}: {}", action, who, detail)
    };
    db::logs::write_log(conn, level, &msg);
}

/// 翻译任务的结果：语言检测一致时短路（返回原文）或正常译文。
/// 短路与正常译文在 on_ok 中区分处理（短路写原文 + log::info，译文写译文 + DB 日志）。
pub(super) enum TranslateOutcome {
    Skipped(String),
    Translated(String),
}

/// 批量注入的流式分片出口：分片带上 `release_id`。
///
/// 批内可能多条同时在出字，只有前端自己知道当前展示的是哪条，故由事件携带 id
/// 让前端过滤，而不是在服务端维护「谁是当前观察者」这种易错状态。
pub type ChunkSink = std::sync::Arc<dyn Fn(i64, &str) + Send + Sync>;

/// 单条任务的流式分片出口（已绑定 release_id）。
pub type DeltaSink = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// AI 任务公共流水线：读配置 → 建 client → 并发调度 → 结果/失败分别落库。
///
/// 摘要与翻译两条流水线共用此骨架，差异点以参数注入：
/// - `job`：控制台日志文案（"摘要"/"译文"）
/// - `truncate_chars`：正文截断上限（摘要 4000 / 翻译见
///   `DEEPSEEK_TRANSLATE_TRUNCATE_CHARS`，须与 `DEEPSEEK_TRANSLATE_MAX_TOKENS` 同量级）
/// - `timeout_secs`：本批请求的总超时（摘要 120 / 翻译 300，见 DEEPSEEK_TIMEOUT_SECS_*）。
///   仅在非流式路径生效：传入 `on_chunk` 时改用流式客户端（空闲读超时），见
///   `build_stream_client`。
/// - `on_chunk`：流式分片出口（None = 非流式）。只有用户点「翻译」的那次单条
///   翻译会传 Some：后台自动批没有观察者，逐片发事件纯属白耗 IPC 与主线程。
/// - `extra_ready`：额外前置开关（翻译的 translate_enabled/force；摘要恒 true）
/// - `call`：AI 调用（翻译侧在闭包内做语言检测短路）
/// - `on_ok` / `on_err`：成功/失败各自的落库与日志动作（在 spawn_blocking 内执行）
///
/// 参数较多：均为两条流水线的真实差异点，刻意集中注入而非隐式复制。
///
/// - `already_done`：调用前复查「这条是不是已经有结果了」，返回 true 则跳过。
///   待办名单是批启动时一次性读取的快照，批内任务并发跑（翻译批可长达 300s），
///   期间同一条可能已被另一路径写入（手动单条翻译、上一批收尾、用户删除重建
///   后重新检查）。不复查会对已完成的条目重复发请求，白白烧 token；更糟的是
///   重复请求若返回空内容，会走 on_err 把本已成功的 retry 计数加 1。
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_ai_job<T, F, Fut>(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    job: &'static str,
    truncate_chars: usize,
    timeout_secs: u64,
    on_chunk: Option<ChunkSink>,
    extra_ready: impl Fn(&Connection) -> bool,
    already_done: impl Fn(&Connection, i64) -> bool + Send + Sync + 'static,
    call: F,
    on_ok: impl Fn(&Connection, i64, T) + Send + Sync + 'static,
    on_err: impl Fn(&Connection, i64, &str) + Send + Sync + 'static,
) where
    F: Fn(reqwest::Client, String, String, String, String, Option<DeltaSink>) -> Fut
        + Send
        + Sync
        + 'static,
    // call 的返回值携带本次任务的用量明细（可能含语言检测 + 翻译多次调用），
    // 统一由本流水线落 `ai_usage` 表，call_* 与 on_ok 均无需感知统计。
    // 失败分支同样带回用量：重试耗尽前的已消耗调用也要记录。
    Fut: std::future::Future<
            Output = Result<(T, Vec<CallUsage>), (String, Vec<CallUsage>)>,
        > + Send
        + 'static,
    T: Send + 'static,
{
    let (cfg, proxy_url, proxy_mode) = {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("数据库连接失败: {}", e);
                return;
            }
        };
        if !extra_ready(&conn) {
            return;
        }
        let cfg = read_config(&conn);
        let (proxy_url, proxy_mode) = load_ai_network_config(&conn);
        (cfg, proxy_url, proxy_mode)
    };
    if !cfg.enabled {
        return;
    }
    let api_key = match cfg.api_key {
        Some(k) => k,
        None => return,
    };
    let client = match if on_chunk.is_some() {
        // 有分片消费者 = 本次走流式请求：客户端也得换成流式口径
        // （不设总超时，改空闲读超时，见 build_stream_client）
        build_stream_client(&api_key, &proxy_url, &proxy_mode)
    } else {
        build_client(&api_key, &proxy_url, &proxy_mode, timeout_secs)
    } {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("创建 DeepSeek 客户端失败: {}", e);
            log::error!("{}", msg);
            // 同时写 DB：log::error! 只落到日志文件，应用内的日志页看不到；只打 log
            // 会让「批静默不发请求」（待办非空、开关全开、retry 计数 = 0 却无任何
            // 可见错误）无法在应用内定位。
            // 写入后由调用方（后台批收尾）emit LogAppended 刷新日志 tab。
            if let Ok(conn) = db_pool.get() {
                db::logs::write_log(&conn, "ERROR", &msg);
            }
            return;
        }
    };

    let call = std::sync::Arc::new(call);
    let on_ok = std::sync::Arc::new(on_ok);
    let on_err = std::sync::Arc::new(on_err);
    let already_done = std::sync::Arc::new(already_done);
    let semaphore = deepseek_semaphore.clone();
    let model = cfg.model;
    let base_url = cfg.base_url;
    let prompt = cfg.prompt;
    let mut handles = Vec::new();
    for (release_id, body) in saved {
        let body_text = match body {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        let truncated: String = body_text.chars().take(truncate_chars).collect();
        let client = client.clone();
        let model = model.clone();
        let base_url = base_url.clone();
        let prompt = prompt.clone();
        let db = db_pool.clone();
        let release_id = *release_id;
        let sem_clone = semaphore.clone();
        let call = call.clone();
        let on_ok = on_ok.clone();
        let on_err = on_err.clone();
        let already_done = already_done.clone();
        // 分片出口绑定本条 release：批内可能多条并行出字，前端按 id 只接自己那条
        let on_chunk: Option<DeltaSink> = on_chunk.clone().map(|sink| {
            let bound: DeltaSink = std::sync::Arc::new(move |delta: &str| sink(release_id, delta));
            bound
        });

        handles.push(tokio::spawn(async move {
            let _permit = match sem_clone.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("信号量获取失败: {}", e);
                    return;
                }
            };
            // 抢到信号量后才复查：此时才是真正要发请求的时刻。远离批启动
            // 快照越久（长批尾部的任务），越可能有别的路径已写入结果。
            let db_check = db.clone();
            let done = already_done.clone();
            let skip = tokio::task::spawn_blocking(move || {
                let conn = match db_check.get() {
                    Ok(c) => c,
                    // 复查失败不阻塞：按未处理继续，宁可重复一次也不漏翻
                    Err(e) => {
                        log::error!("数据库连接失败: {}", e);
                        return false;
                    }
                };
                done(&conn, release_id)
            })
            .await
            .unwrap_or(false);
            if skip {
                log::info!(
                    "跳过{} id={}：调用前复查已有结果（可能由其他路径写入）",
                    job,
                    release_id
                );
                return;
            }
            match call(client, model.clone(), base_url, prompt, truncated, on_chunk).await {
                Ok((result, usages)) => {
                    // 同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
                    let _ = tokio::task::spawn_blocking(move || {
                        let conn = match db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("数据库连接失败: {}", e);
                                return;
                            }
                        };
                        persist_call_usages(&conn, release_id, &model, &usages);
                        on_ok(&conn, release_id, result);
                    })
                    .await;
                }
                Err((e, usages)) => {
                    log::error!("生成{}失败 id={}: {}", job, release_id, e);
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(conn) = db.get() {
                            persist_call_usages(&conn, release_id, &model, &usages);
                            on_err(&conn, release_id, &e);
                        }
                    })
                    .await;
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }
}

pub async fn generate_summaries_for_new(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
) {
    run_ai_job(
        db_pool,
        deepseek_semaphore,
        saved,
        "摘要",
        4000,
        DEEPSEEK_TIMEOUT_SECS_SUMMARY,
        // 摘要不走流式（无人逐片展示，卡片只显示最终摘要）
        None,
        |_| true,
        has_ai_summary,
        |client, model, base_url, prompt, text, _sink| async move {
            call_summary(&client, &model, &base_url, &prompt, &text).await
        },
        |conn, release_id, (summary, importance)| {
            if let Err(e) = db::releases::set_ai_summary(conn, release_id, &summary, &importance) {
                log::error!("保存摘要失败 id={}: {}", release_id, e);
            } else {
                log_ai_job_result(
                    conn,
                    "INFO",
                    "AI 摘要",
                    release_id,
                    &format!("重要度={}", importance),
                    true,
                );
            }
        },
        |conn, release_id, e| {
            let _ = db::releases::increment_retry_count(conn, release_id);
            log_ai_job_result(conn, "ERROR", "AI 摘要", release_id, e, false);
        },
    )
    .await;
}

/// 为新增的 releases 生成全文翻译。与摘要任务共用 `deepseek_semaphore`，
/// 保证 AI 请求总并发不变。
/// - `force=false`：仅在 `deepseek_translate_release=true` 且已配置 API key 时生效（轮询自动场景）
/// - `force=true`：绕过 `translate_enabled` 开关，只要 AI 已启用且配置 key 即翻译（手动单条场景）
///
/// 无流式出口：这里都是后台/无观察者的批，逐片发事件无人消费（需要流式分片的手动
/// 单条翻译走 `translate_single_release`）。
pub async fn generate_translations_for_new(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    force: bool,
) {
    run_translate_job(db_pool, deepseek_semaphore, saved, force, None).await;
}

/// 用户在弹窗里点「翻译」的单条翻译：force 语义（绕过开关与「已有译文」复查）+
/// 可选流式分片出口。
///
/// 独立入口而非给 `generate_translations_for_new` 多加一个 bool/`Option`
/// 参数：调用点写得出「这是手动单条 + 要流式」的意图，不用去猜裸参含义。
pub async fn translate_single_release(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    release_id: i64,
    body: String,
    on_chunk: Option<ChunkSink>,
) {
    run_translate_job(
        db_pool,
        deepseek_semaphore,
        &[(release_id, Some(body))],
        true,
        on_chunk,
    )
    .await;
}

pub(super) async fn run_translate_job(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    force: bool,
    on_chunk: Option<ChunkSink>,
) {
    // 目标语言供 call 闭包（语言检测短路）与 on_ok（短路日志）使用，提前读取。
    let target_lang = {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("数据库连接失败: {}", e);
                return;
            }
        };
        read_translate_config(&conn).1
    };
    let target_lang_log = target_lang.clone();
    run_ai_job(
        db_pool,
        deepseek_semaphore,
        saved,
        "译文",
        // 翻译全文比摘要耗 token 多：截断上限（20000 字符）与 max_tokens（20000）
        // 必须对齐到同一量级——中英互译后输出装不下输入时，长 release note 的
        // 译文会在句子中间被硬截断。
        DEEPSEEK_TRANSLATE_TRUNCATE_CHARS,
        // 翻译超时 300s：max_tokens=20000 的长生成，中转要等上游出完才回
        // 响应头，60s 会在网关排队时超时（流式路径不用它，见 build_stream_client）
        DEEPSEEK_TIMEOUT_SECS_TRANSLATE,
        // 流式出口：仅手动单条翻译注入
        on_chunk,
        // 翻译开关：force 绕过 translate_enabled（手动单条场景）
        move |conn| read_translate_config(conn).0 || force,
        // 复查跳过：仅对自动批（force=false）生效。手动单条翻译（force=true）
        // 是用户显式要求重翻（可能上一版译文不满意），必须放行，不能被
        // 「已有译文」挡住——否则设置页点了翻译却毫无反应、也不报错。
        move |conn, id| !force && has_translation(conn, id),
        move |client, model, base_url, _prompt, text, sink| {
            let target_lang = target_lang.clone();
            async move {
                // 语言检测短路：取 body 前 500 字符让 AI 判断主体语言，
                // 若与目标语言一致则直接把原文返回（由 on_ok 写入 body_translated），
                // 跳过翻译调用。检测失败时不阻塞翻译（视为语言不一致，照常翻译），
                // 但检测侧已产生的用量仍要收集。
                let sample: String = text.chars().take(500).collect();
                let mut usages: Vec<CallUsage> = Vec::new();
                match call_detect_language(&client, &model, &base_url, &sample).await {
                    Ok((detected, mut detect_usages)) => {
                        usages.append(&mut detect_usages);
                        if detected.trim() == target_lang {
                            // 短路没有流式可言（原文即译文）：保持既有行为，
                            // 一次性写库后由 ReleaseStateChanged 驱动前端切视图
                            return Ok((TranslateOutcome::Skipped(text), usages));
                        }
                    }
                    Err((_e, mut detect_usages)) => usages.append(&mut detect_usages),
                }
                // 不能用 `?`：Err 直接传播会把局部 usages（语言检测的用量）丢掉，
                // 翻译失败时检测侧已消耗的调用就漏记了。
                let translate_outcome = match sink.as_deref() {
                    Some(sink) => {
                        call_translate_streaming(&client, &model, &base_url, &target_lang, &text, sink)
                            .await
                    }
                    None => call_translate(&client, &model, &base_url, &target_lang, &text).await,
                };
                let (translated, mut translate_usages) = match translate_outcome {
                    Ok(v) => v,
                    Err((e, mut translate_usages)) => {
                        usages.append(&mut translate_usages);
                        return Err((e, usages));
                    }
                };
                usages.append(&mut translate_usages);
                Ok((TranslateOutcome::Translated(translated), usages))
            }
        },
        move |conn, release_id, outcome| match outcome {
            TranslateOutcome::Skipped(original) => {
                if let Err(e) = db::releases::set_body_translated(conn, release_id, &original) {
                    log::error!("保存译文失败 id={}: {}", release_id, e);
                } else {
                    log::info!(
                        "跳过翻译(语言一致): id={} lang={}",
                        release_id,
                        target_lang_log
                    );
                }
            }
            TranslateOutcome::Translated(translated) => {
                if let Err(e) = db::releases::set_body_translated(conn, release_id, &translated) {
                    log::error!("保存译文失败 id={}: {}", release_id, e);
                } else {
                    log_ai_job_result(conn, "INFO", "AI 译文", release_id, "", true);
                }
            }
        },
        |conn, release_id, e| {
            let _ = db::releases::increment_translate_retry_count(conn, release_id);
            log_ai_job_result(conn, "ERROR", "AI 译文", release_id, e, false);
        },
    )
    .await;
}

