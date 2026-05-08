pub mod crypto;
pub mod db;
mod commands;
mod notify;
mod tray;
mod types;
mod http;
mod github;
mod deepseek;
mod poll;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;

use types::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pool = db::init::init_pool().expect("Failed to initialize database");

    let next_poll_val;
    {
        let conn = pool.get().expect("Failed to get db connection");
        if db::settings::get_setting(&conn, "poll_interval_minutes")
            .unwrap_or(None)
            .is_none()
        {
            let _ = db::settings::set_setting(&conn, "poll_interval_minutes", "30");
        }

        let now = chrono::Utc::now().timestamp();
        next_poll_val = db::settings::get_setting(&conn, "next_poll_at")
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|&v| v > now)
            .unwrap_or_else(|| {
                let interval = db::settings::get_setting(&conn, "poll_interval_minutes")
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(30);
                now + interval * 60
            });
    }
    let next_poll = Arc::new(AtomicI64::new(next_poll_val));

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
        .manage(AppState {
            db: pool,
            next_poll_at: next_poll.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_source,
            commands::remove_source,
            commands::update_source,
            commands::list_sources,
            commands::get_releases,
            commands::get_pending_releases,
            commands::set_notification_state,
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
            notify::request_permission(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                let app_clone = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let state = app_clone.state::<AppState>();
                        let conn = state.db.get().unwrap_or_else(|e| panic!("DB pool error: {}", e));
                        let minimize = db::settings::get_setting(&conn, "minimize_to_tray")
                            .ok()
                            .flatten()
                            .map(|v| v == "true")
                            .unwrap_or(true);
                        let next = state.next_poll_at.load(Ordering::Relaxed);
                        let _ = db::settings::set_setting(&conn, "next_poll_at", &next.to_string());
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
