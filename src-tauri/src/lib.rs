pub mod autostart;
pub mod crypto;
pub mod credential;
pub mod db;
pub mod i18n;
mod commands;
mod events;
mod focus;
mod notify;
mod tray;
mod types;
mod http;
mod github;
mod huggingface;
mod youtube;
mod bilibili;
pub mod source;
mod deepseek;
mod poll;
mod retry;
mod net;
mod media;
mod redact;
pub mod agent;
pub mod agent_context;
pub mod agent_rpc;
pub mod agent_session;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands};

use types::AppState;
use db::settings::{KEY_POLL_INTERVAL, KEY_NEXT_POLL_AT, KEY_MINIMIZE_TO_TRAY};

/// tauri-specta Builder：为全部命令收集类型信息，供生成前端 TS 绑定。
/// 单独提取为函数，便于 `cargo test` 触发导出（CI 可复现生成）；
/// release 构建同样需要它（invoke_handler 与事件注册）。
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        // i64 id 在 JS 侧无精度风险（SQLite rowid / 秒级时间戳均 < 2^53），保持现有 number 类型
        .dangerously_cast_bigints_to_number()
        // Throw 模式：命令失败直接 reject，与前端 invokeI18n 的异常翻译链路兼容
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .events(tauri_specta::collect_events![
            events::ReleaseStateChanged,
            events::PollCompleted,
            events::LogAppended,
            events::SourceAutoDisabled,
            events::Navigate,
            events::FocusRelease,
            events::AgentRunFinished,
            events::AgentRpcStream,
        ])
        .commands(collect_commands![
        commands::add_source,
        commands::list_source_types,
        commands::remove_source,
        commands::update_source,
        commands::list_sources,
        commands::get_release_catalog,
        commands::get_release_detail,
        commands::get_release_search_bodies,
        commands::get_release_search_bodies_by_ids,
        commands::set_notification_state,
        commands::set_release_flag,
        commands::delete_release,
        commands::translate_release,
        commands::clear_logs,
        commands::trigger_poll,
        commands::check_single_source,
        commands::get_settings,
        commands::update_settings,
        commands::get_poll_countdown,
        commands::set_credential,
        commands::read_bilibili_login_cookie,
        commands::close_bilibili_login_window,
        commands::open_bilibili_login_window,
        commands::test_deepseek_connection,
        commands::search_logs,
        commands::report_frontend_error,
        commands::export_backup,
        commands::import_backup,
        commands::hide_to_tray,
        commands::fetch_url_bytes,
        commands::set_clipboard_text,
        commands::set_clipboard_image,
        commands::record_usage,
        commands::get_usage_stats,
        commands::clear_usage_stats,
        commands::get_ai_usage_stats,
        commands::save_agent_config,
        commands::get_agent_config,
        commands::get_agent_ws_width,
        commands::save_agent_ws_width,
        commands::get_agent_available_models,
        commands::run_agent_job,
        commands::list_agent_runs,
        commands::get_agent_queue_status,
        commands::get_agent_queue,
        commands::get_agent_session_usage,
        commands::list_agent_messages,
        commands::list_agent_sessions,
        commands::cancel_agent_run,
        commands::delete_agent_session,
        commands::get_agent_session_command,
        commands::open_agent_session,
        commands::get_agent_rpc_status,
        commands::restart_agent_rpc,
        commands::export_agent_session,
        commands::agent_shutdown_for_update,
        commands::updater_check,
        commands::updater_download_started,
        commands::updater_download_failed,
        commands::updater_install_started,
    ])
}

/// 导出 TS 绑定到前端 src/bindings.ts（debug 构建运行时 + CI 测试两处触发）。
#[cfg(debug_assertions)]
pub fn export_bindings() {
    specta_builder()
        .export(
            specta_typescript::Typescript::default().header("/* eslint-disable */"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts"),
        )
        .expect("Failed to export typescript bindings");
}

/// 声明进程的 AppUserModelID（AUMID），使 Windows 通知归属到 RelWatch 而非 PowerShell。
///
/// 必须在**创建任何窗口之前**调用（此处位于 `tauri::Builder` 构建之前），否则任务栏
/// 分组与 toast 归属可能不生效。AUMID 值由 NSIS 安装器写入快捷方式
/// （`nsis/utils.nsh` 的 `SetLnkAppUserModelId`），与 `notify::WINDOWS_AUMID` 同源。
/// 失败时仅告警：退回当前行为（通知仍会显示，只是来源名可能不正确）。
#[cfg(windows)]
fn declare_app_user_model_id() {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::PCWSTR;

    let aumid: Vec<u16> = format!("{}\0", notify::WINDOWS_AUMID).encode_utf16().collect();
    // 此刻日志插件尚未注册（setup 中才挂载），log 宏是空操作，故用 eprintln
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(aumid.as_ptr())) } {
        eprintln!("WARNING: 设置 AppUserModelID 失败，通知来源可能显示不正确: {}", e);
    }
}

/// 退出时留给 pi RPC 常驻进程优雅收尾的时间预算。
///
/// 取 3 秒的依据：正常路径下 `RpcProcess::shutdown` 只需「关 stdin + 等 500ms」，
/// 远小于该值，因而不会拖慢退出；异常路径下它就是主线程被阻塞的**硬上限**。
const EXIT_SHUTDOWN_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// 有界等待一个清理 future，返回它是否在 `budget` 内完成。
///
/// **为什么不用 `tokio::time::timeout` 兜底**：它同样依赖 tokio 的 timer driver，
/// 而这里要防的恰恰是「tokio 内部不推进」这类悬挂 —— 那种情况下 timeout 自身也不会
/// 到期，等于没兜住。改走 `std::sync::mpsc::recv_timeout`（基于 OS 同步原语，与 tokio
/// 内部状态完全无关），才是真上限。
///
/// **为什么把清理 future 丢给 `async_runtime::spawn` 而不是 `block_on`**：`block_on`
/// 会占住当前线程直到 future 完成，调用方（退出路径上的 Tauri 主线程）就再没有被
/// 第三方收回控制权的机会 —— 那正是「点了退出之后永久卡死」的成因。交给线程池后，
/// 取消权始终留在调用方手里。
fn await_bounded<F>(cleanup: F, budget: std::time::Duration) -> bool
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        cleanup.await;
        // 发送失败 = 调用方已超时离开；这是预期路径之一，无需处理
        let _ = tx.send(());
    });
    rx.recv_timeout(budget).is_ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 先声明 AUMID 再建窗口（Windows 要求在任何 UI 创建前设置）
    #[cfg(windows)]
    declare_app_user_model_id();

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
    // Agent 子进程并发上限：RpcManager 是「单常驻进程」模型——
    // 1) ensure_session 与 prompt 是两次独立加锁操作，并发提交会互相切走会话（A 切完 B 切走，A 的 prompt 落进 B 的会话文件）；
    // 2) 事件流是全局 broadcast 且不带 run 标识，并行 run 会互收对方的 delta/settled/agent_end，串流且可能误判终态。
    // 因此并发上限必须为 1（多个会话的提交排队串行执行）；如需并行，中期方案是事件按 run_id 打标或一会话一进程。
    let agent_semaphore = Arc::new(tokio::sync::Semaphore::new(1));

    // 开发/测试构建时把最新 TS 绑定写入前端（CI 亦可通过 cargo test 触发）
    #[cfg(debug_assertions)]
    export_bindings();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                focus::show_and_focus(&window);
            }
        }))
        // 应用内更新（tauri.conf.json plugins.updater 配置 endpoint/pubkey）与 relaunch
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
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
            agent_rpc: std::sync::Arc::new(crate::agent_rpc::RpcManager::new(pool.clone())),
            agent_cancelled: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            db: pool,
            next_poll_at: next_poll.clone(),
            deepseek_semaphore,
            agent_semaphore,
        })
        // 命令清单单一来源：invoke_handler 从同一个 specta Builder 生成，
        // 与 collect_commands! 共用一份清单，不再存在第二份手工副本。
        .invoke_handler(specta_builder().invoke_handler())
        // media 图片网关：前端把远程图片改写为 http://media.localhost/<url>，
        // 此处拦截并用已按 ProxyPolicy 构建的 reqwest client 下载返回（继承应用代理）。
        // 注册必须在使用前完成；handler 闭包在每次请求时触发，经 UriSchemeContext
        // 取 app handle（读 DB 代理设置），下载在独立线程执行后经 responder 异步返回，
        // 避免阻塞 WebView 请求线程。
        .register_asynchronous_uri_scheme_protocol("media", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let uri = request.uri().to_string();
            std::thread::spawn(move || {
                let app = app.clone();
                let response = tauri::async_runtime::block_on(async move {
                    // 提取 path：http://media.localhost/<path> → <path>
                    let path = uri
                        .split("localhost/")
                        .nth(1)
                        .map(|p| p.to_string())
                        .unwrap_or_default();
                    crate::media::handle_media_request(&app, &path).await
                });
                responder.respond(response);
            });
        })
        .setup(|app| {
            // 主窗口改为 Rust 建窗（原 tauri.conf.json 静态配置逐项迁至此）：
            // on_navigation 导航守卫只能在 builder 上挂载。这是 M-1 的纵深防御——
            // 主 webview 的五重防线（useExternalLinkGuard / DOMPurify / CSP /
            // wry 新窗拒绝 / 禁拖放）全在前端层，这道 Rust 层守卫把「任意内容
            // 成为主窗口文档」从根源排除：只放行自家 origin，http(s) 外链交给
            // 系统浏览器（与前端 useExternalLinkGuard 行为一致），其余（media
            // 等自定义协议域、file: 等）一律拦截并记日志。
            let app_handle = app.handle().clone();
            // dev server 端口取自 build.devUrl（生产构建无 devUrl，为 None）
            let dev_port = app.config().build.dev_url.as_ref().and_then(|u| u.port());
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("RelWatch")
            .inner_size(750.0, 750.0)
            .min_inner_size(710.0, 450.0)
            .resizable(true)
            .center()
            .disable_drag_drop_handler()
            .on_navigation(move |url| {
                let scheme = url.scheme();
                let host = url.host_str().unwrap_or_default();
                // 自家 origin：tauri://localhost（macOS/Linux 生产）、
                // http://tauri.localhost（Windows 生产）、devUrl（开发）
                let is_own = (scheme == "tauri" && host == "localhost")
                    || (scheme == "http" && host == "tauri.localhost" && url.port().is_none())
                    || (scheme == "http"
                        && host == "localhost"
                        && dev_port.is_some()
                        && url.port() == dev_port);
                if is_own {
                    return true;
                }
                if (scheme == "http" || scheme == "https") && !host.ends_with(".localhost") {
                    // 外链：系统浏览器打开（*.localhost 是 media 等自定义协议域，
                    // 不外开、直接拦）
                    use tauri_plugin_opener::OpenerExt;
                    if let Err(e) =
                        app_handle.opener().open_url(url.as_str(), None::<&str>)
                    {
                        log::warn!("外链交系统浏览器失败: {} ({})", url, e);
                    }
                } else {
                    log::warn!("已拦截主窗口的非自家 origin 导航: {}", url);
                }
                false
            })
            .build()?;

            // 注册事件名映射（release 构建的 emit 同样依赖），必须在 emit 之前挂载
            specta_builder().mount_events(app);

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
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // 应用退出：优雅关闭 pi RPC 常驻进程（关 stdin → pi 自身清理子进程）。
            //
            // 这里的等待**必须有界**。本回调跑在 Tauri 事件循环所在的**主线程**上，
            // 它返回之后才轮到 `cleanup_before_exit()`（隐窗口 / 清托盘图标）与
            // `process::exit`。若在此无限阻塞，主线程的消息循环就停摆：窗口与托盘
            // 全部无响应、进程也不会退出 —— 用户看到的就是「点了托盘退出之后卡死，
            // 只能去任务管理器强杀」（1.17.2 / Windows 11 实测发生过：长休眠唤醒后
            // 点退出，`Responding=False` 持续悬挂，WER 记 AppHangTransient）。
            if let tauri::RunEvent::Exit = event {
                let rpc = app_handle.state::<AppState>().agent_rpc.clone();
                let closed = await_bounded(async move { rpc.shutdown().await }, EXIT_SHUTDOWN_BUDGET);
                if !closed {
                    // 放弃等待是安全兜底，不需要重试也不该 panic：
                    // Windows 侧 pi 由 JobObject 兜着（ProcessScope 的
                    // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE —— 本进程退出、句柄关闭时
                    // 内核终止整棵作业树）；Unix 侧 pi 以新进程组首领启动，
                    // kill_process_tree 同样不依赖本函数跑完。最坏后果只是 pi 少了
                    // 一个优雅收尾窗口，不会有孤儿进程残留。
                    //
                    // 留痕走降级文件而不是 DB：退出路径不该为了记一行日志去同步取
                    // DB 连接（那正是本次要消除的阻塞面，且此刻进程即将终止）。
                    // 持久化记录是必要的 —— release 版无控制台、tauri_plugin_log 也
                    // 未挂载（见 setup 的 debug_assertions 分支），`eprintln!` 只对
                    // dev 构建可见，不落文件就什么都留不下。
                    eprintln!(
                        "WARNING: pi RPC 进程未在 {:?} 内结束，放弃等待并继续退出（子进程由内核兜底回收）",
                        EXIT_SHUTDOWN_BUDGET
                    );
                    crate::db::logs::write_fallback_only(
                        "WARN",
                        "agent.exit_shutdown_timeout",
                        &format!("{{\"budget_secs\":{}}}", EXIT_SHUTDOWN_BUDGET.as_secs()),
                    );
                }
            }
        });
}
#[cfg(test)]
mod tests {
    /// 触发 TS 绑定导出：`cargo test` 即重新生成 src/bindings.ts，CI 可据此检查同步。
    #[test]
    #[cfg(debug_assertions)]
    fn export_typescript_bindings() {
        super::export_bindings();
    }

    /// 清理在预算内完成时立即返回 true，不空等到预算用尽。
    #[test]
    fn await_bounded_returns_true_when_cleanup_finishes() {
        let budget = std::time::Duration::from_secs(10);
        let t0 = std::time::Instant::now();
        let finished = super::await_bounded(async {}, budget);
        let elapsed = t0.elapsed();
        assert!(finished, "清理立即完成时应报告已结束");
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "不应等到 {budget:?} 预算耗尽，实际 {elapsed:?}"
        );
    }

    /// 清理永不完成时必须在预算处放弃（返回 false），而不是把调用方永久挂住
    /// —— 这是「点退出后卡死」的回归护栏：把实现换回 `block_on` 本用例会挂死。
    #[test]
    fn await_bounded_gives_up_at_budget() {
        let budget = std::time::Duration::from_millis(300);
        let t0 = std::time::Instant::now();
        let finished = super::await_bounded(std::future::pending(), budget);
        let elapsed = t0.elapsed();
        assert!(!finished, "清理卡住时不应报告已结束");
        assert!(elapsed >= budget, "应至少等满预算，实际 {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "放弃等待必须及时（硬上限），实际 {elapsed:?}"
        );
    }
}


