//! B 站一键登录：从登录 WebView 读取 SESSDATA、验证登录态并加密存储。
//!
//! 流程：设置页「一键登录」→ 前端请求本模块命令用 Rust 建窗（WebviewWindowBuilder）
//! 加载 B 站登录页 → 用户扫码/账号登录 → 前端轮询本模块命令 → 读到有效 SESSDATA
//! 后加密存库 → 前端关闭登录窗口。
//!
//! 为什么由 Rust 建窗而不是前端 `new WebviewWindow()`：登录窗口加载整个远程站点，
//! 其网络请求也应继承应用代理设置；Tauri 2 的 JS API `WindowOptions` 没有 proxy
//! 字段，只有 Rust 侧 `WebviewWindowBuilder::proxy_url()` 能注入。代理语义收敛在
//! `net::ProxyPolicy`（custom → 注入自定义代理；system/none → 交给平台默认）。
//!
//! 不读取系统浏览器的 cookie（Chrome/Edge cookie 为 DPAPI+AES-GCM 加密且新版
//! 加 app-bound encryption，自动导出脆弱且有安全争议），改为应用内 WebView 登录，
//! SESSDATA 只存在于应用自己的 webview cookie 存储中，登录后即加密入库。

use std::str::FromStr;

use tauri::Manager;
use tauri::{WebviewUrl, WebviewWindowBuilder};

use crate::crypto;
use crate::db::settings::{self, KEY_BILIBILI_COOKIE, KEY_PROXY_URL, KEY_PROXY_MODE};
use crate::net::ProxyPolicy;
use crate::types::AppState;

/// 读取 cookie 的匹配域（SESSDATA 域为 .bilibili.com，www 子域可读）。
const BILI_COOKIE_DOMAIN: &str = "https://www.bilibili.com";
/// 验证登录态用（isLogin 字段）。
const BILI_NAV_URL: &str = "https://api.bilibili.com/x/web-interface/nav";
/// B 站对非浏览器 UA 敏感，单请求覆盖 client 级 UA。
const BILI_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
/// 登录页地址（Rust 建窗加载）。
const BILI_LOGIN_URL: &str = "https://passport.bilibili.com/login";
/// 登录窗口尺寸（与前端旧建窗参数一致）。
const BILI_LOGIN_WIDTH: f64 = 460.0;
const BILI_LOGIN_HEIGHT: f64 = 640.0;

/// 登录窗口固定 label（capabilities/bilibili-login.json 的 windows 白名单也依赖此值；
/// 改动时两处必须同步）。
pub const BILI_LOGIN_WINDOW_LABEL: &str = "bilibili-login";

/// 登录窗口专属 WebView2 用户数据目录名（相对 `AppLocalData`）。
///
/// **不可与主窗口共用**：Windows WebView2 要求「同一 user data folder 上运行中的
/// 实例，其 `CoreWebView2EnvironmentOptions` 必须一致」，而登录窗口会按代理设置
/// 注入 `--proxy-server`（主窗口不注入）——选项不同却共用目录会令 WebView 创建
/// 失败；该失败在 tauri-runtime-wry 里只 `log::error!`、**不回传**，`build()` 仍
/// 返回 Ok，表现为「窗口闪一下即消失且按钮锁死」。改动时请保留该隔离。
pub const BILI_LOGIN_DATA_DIR: &str = "bilibili-login";

/// 校验调用方传入的 window_label 必须是登录窗口自身。
///
/// 命令只面向登录窗口：`read_bilibili_login_cookie` 会读取 webview cookie、
/// `close_bilibili_login_window` 会关闭窗口，若接受任意 label，本地内容一旦被
/// 注入即可关任意窗口/读任意窗口 cookie（放大面）。这里在命令层做硬校验，
/// 与 capability 白名单构成纵深防御。
fn require_login_window_label(window_label: &str) -> Result<(), String> {
    if window_label != BILI_LOGIN_WINDOW_LABEL {
        log::warn!(
            "拒绝非登录窗口 label 的 B 站登录命令: {:?}（仅允许 {}",
            window_label,
            BILI_LOGIN_WINDOW_LABEL
        );
        return Err("err.bili_login_window_missing".to_string());
    }
    Ok(())
}

/// 从登录 WebView 读取 SESSDATA，验证有效后加密存储。
///
/// - `Ok(true)`：读取并保存成功（前端应关闭登录窗口）
/// - `Err(err.bili_login_not_logged_in)`：窗口在但尚未登录（前端继续轮询）
/// - `Err(err.bili_login_window_missing)`：窗口已关闭（前端停止轮询）
/// - 其它 Err：读取/验证失败（前端提示）
#[tauri::command]

#[specta::specta]pub async fn read_bilibili_login_cookie(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    window_label: String,
) -> Result<bool, String> {
    require_login_window_label(&window_label)?;
    let win = app
        .get_webview_window(&window_label)
        .ok_or("err.bili_login_window_missing".to_string())?;

    // 1) 读 webview cookie（同步 API）
    let url = tauri::Url::from_str(BILI_COOKIE_DOMAIN)
        .map_err(|e| format!("err.parse_failed|{}", e))?;
    let cookies = win
        .cookies_for_url(url)
        .map_err(|e| format!("err.bili_login_cookie_read|{}", e))?;
    let sessdata = cookies
        .iter()
        .find(|c| c.name() == "SESSDATA")
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty())
        .ok_or("err.bili_login_not_logged_in".to_string())?;

    // 2) 验证登录态（nav isLogin），避免无效/过期 cookie 入库
    //    先从 settings 取代理配置构建 client（与轮询链路一致）
    let (proxy_url, proxy_mode) = {
        let conn = state
            .db
            .get()
            .map_err(|e| format!("err.db_lock|{}", e))?;
        let pu = settings::get_setting(&conn, KEY_PROXY_URL)
            .ok()
            .flatten()
            .unwrap_or_default();
        let pm = settings::get_setting(&conn, KEY_PROXY_MODE)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if pu.is_empty() {
                    "none".to_string()
                } else {
                    "custom".to_string()
                }
            });
        (pu, pm)
    };
    let client = crate::http::build_http_client(crate::http::HttpClientConfig {
        proxy_url: &proxy_url,
        proxy_mode: &proxy_mode,
        bearer_token: None,
        ..Default::default()
    })?;
    let nav = client
        .get(BILI_NAV_URL)
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Cookie", format!("SESSDATA={}", sessdata))
        .send()
        .await
        .map_err(|e| crate::http::describe_request_error(&e))?;
    let body: serde_json::Value =
        crate::http::read_json_limited(nav, crate::http::MAX_JSON_BYTES).await?;
    if body["data"]["isLogin"].as_bool() != Some(true) {
        return Err("err.bili_login_not_logged_in".to_string());
    }

    // 3) 加密存储
    let conn = state
        .db
        .get()
        .map_err(|e| format!("err.db_lock|{}", e))?;
    settings::set_setting(&conn, KEY_BILIBILI_COOKIE, &crypto::encrypt(&sessdata))?;
    crate::db::logs::write_log_key(
        &conn,
        "INFO",
        "setting.bilibili_cookie_updated",
        &serde_json::json!({"source": "webview-login"}).to_string(),
    );
    Ok(true)
}

/// 用 Rust 建窗打开 B 站登录窗口（可注入应用代理设置）。
///
/// 为什么不用前端 `new WebviewWindow`：JS 的 `WindowOptions` 没有 proxy 字段，
/// 登录窗口加载整个远程站点时网络请求只能走系统代理。此命令在 Tauri 侧建窗，
/// 按 `net::ProxyPolicy` 决定是否注入 `proxy_url`——
/// `custom` + 非空 url → 注入（登录页/后续请求走应用自定义代理）；
/// `system` / `none` / 空 url → 不注入（交由平台默认 = 系统代理）。
/// 若窗口已存在（如轮询期间用户重复点击）则静默成功（幂等）。
///
/// 必须是 `async` 命令：Tauri 对**同步**命令在调用线程（= 主线程/事件循环线程）
/// 直接执行函数体，而 Windows 下 `WebviewWindowBuilder::build()` 经
/// `WaitWithPump` 等待 WebView2 的异步环境/控制器回调——回调依赖主线程消息泵，
/// 主线程被占死等回调即自我死锁（wry#583，Tauri 文档对 `WebviewWindowBuilder`
/// 与 `Webview::cookies` 均有明确警告）。表现为点击后弹出一个空窗口骨架且完全
/// 无法关闭：空窗口是 tao 在事件循环中建好的 HWND（标题/尺寸已生效），webview
/// 与 WebView2 环境仍卡在等待中；关不掉是因为 WM_CLOSE 只投递 `CloseRequested`
/// 到事件循环，而事件循环正卡在 `WaitWithPump` 的死循环里（tao 的 WM_CLOSE
/// → 事件循环处理，事件循环不动则窗口无法销毁）。改为 async 后命令体在
/// async runtime（非主线程）执行，主线程消息泵保持可用。
/// 该缺陷自 c8d28f8「B 站登录窗 Rust 建窗」（v1.16.0 起）引入。
#[tauri::command]

#[specta::specta]pub async fn open_bilibili_login_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    title: String,
) -> Result<(), String> {
    if app.get_webview_window(BILI_LOGIN_WINDOW_LABEL).is_some() {
        return Ok(()); // 已存在：幂等复用
    }

    let login_url: tauri::Url = BILI_LOGIN_URL
        .parse()
        .map_err(|_| "err.invalid_url".to_string())?;

    // 读代理设置决定是否注入 proxy_url（与后端轮询 client 的语义一致）。
    // 仅 custom + 有效 URL 注入；none / system / 解析失败不注入（交由平台默认）。
    // 在这里（async 上下文、建窗线程之前）读 DB 并释放连接，避免把非 Send 的
    // 连接/锁跨线程带入下面的 spawn_blocking。
    let proxy = {
        let conn = state.db.get().map_err(|e| format!("err.db_lock|{}", e))?;
        let proxy_url = settings::get_setting(&conn, KEY_PROXY_URL)
            .ok()
            .flatten()
            .unwrap_or_default();
        let proxy_mode = settings::get_setting(&conn, KEY_PROXY_MODE)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if proxy_url.is_empty() {
                    "none".to_string()
                } else {
                    "custom".to_string()
                }
            });
        match ProxyPolicy::resolve(&proxy_mode, &proxy_url) {
            Ok(crate::net::ProxyDecision::Proxy(url)) => Some(url),
            _ => None,
        }
    };

    // 建窗放到 `spawn_blocking` 的独立线程：`build()` 内部用 `WaitWithPump` 泵
    // 消息等待 WebView2 回调，若在主线程上执行即自我死锁（回调等主线程泵消息，
    // 而主线程正在等回调）；放在 tokio worker 上同样不合适——`GetMessageA`
    // 会把一个 reactor worker 卡在阻塞等消息上。blocking 线程池本就是为这类
    // 阻塞任务准备的（一个任务独占一个线程，消息泵不被其他任务干扰），也与
    // Tauri 文档「use async commands **and separate threads**」一致。
    //
    // builder 必须在闭包内部从 `AppHandle` 构建：`WebviewWindowBuilder` 借用
    // `&'a Manager`，拿不到 `'static` 生命周期，无法先建后移入线程。
    let app_for_build = app.clone();
    // 独立 WebView2 用户数据目录：Windows 下 WebView2 要求「同一 user data
    // folder 上运行中的实例，其 CoreWebView2EnvironmentOptions 必须一致」
    // （wry::WebContext 文档 + MS 文档同口径）。主窗口无代理、本窗口按设置可能
    // 注入 `--proxy-server`，两者选项不同却共用 Tauri 默认填的
    // `%LOCALAPPDATA%\<identifier>`，会导致 WebView 创建失败——而失败在
    // tauri-runtime-wry 里只 `log::error!` 且**不回传**，build() 仍返回 Ok，
    // 结果是窗口闪一下即消失（create_webview 失败令局部 window 被 drop）、
    // 命令却报成功，前端转入轮询把按钮锁死。给登录窗口单独一个子目录即可避开。
    // 用 AppLocalData + 子目录（而非 LocalData）以保持与 manager 自动填的
    // `%LOCALAPPDATA%\<identifier>` 同根。
    let login_data_dir = app_for_build
        .path()
        .resolve(BILI_LOGIN_DATA_DIR, tauri::path::BaseDirectory::AppLocalData)
        .map_err(|e| format!("err.bili_login_window_create|{}", e))?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut builder = WebviewWindowBuilder::new(
            &app_for_build,
            BILI_LOGIN_WINDOW_LABEL,
            WebviewUrl::External(login_url),
        )
        .data_directory(login_data_dir)
        .title(title)
        .inner_size(BILI_LOGIN_WIDTH, BILI_LOGIN_HEIGHT)
        .center()
        .resizable(false)
        // 顶层导航白名单（M-1 加固）：该窗口加载第三方远程站点，且不具备主窗口的
        // 五重防线（useExternalLinkGuard / DOMPurify / CSP / wry 新窗拒绝 / 禁拖放），
        // 远程页面可自行 `location.href` 导航到 `http://media.localhost/...`——media
        // 是 app 级注册的协议、被 Tauri 判定为本地源，一旦导航成功，窗口就变成
        // 「本地 origin + 继承全部 IPC 能力」的文档（审计报告 §M-1 环节 5）。
        // 只放行 B 站系域（hdslb.com 为 B 站 CDN）；media 协议与其余一律拦截。
        .on_navigation(|url| {
            let host = url.host_str().unwrap_or_default();
            let ok = host == "bilibili.com"
                || host.ends_with(".bilibili.com")
                || host == "hdslb.com"
                || host.ends_with(".hdslb.com");
            if !ok {
                log::warn!("已拦截 B 站登录窗口的非白名单导航: {}", url);
            }
            ok
        });
        if let Some(url) = proxy {
            builder = builder.proxy_url(url);
        }
        builder
            .build()
            .map(|_| ())
            .map_err(|e| format!("err.bili_login_window_create|{}", e))
    })
    .await
    // 线程内 panic 也走这里（JoinError），命令不会永久挂起
    .map_err(|e| format!("err.bili_login_window_create|{}", e))?
}

/// 关闭登录窗口（前端在登录成功或用户放弃时调用）。
#[tauri::command]

#[specta::specta]pub fn close_bilibili_login_window(
    app: tauri::AppHandle,
    window_label: String,
) -> Result<(), String> {
    require_login_window_label(&window_label)?;
    if let Some(win) = app.get_webview_window(&window_label) {
        let _ = win.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_login_window_label_accepts_own_label() {
        assert_eq!(require_login_window_label(BILI_LOGIN_WINDOW_LABEL), Ok(()));
    }

    #[test]
    fn test_require_login_window_label_rejects_other_labels() {
        for label in ["main", "", "bilibili-login-2", "Bilibili-Login"] {
            let err = require_login_window_label(label).unwrap_err();
            assert_eq!(err, "err.bili_login_window_missing", "label={:?}", label);
        }
    }

    #[test]
    fn test_require_login_window_label_constant_is_stable() {
        // capabilities/bilibili-login.json 的 windows 白名单依赖此值，改动需两处同步
        assert_eq!(BILI_LOGIN_WINDOW_LABEL, "bilibili-login");
    }

    #[test]
    fn test_login_data_dir_isolated_from_main_window() {
        // WebView2 要求同 data_directory 下的环境选项一致；登录窗口会注入
        // --proxy-server 而主窗口不会，故必须用独立子目录（否则 WebView 创建失败
        // → 窗口闪退且 build() 仍返回 Ok）。
        assert_eq!(BILI_LOGIN_DATA_DIR, "bilibili-login");
        // 必须是一个不同层次的单一相对目录名：不是空串、不是 "."/".."、
        // 不含路径分隔符——否则会落到主窗口的默认目录（%LOCALAPPDATA%\<identifier>）
        // 或逃逸出 AppLocalData，失去隔离。
        let p = std::path::Path::new(BILI_LOGIN_DATA_DIR);
        assert_eq!(p.components().count(), 1, "data dir 应为单一目录名");
        assert!(!BILI_LOGIN_DATA_DIR.is_empty());
        assert_ne!(BILI_LOGIN_DATA_DIR, ".");
        assert_ne!(BILI_LOGIN_DATA_DIR, "..");
    }
}
