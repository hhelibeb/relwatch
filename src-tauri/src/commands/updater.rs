//! 应用内检查更新：自建 `updater_check` 命令，让代理语义与后台监控保持一致。
//!
//! ## 为什么不用插件自带的 `plugin:updater|check`
//!
//! 插件 JS API 的 `CheckOptions`（2.10.1 `dist-js/index.d.ts`）只有 `proxy`，
//! 没有任何字段能表达「强制直连」；Rust 侧 `commands::check` 的签名
//! （`headers/timeout/proxy/target/allow_downgrades`）同样没有对应参数。
//!
//! 而插件内部 `Updater::check` 用 `reqwest::ClientBuilder::new()` 建客户端，
//! reqwest 0.13.4（插件的实际依赖）默认 `auto_sys_proxy: true`
//! （`async_impl/client.rs:309`），在 `proxy` 为 `None` 时会追加
//! `ProxyMatcher::system()`（读 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` 等
//! 环境变量 + 各平台的 OS 代理设置）。
//!
//! 结论：`proxy_mode = "none"` 在「检查更新」这条链路上静默失效，与后台监控
//! （`http.rs::build_http_client` 对 none 显式调 `no_proxy()`）语义相反——
//! 用户因为系统/环境变量代理坏了才选「不使用代理」，结果监控通了、
//! 检查更新仍走坏代理，且无从绕开。
//!
//! 本命令照插件 `commands::check` 抄一份，按 relwatch 的 proxy_mode 三态显式映射
//! `no_proxy()` / `proxy()` / 系统代理，使两条链路语义对齐。
//!
//! 代理决策与 `http.rs::build_http_client` 逐分支对齐（见 `resolve_proxy`）：
//! `none` → 直连；`custom` + 非空 url → 显式代理；`custom` + 空 url → 直连
//! （http.rs 同此处理，避免落入系统代理）；`system` / 未知值 → 系统代理。
//!
//! 返回结构与插件内部的 `commands::Metadata` 同构（camelCase），前端用
//! `new Update(meta)` 构造资源对象；后续 `download` / `install` 仍走插件自带命令——
//! 它们按 rid 从 **webview** 资源表取 `Update`，并复用 check 时写入的
//! `proxy` / `no_proxy`。因此这里必须把 `Update` 放进 webview 资源表：
//! 换成 AppHandle（app 级资源表）会导致插件的 download 命令取不到资源。

use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::{Manager, ResourceId, State, Webview};
use tauri_plugin_updater::UpdaterExt;

use crate::db;
use crate::types::AppState;

/// 检查更新的返回结构，字段与 tauri-plugin-updater 内部的 `commands::Metadata` 一致
/// （含 camelCase 重命名），保证前端 `new Update(meta)` 直接可用。
///
/// `raw_json` 走字符串传递：specta rc.25 对 `serde_json::Value` 的 Type 实现是
/// 递归内联的枚举（Array/Object 又引用回 Value），导出 TS 时会触发 inline cycle panic。
/// 前端 `JSON.parse` 还原即可（latest.json 很小，开销可忽略）。
#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterMetadata {
    pub rid: ResourceId,
    pub current_version: String,
    pub version: String,
    pub date: Option<String>,
    pub body: Option<String>,
    pub raw_json: String,
}

/// 代理决策：返回 `ProxyDecision`（由 `updater_check` 中的 match 落到 UpdaterBuilder 上）。
///
/// 与 `http.rs::build_http_client` 逐分支对齐：
/// - `none` → `NoProxy`（真正绕过系统与环境变量代理）；
/// - `custom` + 非空 url → `Proxy(url)`（同时会关掉系统代理探测）；
/// - `custom` + 空 url → `NoProxy`（与 http.rs 一致，不落入系统代理）；
/// - `system` / 未知值 → `System`（不设置任何 proxy，由 reqwest 追加系统代理）。
fn resolve_proxy(proxy_mode: &str, proxy_url: &str) -> Result<ProxyDecision, String> {
    let url = proxy_url.trim();
    match proxy_mode {
        "none" => Ok(ProxyDecision::NoProxy),
        "custom" if url.is_empty() => Ok(ProxyDecision::NoProxy),
        "custom" => Ok(ProxyDecision::Proxy(
            reqwest::Url::parse(url).map_err(|_| "err.invalid_url".to_string())?,
        )),
        _ => Ok(ProxyDecision::System),
    }
}

#[derive(Debug)]
enum ProxyDecision {
    NoProxy,
    Proxy(reqwest::Url),
    System,
}

/// 抹去文本中 URL 的 userinfo：`scheme://user:pass@host` → `scheme://***:***@host`。
///
/// 更新链路的代理 URL 由用户自填，可能带凭据（`http://user:pass@host:port`）；
/// reqwest 的错误文本在部分场景会回显完整 URL，若不处理凭据会以明文落库，
/// 并随日志搜索展示、备份导出一起外泄。非 URL 文本（如 `connection reset by peer`）原样返回。
///
/// 按 RFC 3986 定位：authority 为 `://` 之后到首个 `/` `?` `#` 之间的片段，
/// 其中最后一个 `@` 之前即 userinfo（host 不允许含 `@`）。
///
/// authority 边界只取 `/` `?` `#` 与空白：**不**把 `)` `,` `'` 等当作边界。
/// 这些字符在 RFC 3986 里属于 sub-delims，可以合法出现在 userinfo 中；若拿它们切分，
/// 一段含 `)` 的口令会被截断成「authority 无 @ → 原样输出」，反而把凭据留在明文里。
/// 安全优先：多切不如少切，宁可让 `)` 留在 host 侧（形如 `***:***@host:8080)`），
/// 也绝不让 userinfo 片段逃过脱敏。
fn redact_url_credentials(text: &str) -> String {
    const SCHEME_SEP: &str = "://";
    /// authority 终止符：路径/查询/片段起始，或空白（URL 嵌在散文里时由空白断词）
    fn is_authority_end(c: char) -> bool {
        matches!(c, '/' | '?' | '#') || c.is_ascii_whitespace()
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(sep_pos) = rest.find(SCHEME_SEP) {
        let authority_start = sep_pos + SCHEME_SEP.len();
        out.push_str(&rest[..authority_start]);
        let after = &rest[authority_start..];
        let authority_end = after.find(is_authority_end).unwrap_or(after.len());
        let (authority, tail) = after.split_at(authority_end);
        match authority.rfind('@') {
            // at > 0 排除空 userinfo（`http://@host` 无需脱敏）
            Some(at) if at > 0 => {
                out.push_str("***:***@");
                out.push_str(&authority[at + 1..]);
            }
            _ => out.push_str(authority),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// 一条更新操作日志的参数（尚未落库）。
///
/// 构造逻辑从各命令体中抽出为纯函数（见下方 `*_log_entry`），使单测能直接断言
/// level / message_key / args，而不必依赖 webview 与 updater 插件；命令体与测试
/// 共用同一份构造，避免两处漂移。
struct UpdateLogEntry {
    level: &'static str,
    key: &'static str,
    /// JSON 对象字符串，供 `crate::i18n::render` 填充占位符
    args: String,
}

impl UpdateLogEntry {
    fn new(level: &'static str, key: &'static str, args: String) -> Self {
        Self { level, key, args }
    }

    /// 落库。同步 SQLite I/O，调用方需放进 `spawn_blocking`。
    fn write(&self, conn: &rusqlite::Connection) {
        db::logs::write_log_key(conn, self.level, self.key, &self.args);
    }
}

/// 「检查更新」的日志参数：发现新版本 / 已是最新 → INFO，失败 → WARN。
///
/// 失败级别与下载失败一致取 WARN（而非 ERROR）：更新检查是用户可重试的非关键操作，
/// 网络抖动、代理不通都属预期内，不该与数据层故障混在同一级别。
fn check_log_entry(result: &Result<Option<UpdaterMetadata>, String>) -> UpdateLogEntry {
    match result {
        Ok(Some(meta)) => UpdateLogEntry::new(
            "INFO",
            "update.log.check_found",
            json!({ "version": &meta.version }).to_string(),
        ),
        Ok(None) => UpdateLogEntry::new("INFO", "update.log.check_success", "{}".to_string()),
        Err(e) => UpdateLogEntry::new(
            "WARN",
            "update.log.check_failed",
            json!({ "error": redact_url_credentials(e) }).to_string(),
        ),
    }
}

fn download_started_log_entry(version: &str) -> UpdateLogEntry {
    UpdateLogEntry::new(
        "INFO",
        "update.log.download_started",
        json!({ "version": version }).to_string(),
    )
}

/// 下载失败取 WARN，与 `check_log_entry` 的失败分支对齐（理由见其文档注释）。
fn download_failed_log_entry(version: &str, error: &str) -> UpdateLogEntry {
    UpdateLogEntry::new(
        "WARN",
        "update.log.download_failed",
        json!({ "version": version, "error": redact_url_credentials(error) }).to_string(),
    )
}

fn install_started_log_entry(version: &str) -> UpdateLogEntry {
    UpdateLogEntry::new(
        "INFO",
        "update.log.install_started",
        json!({ "version": version }).to_string(),
    )
}

/// 检查应用更新。返回 `None` 表示当前已是最新版本（endpoint 返回 204，或远端版本不高于当前版本）。
/// 检查结果写入操作日志（成功/发现新版本/失败均记录）。
#[tauri::command]
#[specta::specta]
pub async fn updater_check(
    webview: Webview,
    state: State<'_, AppState>,
    timeout_ms: u64,
    proxy_mode: String,
    proxy_url: String,
) -> Result<Option<UpdaterMetadata>, String> {
    let result = updater_check_impl(&webview, timeout_ms, &proxy_mode, &proxy_url).await;

    // 写操作日志：检查成功（发现/最新）与失败都记录；日志写入失败不阻塞更新链路。
    // 先在主线程把日志参数整理好（result 非 Copy），再 move 进 spawn_blocking 闭包。
    let entry = check_log_entry(&result);
    let pool = state.db.clone();
    let _log_result = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        entry.write(&conn);
        Ok::<_, String>(())
    })
    .await;

    result
}

/// updater_check 核心逻辑（不含日志），便于单测直接验证返回语义。
async fn updater_check_impl(
    webview: &Webview,
    timeout_ms: u64,
    proxy_mode: &str,
    proxy_url: &str,
) -> Result<Option<UpdaterMetadata>, String> {
    let mut builder = webview
        .updater_builder()
        .timeout(Duration::from_millis(timeout_ms));
    match resolve_proxy(proxy_mode, proxy_url)? {
        ProxyDecision::NoProxy => builder = builder.no_proxy(),
        ProxyDecision::Proxy(url) => builder = builder.proxy(url),
        ProxyDecision::System => {}
    }

    let Some(update) = builder
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };

    // Update.date 是 time 0.3 的 OffsetDateTime；项目统一用 chrono（time 仅随插件传递依赖引入），
    // 这里经 unix 时间戳转换，避免为格式化再直接依赖 time。
    // 用 `SecondsFormat::Secs + use_z` 输出 `Z` 后缀，与插件内部 `time::Rfc3339` 的格式一致。
    let date = update.date.and_then(|d| {
        chrono::DateTime::from_timestamp(d.unix_timestamp(), d.nanosecond())
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    });
    let current_version = update.current_version.clone();
    let version = update.version.clone();
    let body = update.body.clone();
    let raw_json = update.raw_json.to_string();

    Ok(Some(UpdaterMetadata {
        rid: webview.resources_table().add(update),
        current_version,
        version,
        date,
        body,
        raw_json,
    }))
}

/// 写「开始下载更新」操作日志。由前端在调用 `Update.downloadAndInstall()` 之前触发。
#[tauri::command]
#[specta::specta]
pub async fn updater_download_started(
    state: State<'_, AppState>,
    version: String,
) -> Result<(), String> {
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        download_started_log_entry(&version).write(&conn);
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("err.task_failed|updater_download_started|{}", e))?
}

/// 写「更新下载失败」操作日志。由前端在下载抛出异常时触发（error 分类文案已由前端生成）。
#[tauri::command]
#[specta::specta]
pub async fn updater_download_failed(
    state: State<'_, AppState>,
    version: String,
    error: String,
) -> Result<(), String> {
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        download_failed_log_entry(&version, &error).write(&conn);
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("err.task_failed|updater_download_failed|{}", e))?
}

/// 写「开始安装更新并重启」操作日志。仅 Linux/macOS 路径可到达（下载安装完成后、relaunch 前）。
#[tauri::command]
#[specta::specta]
pub async fn updater_install_started(
    state: State<'_, AppState>,
    version: String,
) -> Result<(), String> {
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        install_started_log_entry(&version).write(&conn);
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("err.task_failed|updater_install_started|{}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 断言 resolve_proxy 的决策类别，避免匹配到 reqwest::Url 内部字段。
    fn decision_kind(mode: &str, url: &str) -> &'static str {
        match resolve_proxy(mode, url).unwrap() {
            ProxyDecision::NoProxy => "no_proxy",
            ProxyDecision::Proxy(u) => {
                // reqwest::Url::parse 会规范化（裸 host 补尾部 `/`），用 host:port 断言
                assert_eq!(u.host_str(), reqwest::Url::parse(url.trim()).unwrap().host_str());
                "proxy"
            }
            ProxyDecision::System => "system",
        }
    }

    #[test]
    fn proxy_none_is_no_proxy() {
        assert_eq!(decision_kind("none", ""), "no_proxy");
        assert_eq!(decision_kind("none", "http://127.0.0.1:17890"), "no_proxy");
    }

    #[test]
    fn proxy_custom_with_url_is_explicit_proxy() {
        assert_eq!(decision_kind("custom", "http://127.0.0.1:17890"), "proxy");
        assert_eq!(decision_kind("custom", "  http://127.0.0.1:17890  "), "proxy");
    }

    #[test]
    fn proxy_custom_with_empty_url_is_no_proxy() {
        // 与 http.rs::build_http_client 对齐：custom + 空 url → 直连，不落入系统代理
        assert_eq!(decision_kind("custom", ""), "no_proxy");
        assert_eq!(decision_kind("custom", "   "), "no_proxy");
    }

    #[test]
    fn proxy_system_and_unknown_use_system() {
        assert_eq!(decision_kind("system", ""), "system");
        assert_eq!(decision_kind("system", "http://127.0.0.1:17890"), "system");
        assert_eq!(decision_kind("whatever", ""), "system");
    }

    #[test]
    fn proxy_custom_with_invalid_url_errors() {
        let err = resolve_proxy("custom", "not a url").unwrap_err();
        assert_eq!(err, "err.invalid_url");
    }

    // ── 操作日志：驱动命令体真正使用的 `*_log_entry` 构造函数 ──
    // 早期版本直接调 `write_log_key` 复述一遍命令内的参数，属于自证式测试：
    // 命令里 level/key 写错测试照样通过。改为与命令共用同一份构造后，
    // 断言才真正覆盖到命令行为（落库仍走内存库，端到端校验 rendered_message）。

    /// 落库一条日志并返回其 rendered_message（默认 locale）。
    fn render_log(entry: &UpdateLogEntry) -> String {
        let conn = crate::db::init::init_memory_db().unwrap();
        entry.write(&conn);
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        let row = logs
            .iter()
            .find(|l| l.message_key.as_deref() == Some(entry.key))
            .unwrap_or_else(|| panic!("log missing: {}", entry.key));
        assert_eq!(row.level, entry.level);
        row.rendered_message.clone().unwrap_or_default()
    }

    /// 取日志行的原始 message_args，用于断言脱敏后的入参。
    fn logged_args(entry: &UpdateLogEntry) -> String {
        let conn = crate::db::init::init_memory_db().unwrap();
        entry.write(&conn);
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        logs.iter()
            .find(|l| l.message_key.as_deref() == Some(entry.key))
            .and_then(|l| l.message_args.clone())
            .unwrap_or_default()
    }

    fn meta(version: &str) -> UpdaterMetadata {
        UpdaterMetadata {
            rid: 0,
            current_version: "1.13.0".into(),
            version: version.into(),
            date: None,
            body: None,
            raw_json: "{}".into(),
        }
    }

    #[test]
    fn check_found_log_contains_version() {
        let entry = check_log_entry(&Ok(Some(meta("1.14.0"))));
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.key, "update.log.check_found");
        assert!(render_log(&entry).contains("1.14.0"));
    }

    #[test]
    fn check_success_log_has_no_version_arg() {
        let entry = check_log_entry(&Ok(None));
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.key, "update.log.check_success");
        // 无占位符残留：rendered_message 不含 {version}
        let rendered = render_log(&entry);
        assert!(!rendered.contains("{version}"));
        assert!(!rendered.is_empty());
    }

    #[test]
    fn check_failed_log_is_warn_and_keeps_error() {
        let entry = check_log_entry(&Err("connection reset".to_string()));
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.key, "update.log.check_failed");
        assert!(render_log(&entry).contains("connection reset"));
    }

    #[test]
    fn download_started_log_contains_version() {
        let entry = download_started_log_entry("1.14.0");
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.key, "update.log.download_started");
        assert!(render_log(&entry).contains("1.14.0"));
    }

    #[test]
    fn download_failed_log_is_warn_and_keeps_error() {
        let entry = download_failed_log_entry("1.14.0", "connection reset");
        // 与检查失败同级：更新链路属可重试的非关键操作
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.key, "update.log.download_failed");
        assert!(render_log(&entry).contains("connection reset"));
    }

    #[test]
    fn install_started_log_contains_version() {
        let entry = install_started_log_entry("1.14.0");
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.key, "update.log.install_started");
        assert!(render_log(&entry).contains("1.14.0"));
    }

    // ── redact_url_credentials ──

    #[test]
    fn redact_strips_userinfo_from_url() {
        assert_eq!(
            redact_url_credentials("error sending request for url (http://user:pass@proxy.example.com:8080)"),
            "error sending request for url (http://***:***@proxy.example.com:8080)"
        );
    }

    #[test]
    fn redact_strips_userinfo_without_password() {
        assert_eq!(
            redact_url_credentials("http://alice@host/path"),
            "http://***:***@host/path"
        );
    }

    #[test]
    fn redact_handles_multiple_urls() {
        assert_eq!(
            redact_url_credentials("a http://u:p@h1 b https://u2:p2@h2/x?y=1 c"),
            "a http://***:***@h1 b https://***:***@h2/x?y=1 c"
        );
    }

    #[test]
    fn redact_handles_url_wrapped_in_parentheses() {
        // 回归：reqwest 的错误文本形如 `... for url (http://...)`。
        // 早期实现把 `)` 当 authority 边界，导致带 `)` 的口令被截断、凭据漏网。
        assert_eq!(
            redact_url_credentials("error sending request for url (http://u:p@h:8080)"),
            "error sending request for url (http://***:***@h:8080)"
        );
        // 口令自身含 `)`：必须从最后一个 `@` 切分，而非从 `)` 断词
        assert_eq!(
            redact_url_credentials("http://u:p)ss@h/x"),
            "http://***:***@h/x"
        );
    }

    #[test]
    fn redact_uses_last_at_as_userinfo_boundary() {
        // host 不允许含 `@`，故最后一个 `@` 即 userinfo 结尾
        assert_eq!(redact_url_credentials("http://a@b@c/d"), "http://***:***@c/d");
    }

    #[test]
    fn redact_leaves_plain_text_untouched() {
        // 普通错误文本不含 `://`，须原样保留
        for s in [
            "connection reset by peer",
            "err.invalid_url",
            "error sending request for url (https://objects.githubusercontent.com/...)",
        ] {
            assert_eq!(redact_url_credentials(s), s);
        }
    }

    #[test]
    fn redact_leaves_empty_userinfo_untouched() {
        // 空 userinfo 无需脱敏（at > 0 判断）
        assert_eq!(redact_url_credentials("http://@host"), "http://@host");
    }

    #[test]
    fn download_failed_log_redacts_proxy_credentials() {
        // 端到端：带凭据的代理 URL 不得以明文落库
        let entry = download_failed_log_entry(
            "1.14.0",
            "error sending request for url (http://user:secret@proxy.example.com:8080)",
        );
        let args = logged_args(&entry);
        assert!(!args.contains("secret"), "凭据泄露到日志: {args}");
        assert!(!args.contains("user:"), "用户名泄露到日志: {args}");
        assert!(args.contains("***:***@proxy.example.com:8080"));
    }

    #[test]
    fn check_failed_log_redacts_proxy_credentials() {
        let entry = check_log_entry(&Err(
            "proxy connect failed: http://admin:hunter2@10.0.0.1:3128".to_string(),
        ));
        let args = logged_args(&entry);
        assert!(!args.contains("hunter2"), "凭据泄露到日志: {args}");
        assert!(args.contains("***:***@10.0.0.1:3128"));
    }
}
