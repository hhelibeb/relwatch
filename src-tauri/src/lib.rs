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

use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};
use tauri::Manager;

use types::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let conn = db::init::init_db().expect("Failed to initialize database");

    {
        if db::settings::get_setting(&conn, "poll_interval_minutes")
            .unwrap_or(None)
            .is_none()
        {
            let _ = db::settings::set_setting(&conn, "poll_interval_minutes", "30");
        }
    }

    let next_poll_val = {
        let interval: i64 = db::settings::get_setting(&conn, "poll_interval_minutes")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        if let Some(last_str) = db::settings::get_setting(&conn, "last_poll_at")
            .ok()
            .flatten()
        {
            if let Ok(last) = last_str.parse::<i64>() {
                let candidate = last + interval * 60;
                candidate.max(chrono::Utc::now().timestamp())
            } else {
                chrono::Utc::now().timestamp() + interval * 60
            }
        } else {
            chrono::Utc::now().timestamp() + interval * 60
        }
    };
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
            db: Mutex::new(conn),
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
                        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
                        let minimize = db::settings::get_setting(&conn, "minimize_to_tray")
                            .ok()
                            .flatten()
                            .map(|v| v == "true")
                            .unwrap_or(true);
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
