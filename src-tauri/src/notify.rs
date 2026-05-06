use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tauri_winrt_notification::Toast;

static COM_INIT: OnceLock<()> = OnceLock::new();
static COM_INITIALIZED: AtomicBool = AtomicBool::new(false);

fn ensure_com() {
    COM_INIT.get_or_init(|| {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        COM_INITIALIZED.store(true, Ordering::Relaxed);
    });
}

pub fn uninit_com() {
    if COM_INITIALIZED.swap(false, Ordering::Relaxed) {
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}

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
                    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());

                    if action.starts_with("go:") {
                        let rid: i64 = action[3..].parse().unwrap_or(0);
                        let _ = crate::db::releases::set_notification_state(&conn, rid, "clicked", None);
                        crate::db::logs::write_log(&conn, "INFO", &format!("前往版本 id={}", rid));
                        drop(conn);
                        if !go_url.is_empty() {
                            if let Err(e) = app_handle.opener().open_url(&go_url, None::<&str>) {
                                log::error!("打开浏览器失败: {}", e);
                            } else {
                                log::info!("打开浏览器: {}", go_url);
                            }
                        }
                    } else if action.starts_with("ignore:") {
                        let rid: i64 = action[7..].parse().unwrap_or(0);
                        let _ = crate::db::releases::set_notification_state(&conn, rid, "ignored", None);
                        crate::db::logs::write_log(&conn, "INFO", &format!("忽略版本 id={}", rid));
                    } else if action.starts_with("snooze:") {
                        let rid: i64 = action[7..].parse().unwrap_or(0);
                        let until = chrono::Utc::now() + chrono::Duration::minutes(60);
                        let _ = crate::db::releases::set_notification_state(&conn, rid, "snoozed", Some(&until.to_rfc3339()));
                        crate::db::logs::write_log(&conn, "INFO", &format!("推迟版本 id={}", rid));
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

pub fn request_permission(_app: &AppHandle) {}
