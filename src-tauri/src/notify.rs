use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tauri_winrt_notification::Toast;
use serde_json::json;

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
                    let conn = state.db.get().unwrap();

                    if let Some(rest) = action.strip_prefix("go:") {
                        let rid: i64 = rest.parse().unwrap_or(0);
                        let rel = crate::db::releases::get_release(&conn, rid).ok().flatten();
                        let _ = crate::db::releases::set_notification_state(&conn, rid, "clicked", None);
                        match rel {
                            Some(r) => crate::db::logs::write_log_key(&conn, "INFO", "release.go", &json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid}).to_string()),
                            None => crate::db::logs::write_log_key(&conn, "INFO", "release.go_unknown", &json!({"id": rid}).to_string()),
                        }
                        drop(conn);
                        if !go_url.is_empty() {
                            if let Err(e) = app_handle.opener().open_url(&go_url, None::<&str>) {
                                log::error!("打开浏览器失败: {}", e);
                            } else {
                                log::info!("打开浏览器: {}", go_url);
                            }
                        }
                    } else if let Some(rest) = action.strip_prefix("ignore:") {
                        let rid: i64 = rest.parse().unwrap_or(0);
                        let rel = crate::db::releases::get_release(&conn, rid).ok().flatten();
                        let _ = crate::db::releases::set_notification_state(&conn, rid, "ignored", None);
                        match rel {
                            Some(r) => crate::db::logs::write_log_key(&conn, "INFO", "release.ignored", &json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid}).to_string()),
                            None => crate::db::logs::write_log_key(&conn, "INFO", "release.ignored_unknown", &json!({"id": rid}).to_string()),
                        }
                    } else if let Some(rest) = action.strip_prefix("snooze:") {
                        let rid: i64 = rest.parse().unwrap_or(0);
                        let rel = crate::db::releases::get_release(&conn, rid).ok().flatten();
                        let until = chrono::Utc::now() + chrono::Duration::minutes(60);
                        let _ = crate::db::releases::set_notification_state(&conn, rid, "snoozed", Some(&until.to_rfc3339()));
                        match rel {
                            Some(r) => crate::db::logs::write_log_key(&conn, "INFO", "release.snoozed", &json!({"owner": &r.owner, "repo": &r.repo, "tag": &r.tag_name, "id": rid}).to_string()),
                            None => crate::db::logs::write_log_key(&conn, "INFO", "release.snoozed_unknown", &json!({"id": rid}).to_string()),
                        }
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
