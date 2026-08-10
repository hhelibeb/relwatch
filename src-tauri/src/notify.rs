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
            let title = format!("{} / {}", owner, repo);
            let body = if let Some(ref imp) = importance {
                let label = match imp.as_str() {
                    "大" => "重要度: 🔴 大",
                    "中" => "重要度: 🟡 中",
                    _ => "重要度: 🟢 小",
                };
                format!("{} - {}  |  {}", tag, name, label)
            } else {
                format!("{} - {}", tag, name)
            };
            let toast = Toast::new(Toast::POWERSHELL_APP_ID)
                .title(&title)
                .text1(&body)
                .add_button("前往", &format!("go:{}", rid))
                .add_button("忽略", &format!("ignore:{}", rid))
                .add_button("稍后提醒", &format!("snooze:{}", rid))
                .on_activated(move |action| {
                    log::info!("通知按钮回调: {:?}", action);
                    if let Some(action) = action {
                        let app = app_handle.clone();
                        let state = app.state::<crate::types::AppState>();
                        if let Ok(conn) = state.db.get() {
                            if let Some(rest) = action.strip_prefix("go:") {
                                let rid: i64 = rest.parse().unwrap_or(0);
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
                            } else if let Some(rest) = action.strip_prefix("ignore:") {
                                let rid: i64 = rest.parse().unwrap_or(0);
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
                            } else if let Some(rest) = action.strip_prefix("snooze:") {
                                let rid: i64 = rest.parse().unwrap_or(0);
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
        let title = format!("{} / {}", owner, repo);
        let body = if let Some(ref imp) = importance {
            let label = match imp.as_str() {
                "大" => "重要度: 🔴 大",
                "中" => "重要度: 🟡 中",
                _ => "重要度: 🟢 小",
            };
            format!("{} - {}  |  {}", tag, name, label)
        } else {
            format!("{} - {}", tag, name)
        };

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

        let title = format!("{} / {}", owner, repo);
        let body = if let Some(ref imp) = importance {
            let label = match imp.as_str() {
                "大" => "重要度: 🔴 大",
                "中" => "重要度: 🟡 中",
                _ => "重要度: 🟢 小",
            };
            format!("{} - {}  |  {}", tag, name, label)
        } else {
            format!("{} - {}", tag, name)
        };

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
