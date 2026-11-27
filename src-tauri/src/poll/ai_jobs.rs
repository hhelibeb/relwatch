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
        let conn = pool
            .get()
            .map_err(|e| crate::error::DbError::Lock(e.to_string()))?;
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

pub(super) struct BatchGuard<'a> {
    running: &'a AtomicBool,
    job: &'static str,
}

impl<'a> BatchGuard<'a> {
    pub(super) fn new(running: &'a AtomicBool, job: &'static str) -> Self {
        BatchGuard { running, job }
    }
}

impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            log::error!(
                "后台{}批 panic，门控已复位（否则后续批次将永久停摆）",
                self.job
            );
        }
        self.running.store(false, Ordering::Release);
    }
}

pub(super) fn schedule_translations_background(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    emitter: &std::sync::Arc<dyn crate::types::Emitter>,
) {
    if !translation_enabled(db_pool) {
        return;
    }
    if read_translation_pending(db_pool).is_empty() {
        return;
    }
    if super::RUNTIME.translation_batch_running.swap(true, Ordering::AcqRel) {
        return;
    }

    let pool = db_pool.clone();
    let sem = deepseek_semaphore.clone();
    let emitter = emitter.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = BatchGuard::new(&super::RUNTIME.translation_batch_running, "翻译");
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
    if super::RUNTIME.summary_batch_running.swap(true, Ordering::AcqRel) {
        return;
    }
    let pool = db_pool.clone();
    let sem = deepseek_semaphore.clone();
    let emitter = emitter.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = BatchGuard::new(&super::RUNTIME.summary_batch_running, "摘要");
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

