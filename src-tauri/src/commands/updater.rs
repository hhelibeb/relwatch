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
use tauri::{Manager, ResourceId, Webview};
use tauri_plugin_updater::UpdaterExt;

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

/// 检查应用更新。返回 `None` 表示当前已是最新版本（endpoint 返回 204，或远端版本不高于当前版本）。
#[tauri::command]
#[specta::specta]
pub async fn updater_check(
    webview: Webview,
    timeout_ms: u64,
    proxy_mode: String,
    proxy_url: String,
) -> Result<Option<UpdaterMetadata>, String> {
    let mut builder = webview
        .updater_builder()
        .timeout(Duration::from_millis(timeout_ms));
    match resolve_proxy(&proxy_mode, &proxy_url)? {
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
}
