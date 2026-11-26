use std::sync::atomic::{AtomicBool, Ordering};

use crate::db;
use crate::deepseek;
use crate::source;

/// 收集不参与 AI 摘要/翻译的源类型（如 youtube/bilibili）。
///
/// 由 `list_adapters()` 的能力声明 `ai_eligible()=false` 动态枚举，
/// 新增源类型在适配器登记后自动生效，即时路径与重试路径共用同一份能力声明，
/// 避免 SQL 硬编码排除集合遗漏新类型（如 bilibili）。
pub(super) fn ai_excluded_types() -> Vec<&'static str> {
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
pub(super) async fn filter_ai_eligible(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    saved: &[(i64, Option<String>)],
) -> Result<Vec<(i64, Option<String>)>, String> {
    if saved.is_empty() {
        return Ok(vec![]);
    }
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

/// 后台派发「待翻译」批的 in-flight 门控：true 表示上一轮派发的后台翻译仍在跑。
/// 翻译批最长可跑 ~300s（单条超时 × 并发），期间若又有新轮次到达（手动立即
/// 检查 / 定时轮询），不重复派发——直接跳过本轮，批在跑期间新到的待办由
/// **下一轮轮询**兜底（`get_releases_without_translation` 重读 DB 一并覆盖）。
///
/// 不用 `Notify` 注册「跟随批」等收尾续跑：`Notify` 不带状态，`notify_one()` 只
/// 唤醒**此刻已注册**的等待者，spawn 后不保证先注册再被 notify，信号会永久丢失
/// （挂起的任务泄漏）。轮询兜底已保证新待办最迟下一轮被翻，不值得这份复杂度。
pub(super) static TRANSLATION_BATCH_RUNNING: AtomicBool = AtomicBool::new(false);

/// 后台批门控的复位守卫：构造即视为「批在跑」（flag 由调用方抢执行权时置位），
/// Drop 时必定复位 flag。
///
/// 复位不能写在 async 体末尾：批内任一步 panic（DB 调用、AI 链路）会跳过那几
/// 行，flag 永久停在 true，此后该进程再不派发该批——静默失效，只能重启恢复。
/// 守卫借栈展开触发 Drop，panic 与正常收尾走同一条复位路径。
pub(super) struct BatchGuard {
    running: &'static AtomicBool,
    /// 日志文案，panic 时标识是哪个批。
    job: &'static str,
}

impl BatchGuard {
    pub(super) fn new(running: &'static AtomicBool, job: &'static str) -> Self {
        BatchGuard { running, job }
    }
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        // 栈展开中正执行 Drop 说明批是 panic 出来的，记一条便于定位（与
        // 既有 spawn_blocking panic 日志的口径一致）。
        if std::thread::panicking() {
            log::error!(
                "后台{}批 panic，门控已复位（否则后续批次将永久停摆）",
                self.job
            );
        }
        self.running.store(false, Ordering::Release);
    }
}

/// 后台翻译调度入口（fire-and-forget）。
///
/// 从 DB 读取「未完成翻译的 release」（= 本轮新入库 + 历史失败重试，两者在
/// fetch→save 后均已入库，`get_releases_without_translation` 一并覆盖），
/// 交给后台任务翻译；**调用方不等待**，检查/通知/轮询节奏不被翻译的最长
/// ~300s 超时拖累。
///
/// 批次并发由 `generate_translations_for_new` 内部按 release spawn + 信号量
/// 限流；失败/超时条目在批内最多试一次，留待下一轮轮询的后台批重读待办续跑
/// （`get_releases_without_translation` 自带 retry_count<5 封顶，不会无限重试）。
///
/// 门控：`TRANSLATION_BATCH_RUNNING` 为 true（上一批在跑）时，**直接跳过本轮**，
/// 不注册任何等待/续跑机制（理由见该静态的文档注释）——批在跑期间新到的
/// release 最迟下一轮轮询被补上，不会漏翻。
pub(super) fn schedule_translations_background(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &std::sync::Arc<dyn crate::types::Emitter>,
) {
    // AI 未启用或翻译未开 → generate_translations_for_new 内部会早返回，白占一批
    if !translation_enabled(db_pool) {
        return;
    }

    // 抢执行权前先在调用线程读一次待办：空则直接 return，不为每轮轮询
    // 都 spawn 一个空任务（轮询密集时避免 task 堆积）。注：与批内的重读
    // 是两次查询，但换来“不 spawn 空任务”，划算。
    if read_translation_pending(db_pool).is_empty() {
        return;
    }

    // 抢占执行权；失败说明上一批在跑，跳过本轮，新待办由下一轮轮询兜底
    if TRANSLATION_BATCH_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }

    // 成功抢占：后台跑一批（fire-and-forget，守卫负责收尾复位门控）
    let pool = db_pool.clone();
    let sem = deepseek_semaphore.clone();
    let emitter = emitter.clone();
    tauri::async_runtime::spawn(async move {
        // 守卫接管复位：批内 panic 也不会让门控永久停在 true
        let _guard = BatchGuard::new(&TRANSLATION_BATCH_RUNNING, "翻译");
        run_single_translation_batch(&pool, &sem, &emitter).await;
    });
}

/// 读库中仍未完成翻译的 release 列表（排除 ai_eligible=false 源）。
///
/// 单条小查询、调用方（do_poll_core / check_single_source 的收尾）本就在
/// 后台上下文，直接同步查即可；无需 spawn_blocking/block_in_place 的额外开销。
pub(super) fn read_translation_pending(
    pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
) -> Vec<(i64, Option<String>)> {
    let excluded = ai_excluded_types();
    let conn = match pool.get() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    db::releases::get_releases_without_translation(&conn, &excluded).unwrap_or_default()
}

/// 读一次 AI 开关（DeepSeek 已启用）。
pub(super) fn ai_enabled(pool: &r2d2::Pool<crate::db::SqliteConnectionManager>) -> bool {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(_) => return false,
    };
    deepseek::read_config(&conn).enabled
}

/// 读一次翻译开关（AI 启用且翻译开启）。
pub(super) fn translation_enabled(
    pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
) -> bool {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let cfg = deepseek::read_config(&conn);
    if !cfg.enabled {
        return false;
    }
    db::settings::get_setting(&conn, db::settings::KEY_DEEPSEEK_TRANSLATE_RELEASE)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false)
}

/// 单批执行：读一次 DB 待办 → 翻译一轮 → 成功落库的逐条 emit 刷新。
/// 只跑一轮：失败条目留给下一轮轮询 retry（DB 队列自带 retry<5 封顶），
/// 避免批内对持续失败的条目死循环。
pub(super) async fn run_single_translation_batch(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &std::sync::Arc<dyn crate::types::Emitter>,
) {
    // 读待办放 spawn_blocking：避免在 tokio worker 同步 rusqlite 阻塞线程池
    let excluded = ai_excluded_types();
    let pool = db_pool.clone();
    let saved = match tokio::task::spawn_blocking(move || {
        let conn = pool.get().ok()?;
        db::releases::get_releases_without_translation(&conn, &excluded).ok()
    })
    .await
    {
        Ok(Some(list)) => list,
        _ => return,
    };
    if saved.is_empty() {
        return;
    }
    log::info!("后台翻译批: {} 条待翻译（含历史失败重试）", saved.len());

    deepseek::generate_translations_for_new(db_pool, deepseek_semaphore, &saved, false).await;

    // 落库结果回查：DB 已带译文 = 成功，逐条 emit 刷新前端（50ms 合帧重拉）
    let pool = db_pool.clone();
    let ok_ids: Vec<i64> = tokio::task::spawn_blocking(move || {
        let conn = match pool.get() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        saved
            .iter()
            .filter(|(id, _)| {
                db::releases::get_release(&conn, *id)
                    .ok()
                    .flatten()
                    .map(|r| r.body_translated.is_some())
                    .unwrap_or(false)
            })
            .map(|(id, _)| *id)
            .collect()
    })
    .await
    .unwrap_or_default();
    if !ok_ids.is_empty() {
        log::info!("后台翻译批完成，{} 条已落库", ok_ids.len());
        for id in ok_ids {
            emitter.emit_release_state_changed(id);
        }
    }
    // 无条件刷新日志：批内无论成功还是失败都写了 DB 日志（成功 INFO / 失败 ERROR）。
    // 本批是 fire-and-forget，收尾时发起它的那轮轮询早已结束，若不在此时通知，
    // 日志 tab 会停在用户切过去那一刻的快照——正是「译文失败了但日志里看不到
    // 原因」的成因。放在末尾覆盖全部写日志路径，不只在成功分支发。
    emitter.emit_log_appended();
}

/// 摘要后台补全批的门控（与翻译批独立互斥）。
///
/// 与翻译侧同构：不设跟随批，批在跑期间新产生的待办留待下一轮轮询补。
/// 摘要超时 120s、远短于翻译的 300s，且轮询会兜底重试，无需额外机制。
pub(super) static SUMMARY_BATCH_RUNNING: AtomicBool = AtomicBool::new(false);

/// 后台补全「历史失败/未完成的 AI 摘要」（fire-and-forget）。
///
/// 不做补全的话，一次失败的摘要将永久空缺（`get_releases_without_summary`
/// 自带 retry<5 封顶，反复补全不会无限重试）。
///
/// 与新 release 的顺序：新 release 的摘要在 do_poll_core 内同步生成（通知带重要
/// 度），此补全只处理**已入库的历史** release，不产生新通知，无顺序冲突。
pub(super) fn schedule_summaries_background(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &std::sync::Arc<dyn crate::types::Emitter>,
) {
    if !ai_enabled(db_pool) {
        return;
    }
    if read_summary_pending(db_pool).is_empty() {
        return;
    }
    // 上一批在跑则放弃本轮：批在跑期间新到的待办由下一轮轮询补（见上方门控注释）
    if SUMMARY_BATCH_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let pool = db_pool.clone();
    let sem = deepseek_semaphore.clone();
    let emitter = emitter.clone();
    tauri::async_runtime::spawn(async move {
        // 守卫接管复位：批内 panic 不会让门控永久停在 true
        let _guard = BatchGuard::new(&SUMMARY_BATCH_RUNNING, "摘要");
        run_single_summary_batch(&pool, &sem, &emitter).await;
    });
}

/// 读库中无 AI 摘要的 release（排除 ai_eligible=false 源）。
pub(super) fn read_summary_pending(
    pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
) -> Vec<(i64, Option<String>)> {
    let excluded = ai_excluded_types();
    let conn = match pool.get() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    db::releases::get_releases_without_summary(&conn, &excluded).unwrap_or_default()
}

/// 单批摘要补全：读一次待办 → 摘要一轮。只跑一轮，失败留给下一轮轮询。
pub(super) async fn run_single_summary_batch(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &std::sync::Arc<dyn crate::types::Emitter>,
) {
    let saved = read_summary_pending(db_pool);
    if saved.is_empty() {
        return;
    }
    log::info!("后台摘要补全批: {} 条待补全", saved.len());
    deepseek::generate_summaries_for_new(db_pool, deepseek_semaphore, &saved).await;
    // 与翻译批同理：无论成败都写了日志，收尾时通知前端刷新日志 tab
    emitter.emit_log_appended();
}

