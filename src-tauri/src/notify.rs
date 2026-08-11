// ═══════════════════════════════════════════════════════════════
// 跨平台纯函数（三平台实现共用，独立于桌面通知 API，可直接单测）
// ═══════════════════════════════════════════════════════════════

/// 通知标题：`owner / repo`；repo 为空时仅显示 owner（视频源无仓库概念）。
pub(crate) fn notification_title(owner: &str, repo: &str) -> String {
    if repo.is_empty() {
        owner.to_string()
    } else {
        format!("{} / {}", owner, repo)
    }
}

/// 通知正文：`tag - name`；tag 为空（视频源 videoId/bvid 对用户无意义，标题即正文）时
/// 仅显示 name。带重要度时追加中文 label（后端 ai_importance 存中文枚举）。
/// 此前 Windows/Linux/macOS 三份实现各复制一份，语义漂移风险高（见阶段 2-2）。
pub(crate) fn notification_body(tag: &str, name: &str, importance: Option<&str>) -> String {
    let base = if tag.is_empty() {
        name.to_string()
    } else {
        format!("{} - {}", tag, name)
    };
    match importance {
        Some(imp) => {
            let label = match imp {
                "大" => "重要度: 🔴 大",
                "中" => "重要度: 🟡 中",
                _ => "重要度: 🟢 小",
            };
            format!("{}  |  {}", base, label)
        }
        None => base,
    }
}

/// Windows 通知按钮动作（toast 按钮回调字符串格式 `go:<rid>` 等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WinNotificationAction {
    Go,
    Ignore,
    Snooze,
}

/// 解析 Windows 按钮动作字符串；rid 解析失败回退 0（与原 `unwrap_or(0)` 行为一致）。
pub(crate) fn parse_win_action(action: &str) -> Option<(WinNotificationAction, i64)> {
    let (kind, rest) = if let Some(rest) = action.strip_prefix("go:") {
        (WinNotificationAction::Go, rest)
    } else if let Some(rest) = action.strip_prefix("ignore:") {
        (WinNotificationAction::Ignore, rest)
    } else if let Some(rest) = action.strip_prefix("snooze:") {
        (WinNotificationAction::Snooze, rest)
    } else {
        return None;
    };
    Some((kind, rest.parse().unwrap_or(0)))
}

// ═══════════════════════════════════════════════════════════════
// Windows 实现 (WinRT Toast 通知)
// ═══════════════════════════════════════════════════════════════
#[cfg(windows)]
mod inner {
    use tauri::AppHandle;
        use tauri_specta::Event;
    use tauri::Manager;
    use tauri_plugin_opener::OpenerExt;
    use tauri_winrt_notification::Toast;
    use serde_json::json;

    /// 每个线程独立的 COM 守卫。
    /// 首次在线程上访问时调用 CoInitializeEx，线程退出时自动调用 CoUninitialize。
    struct ComGuard;
    impl ComGuard {
        fn new() -> Self {
            let _result = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                )
            };
            ComGuard
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
        }
    }

    thread_local! {
        static COM_CTX: ComGuard = ComGuard::new();
    }

    pub fn ensure_com() {
        COM_CTX.with(|_| {});
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_release_notification(
        app: &AppHandle,
        release_id: i64,
        html_url: String,
        owner: String,
        repo: String,
        tag: String,
        name: String,
        importance: Option<String>,
    ) {
        ensure_com();
        let app_handle = app.clone();
        let go_url = html_url;
        let rid = release_id;

        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let title = crate::notify::notification_title(&owner, &repo);
            let body = crate::notify::notification_body(&tag, &name, importance.as_deref());
            let toast = Toast::new(Toast::POWERSHELL_APP_ID)
                .title(&title)
                .text1(&body)
                .add_button("前往", &format!("go:{}", rid))
                .add_button("忽略", &format!("ignore:{}", rid))
                .add_button("稍后提醒", &format!("snooze:{}", rid))
                .on_activated(move |action| {
                    log::info!("通知按钮回调: {:?}", action);
                    if let Some(action) = action {
                        let (kind, rid) = match crate::notify::parse_win_action(&action) {
                            Some(p) => p,
                            // 未知动作（如系统关闭回调）不处理
                            None => return Ok(()),
                        };
                        let app = app_handle.clone();
                        let state = app.state::<crate::types::AppState>();
                        if let Ok(conn) = state.db.get() {
                            match kind {
                                crate::notify::WinNotificationAction::Go => {
                                    let rel = crate::db::releases::get_release(&conn, rid).ok().flatten();
                                    let _ = crate::db::releases::set_notification_state(
                                        &conn, rid, "clicked", None,
                                    );
                                    match rel {
                                        Some(r) => {
                                            let (log_owner, log_repo, log_tag) = crate::db::logs::release_log_ident(&r);
                                            crate::db::logs::write_log_key(
                                                &conn,
                                                "INFO",
                                                "release.go",
                                                &json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": rid}).to_string(),
                                            )
                                        }
                                        None => crate::db::logs::write_log_key(
                                            &conn,
                                            "INFO",
                                            "release.go_unknown",
                                            &json!({"id": rid}).to_string(),
                                        ),
                                    }
                                    let _ = crate::events::ReleaseStateChanged(rid).emit(&app);
                                    drop(conn);
                                    if !go_url.is_empty() {
                                        if let Err(e) = app.opener().open_url(&go_url, None::<&str>) {
                                            log::error!("打开浏览器失败: {}", e);
                                        } else {
                                            log::info!("打开浏览器: {}", go_url);
                                        }
                                    }
                                }
                                crate::notify::WinNotificationAction::Ignore => {
                                    let rel =
                                        crate::db::releases::get_release(&conn, rid).ok().flatten();
                                    let _ = crate::db::releases::set_notification_state(
                                        &conn, rid, "ignored", None,
                                    );
                                    match rel {
                                        Some(r) => {
                                            let (log_owner, log_repo, log_tag) = crate::db::logs::release_log_ident(&r);
                                            crate::db::logs::write_log_key(
                                                &conn,
                                                "INFO",
                                                "release.ignored",
                                                &json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": rid}).to_string(),
                                            )
                                        }
                                        None => crate::db::logs::write_log_key(
                                            &conn,
                                            "INFO",
                                            "release.ignored_unknown",
                                            &json!({"id": rid}).to_string(),
                                        ),
                                    }
                                    let _ = crate::events::ReleaseStateChanged(rid).emit(&app);
                                }
                                crate::notify::WinNotificationAction::Snooze => {
                                    let rel =
                                        crate::db::releases::get_release(&conn, rid).ok().flatten();
                                    let until =
                                        chrono::Utc::now() + chrono::Duration::hours(24);
                                    let _ = crate::db::releases::set_notification_state(
                                        &conn,
                                        rid,
                                        "snoozed",
                                        Some(&until.to_rfc3339()),
                                    );
                                    match rel {
                                        Some(r) => {
                                            let (log_owner, log_repo, log_tag) = crate::db::logs::release_log_ident(&r);
                                            crate::db::logs::write_log_key(
                                                &conn,
                                                "INFO",
                                                "release.snoozed",
                                                &json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": rid}).to_string(),
                                            )
                                        }
                                        None => crate::db::logs::write_log_key(
                                            &conn,
                                            "INFO",
                                            "release.snoozed_unknown",
                                            &json!({"id": rid}).to_string(),
                                        ),
                                    }
                                    let _ = crate::events::ReleaseStateChanged(rid).emit(&app);
                                }
                            }
                        } else {
                            log::error!("通知回调无法获取数据库连接");
                        }
                    }
                    Ok(())
                });

            toast.show()?;
            log::info!("通知已发送: {}/{} {}", owner, repo, tag);
            Ok(())
        })();

        if let Err(e) = result {
            log::error!("发送通知失败: {}", e);
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Linux 实现 (notify-rust D-Bus 通知)
// ═══════════════════════════════════════════════════════════════
#[cfg(all(unix, not(target_os = "macos")))]
mod inner {
    use tauri::{AppHandle, Manager};
    use tauri_plugin_opener::OpenerExt;
    // ReleaseStateChanged(...).emit() 来自 tauri_specta::Event trait（与 Windows 分支同源）
    use tauri_specta::Event;

    #[allow(clippy::too_many_arguments)]
    pub fn send_release_notification(
        app: &AppHandle,
        release_id: i64,
        html_url: String,
        owner: String,
        repo: String,
        tag: String,
        name: String,
        importance: Option<String>,
    ) {
        let title = crate::notify::notification_title(&owner, &repo);
        let body = crate::notify::notification_body(&tag, &name, importance.as_deref());

        let handle = match notify_rust::Notification::new()
            .summary(&title)
            .body(&body)
            .appname("RelWatch")
            .action("go", "前往")
            .action("ignore", "忽略")
            .action("snooze", "稍后提醒")
            .show()
        {
            Ok(h) => h,
            Err(e) => {
                log::error!("发送通知失败: {}", e);
                return;
            }
        };

        log::info!("通知已发送: {}/{} {}", owner, repo, tag);

        let go_url = html_url;
        let rid = release_id;
        let app_handle = app.clone();

        // wait_for_action 会阻塞当前线程，在独立线程中等待用户操作
        std::thread::spawn(move || {
            let _ = handle.wait_for_action(move |action| {
                log::info!("通知按钮回调: {:?}", action);
                let app = app_handle.clone();

                match action {
                    "go" => {
                        let state = app.state::<crate::types::AppState>();
                        if let Ok(conn) = state.db.get() {
                            let rel =
                                crate::db::releases::get_release(&conn, rid).ok().flatten();
                            let _ = crate::db::releases::set_notification_state(
                                &conn, rid, "clicked", None,
                            );
                            match rel {
                                Some(r) => {
                                    let (log_owner, log_repo, log_tag) = crate::db::logs::release_log_ident(&r);
                                    crate::db::logs::write_log_key(
                                        &conn,
                                        "INFO",
                                        "release.go",
                                        &serde_json::json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": rid}).to_string(),
                                    )
                                }
                                None => crate::db::logs::write_log_key(
                                    &conn,
                                    "INFO",
                                    "release.go_unknown",
                                    &serde_json::json!({"id": rid}).to_string(),
                                ),
                            }
                            let _ = crate::events::ReleaseStateChanged(rid).emit(&app);
                            drop(conn);
                        } else {
                            log::error!("通知回调无法获取数据库连接");
                        }
                        if !go_url.is_empty() {
                            if let Err(e) = app.opener().open_url(&go_url, None::<&str>) {
                                log::error!("打开浏览器失败: {}", e);
                            } else {
                                log::info!("打开浏览器: {}", go_url);
                            }
                        }
                    }
                    "ignore" => {
                        let state = app.state::<crate::types::AppState>();
                        if let Ok(conn) = state.db.get() {
                            let rel =
                                crate::db::releases::get_release(&conn, rid).ok().flatten();
                            let _ = crate::db::releases::set_notification_state(
                                &conn, rid, "ignored", None,
                            );
                            match rel {
                                Some(r) => {
                                    let (log_owner, log_repo, log_tag) = crate::db::logs::release_log_ident(&r);
                                    crate::db::logs::write_log_key(
                                        &conn,
                                        "INFO",
                                        "release.ignored",
                                        &serde_json::json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": rid}).to_string(),
                                    )
                                }
                                None => crate::db::logs::write_log_key(
                                    &conn,
                                    "INFO",
                                    "release.ignored_unknown",
                                    &serde_json::json!({"id": rid}).to_string(),
                                ),
                            }
                            let _ = crate::events::ReleaseStateChanged(rid).emit(&app);
                        } else {
                            log::error!("通知回调无法获取数据库连接");
                        }
                    }
                    "snooze" => {
                        let state = app.state::<crate::types::AppState>();
                        if let Ok(conn) = state.db.get() {
                            let rel =
                                crate::db::releases::get_release(&conn, rid).ok().flatten();
                            let until =
                                chrono::Utc::now() + chrono::Duration::hours(24);
                            let _ = crate::db::releases::set_notification_state(
                                &conn,
                                rid,
                                "snoozed",
                                Some(&until.to_rfc3339()),
                            );
                            match rel {
                                Some(r) => {
                                    let (log_owner, log_repo, log_tag) = crate::db::logs::release_log_ident(&r);
                                    crate::db::logs::write_log_key(
                                        &conn,
                                        "INFO",
                                        "release.snoozed",
                                        &serde_json::json!({"owner": &log_owner, "repo": &log_repo, "tag": &log_tag, "id": rid}).to_string(),
                                    )
                                }
                                None => crate::db::logs::write_log_key(
                                    &conn,
                                    "INFO",
                                    "release.snoozed_unknown",
                                    &serde_json::json!({"id": rid}).to_string(),
                                ),
                            }
                            let _ = crate::events::ReleaseStateChanged(rid).emit(&app);
                        } else {
                            log::error!("通知回调无法获取数据库连接");
                        }
                    }
                    "__closed" => {
                        // 通知被关闭，无需操作
                    }
                    _ => {}
                }
            });
        });
    }
}

// ═══════════════════════════════════════════════════════════════
// macOS 实现 (osascript 通知)
// ═══════════════════════════════════════════════════════════════
#[cfg(target_os = "macos")]
mod inner {
    use tauri::AppHandle;

    /// 使用 osascript display notification 发送 macOS 通知。
    ///
    /// 注意: osascript 不支持动作按钮，因此与 Windows/Linux 不同，
    /// 本实现不包含「前往」「忽略」「稍后提醒」按钮。
    /// TODO: 未来可改用 UNUserNotificationCenter 或 notify-rust 以获得按钮支持。
    #[allow(clippy::too_many_arguments)]
    pub fn send_release_notification(
        _app: &AppHandle,
        _release_id: i64,
        _html_url: String,
        owner: String,
        repo: String,
        tag: String,
        name: String,
        importance: Option<String>,
    ) {
        use std::process::Command;

        fn escape_apple(s: &str) -> String {
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', " ")
                .replace('\r', " ")
        }

        let title = crate::notify::notification_title(&owner, &repo);
        let body = crate::notify::notification_body(&tag, &name, importance.as_deref());

        let script = format!(
            r#"display notification "{}" with title "{}" subtitle "RelWatch""#,
            escape_apple(&body),
            escape_apple(&title),
        );

        match Command::new("osascript").arg("-e").arg(&script).output() {
            Ok(out) if out.status.success() => {
                log::info!("通知已发送: {} / {} {}", owner, repo, tag);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                log::error!("osascript 发送通知失败: {}", stderr);
            }
            Err(e) => {
                log::error!("无法执行 osascript: {}", e);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 未知平台回退（仅记录日志，无桌面通知）
// ═══════════════════════════════════════════════════════════════
#[cfg(not(any(windows, target_os = "macos", all(unix, not(target_os = "macos")))))]
mod inner {
    use tauri::AppHandle;

    #[allow(clippy::too_many_arguments)]
    pub fn send_release_notification(
        _app: &AppHandle,
        _release_id: i64,
        _html_url: String,
        _owner: String,
        _repo: String,
        _tag: String,
        _name: String,
        _importance: Option<String>,
    ) {
        log::info!("通知: 当前平台不支持桌面通知");
    }
}

// ── Re-export ─────────────────────────────────────────
pub use inner::send_release_notification;

// ── 权限请求（跨平台空操作）─────────────────────────
pub fn request_permission(_app: &tauri::AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_title_owner_repo() {
        assert_eq!(notification_title("torvalds", "linux"), "torvalds / linux");
    }

    #[test]
    fn notification_title_empty_repo_owner_only() {
        // 视频源（YouTube/B 站）无仓库概念，repo 为空 → 仅显示可读源名
        assert_eq!(notification_title("某频道", ""), "某频道");
        assert_eq!(notification_title("频道名", ""), "频道名");
    }

    #[test]
    fn notification_body_without_importance() {
        assert_eq!(notification_body("v1.0.0", "Release v1", None), "v1.0.0 - Release v1");
    }

    #[test]
    fn notification_body_empty_tag_name_only() {
        // 视频源 tag（videoId/bvid）无意义 → 正文仅显示视频标题
        assert_eq!(notification_body("", "视频标题", None), "视频标题");
    }

    #[test]
    fn notification_body_empty_tag_with_importance() {
        assert_eq!(
            notification_body("", "视频标题", Some("大")),
            "视频标题  |  重要度: 🔴 大"
        );
    }

    #[test]
    fn notification_body_importance_high() {
        assert_eq!(
            notification_body("v1.0.0", "Release v1", Some("大")),
            "v1.0.0 - Release v1  |  重要度: 🔴 大"
        );
    }

    #[test]
    fn notification_body_importance_medium() {
        assert_eq!(
            notification_body("v1.0.0", "Release v1", Some("中")),
            "v1.0.0 - Release v1  |  重要度: 🟡 中"
        );
    }

    #[test]
    fn notification_body_importance_low_and_unknown_fallback() {
        assert_eq!(
            notification_body("v1.0.0", "Release v1", Some("小")),
            "v1.0.0 - Release v1  |  重要度: 🟢 小"
        );
        // 未知重要度值兜底为“小”
        assert_eq!(
            notification_body("v1.0.0", "Release v1", Some("未知")),
            "v1.0.0 - Release v1  |  重要度: 🟢 小"
        );
    }

    #[test]
    fn notification_body_escapes_nothing_but_keeps_empty_fields() {
        // tag/name 为空时保留占位格式（不 panic、不吞字段）；tag 空则仅显示 name
        assert_eq!(notification_body("", "", None), "");
        assert_eq!(notification_body("", "Release v1", None), "Release v1");
        assert_eq!(notification_body("v1.0.0", "", None), "v1.0.0 - ");
    }

    #[test]
    fn parse_win_action_go_ignore_snooze() {
        assert_eq!(
            parse_win_action("go:42"),
            Some((WinNotificationAction::Go, 42))
        );
        assert_eq!(
            parse_win_action("ignore:7"),
            Some((WinNotificationAction::Ignore, 7))
        );
        assert_eq!(
            parse_win_action("snooze:99"),
            Some((WinNotificationAction::Snooze, 99))
        );
    }

    #[test]
    fn parse_win_action_invalid_rid_falls_back_to_zero() {
        // 与原 strip_prefix + unwrap_or(0) 行为一致
        assert_eq!(
            parse_win_action("go:abc"),
            Some((WinNotificationAction::Go, 0))
        );
        assert_eq!(parse_win_action("ignore:"), Some((WinNotificationAction::Ignore, 0)));
    }

    #[test]
    fn parse_win_action_unknown_returns_none() {
        assert_eq!(parse_win_action("__closed"), None);
        assert_eq!(parse_win_action(""), None);
        assert_eq!(parse_win_action("open:1"), None);
        assert_eq!(parse_win_action("go"), None);
    }

    #[test]
    fn parse_win_action_negative_rid() {
        assert_eq!(parse_win_action("go:-1"), Some((WinNotificationAction::Go, -1)));
    }
}
