use serde_json::json;

use crate::db;
use crate::types::ReleaseNotifyParams;

pub(super) async fn collect_pending_and_notify(
    db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    emitter: &dyn crate::types::Emitter,
    new_ids: &[i64],
    is_manual: bool,
) -> (Vec<db::releases::ReleaseInfo>, Vec<db::releases::ReleaseInfo>) {
    // 读 pending + muted 并把本轮 pending 批量标记已通知，收在同一次取连接里；
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
        // 视频源（YouTube/B 站）：标题用频道名/UP 主名（description，channel_id/UID 无阅读
        // 意义）并加来源标签（`哔哩哔哩 / UP主名`），正文隐藏 videoId/bvid 只留视频标题。
        // 能力声明在 SourceAdapter，避免特判。
        let adapter = crate::source::get_adapter(&release.source_type).ok();
        let display_name = adapter
            .as_ref()
            .map(|a| {
                a.notification_source_name(
                    &release.owner,
                    &release.repo,
                    release.source_description.as_deref(),
                )
            })
            .unwrap_or_else(|| release.owner.clone());
        let notify_owner = match adapter.as_ref().and_then(|a| a.notification_source_label()) {
            Some(label) => format!("{} / {}", label, display_name),
            None => display_name,
        };
        let notify_tag = if adapter.is_none_or(|a| a.notification_show_tag()) {
            release.tag_name.clone()
        } else {
            String::new()
        };
        emitter.notify_release(ReleaseNotifyParams {
            release_id: release.id,
            html_url: release.html_url.clone(),
            owner: notify_owner,
            repo: release.repo.clone(),
            tag: notify_tag,
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

