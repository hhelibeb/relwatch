pub mod autostart;
pub mod crypto;
pub mod db;
pub mod i18n;
mod commands;
mod notify;
mod tray;
mod types;
mod http;
mod github;
mod deepseek;
mod poll;
mod retry;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;

use types::AppState;
use db::settings::{KEY_POLL_INTERVAL, KEY_NEXT_POLL_AT, KEY_MINIMIZE_TO_TRAY};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(e) = crypto::initialize_master_key() {
        eprintln!(
            "FATAL: 无法初始化加密密钥: {}\n请确保 OS keyring 可用（Linux 需安装并运行 dbus 和密钥环守护进程）。\nLinux: sudo apt install gnome-keyring 或 secret-service-dbus\nmacOS / Windows: 通常无需额外操作",
            e
        );
        std::process::exit(1);
    }

    let pool = db::init::init_pool().expect("Failed to initialize database");

    // 一致性检查：确保 master key 能解密 DB 中已有的 v2 密文
    // 如果出现不匹配，自动清空对应设置项并打印警告（不阻塞启动）
    {
        let conn = pool.get().expect("Failed to get db connection");
        let cleared = crypto::verify_master_key_consistency(&conn);
        if !cleared.is_empty() {
            eprintln!(
                "WARNING: master key 无法解密以下数据，已自动清空，请重新设置：{}",
                cleared.join(", ")
            );
        }
    }

    let next_poll_val;
    {
        let conn = pool.get().expect("Failed to get db connection");
        if db::settings::get_setting(&conn, KEY_POLL_INTERVAL)
            .unwrap_or(None)
            .is_none()
        {
            let _ = db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30");
        }

        let now = chrono::Utc::now().timestamp();
        next_poll_val = db::settings::get_setting(&conn, KEY_NEXT_POLL_AT)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|&v| v > now)
            .unwrap_or(now);
    }
    let next_poll = Arc::new(AtomicI64::new(next_poll_val));
    let deepseek_semaphore = Arc::new(tokio::sync::Semaphore::new(50));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin({
            use tauri_plugin_prevent_default::Flags;
            let keep = if cfg!(debug_assertions) {
                Flags::FIND | Flags::FOCUS_MOVE | Flags::CONTEXT_MENU
                    | Flags::DEV_TOOLS | Flags::RELOAD
            } else {
                Flags::FIND | Flags::FOCUS_MOVE | Flags::CONTEXT_MENU
            };
            tauri_plugin_prevent_default::Builder::new()
                .with_flags(Flags::all().difference(keep))
                .build()
        })
        .manage(AppState {
            db: pool,
            next_poll_at: next_poll.clone(),
            deepseek_semaphore,
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_source,
            commands::remove_source,
            commands::update_source,
            commands::list_sources,
            commands::get_releases,
            commands::get_pending_releases,
            commands::set_notification_state,
            commands::delete_release,
            commands::get_logs,
            commands::clear_logs,
            commands::trigger_poll,
            commands::check_single_source,
            commands::get_settings,
            commands::update_settings,
            commands::get_poll_countdown,
            commands::set_deepseek_api_key,
            commands::set_github_token,
            commands::test_deepseek_connection,
            commands::search_logs,
            commands::export_backup,
            commands::import_backup,
            commands::hide_to_tray,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            tray::create_tray(app.handle())?;
            tray::setup_tray_listeners(app.handle());
            tray::update_tray_badge(app.handle());
            notify::request_permission(app.handle());

            // 如果是从开机自启动启动的（带有 --autostart 参数），自动隐藏到托盘
            if std::env::args().any(|a| a == "--autostart") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            if let Some(window) = app.get_webview_window("main") {
                let app_clone = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let state = app_clone.state::<AppState>();
                        let conn = match state.db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("关闭窗口时数据库连接失败: {}", e);
                                poll::stop_poll();
                                return;
                            }
                        };
                        let minimize = db::settings::get_setting(&conn, KEY_MINIMIZE_TO_TRAY)
                            .ok()
                            .flatten()
                            .map(|v| v == "true")
                            .unwrap_or(true);
                        let next = state.next_poll_at.load(Ordering::Relaxed);
                        let _ = db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
                        drop(conn);
                        if minimize {
                            api.prevent_close();
                            if let Some(w) = app_clone.get_webview_window("main") {
                                let _ = w.hide();
                            }
                        } else {
                            poll::stop_poll();
                        }
                    }
                });
            }

            let app_handle = app.handle().clone();
            poll::start_poll_thread(app_handle, next_poll);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
