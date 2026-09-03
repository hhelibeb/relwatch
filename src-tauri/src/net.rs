//! 网络出口的统一代理策略：三态判定唯一来源。
//!
//! ## 为什么需要这个模块
//!
//! `proxy_mode` / `proxy_url` 的语义曾经散落多处：`http.rs::build_http_client`
//! 内联一份 match，`commands/updater.rs` 又复制一份 `resolve_proxy`（为绕开
//! tauri-updater 插件默认 `auto_sys_proxy` 的静默失效），两侧靠注释维持一致性。
//! 同一病因复发两次后，把「三态 → 决策」收敛到这里，新增网络出口一律消费
//! `ProxyPolicy::resolve`，从「需要人记住」变成「类型/接口上绕不开」。
//!
//! ## 三态语义（对系统代理/环境变量的统一约定）
//!
//! - `none` → `NoProxy`：显式直连，真正绕过系统代理与环境变量代理；
//! - `custom` + 非空 url → `Proxy(url)`：显式代理，同时关闭系统代理探测；
//! - `custom` + 空 url → `NoProxy`：不设代理即直连，避免落入系统代理；
//! - `system` / 未知值 → `System`：不设置任何 proxy，由 reqwest / 平台默认
//!   追加系统代理。
//!
//! 未知值归入 `System` 而非直接报错：数据库里若残留脏值，走系统代理比静默
//! 直连更接近用户预期（与 updater.rs 既有行为一致）。

/// 一次代理决策的类别化结果。
#[derive(Debug)]
pub enum ProxyDecision {
    NoProxy,
    Proxy(reqwest::Url),
    System,
}

/// 三态代理判定唯一来源。
pub struct ProxyPolicy;

impl ProxyPolicy {
    /// 把 (mode, url) 解析为决策。url 两端空白会被裁剪（用户手填常见前后缀空格）。
    ///
    /// `custom` 时 url 必须可解析且协议受支持（http/https/socks5），否则返回
    /// `err.invalid_url`——调用方据此给出「仅支持 http/https/socks5」类提示。
    pub fn resolve(proxy_mode: &str, proxy_url: &str) -> Result<ProxyDecision, String> {
        let url = proxy_url.trim();
        match proxy_mode {
            "none" => Ok(ProxyDecision::NoProxy),
            "custom" if url.is_empty() => Ok(ProxyDecision::NoProxy),
            "custom" => {
                let parsed = reqwest::Url::parse(url).map_err(|_| "err.invalid_url".to_string())?;
                if !matches!(parsed.scheme(), "http" | "https" | "socks5") {
                    return Err("err.invalid_url".to_string());
                }
                Ok(ProxyDecision::Proxy(parsed))
            }
            _ => Ok(ProxyDecision::System),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision_kind(mode: &str, url: &str) -> &'static str {
        match ProxyPolicy::resolve(mode, url).unwrap() {
            ProxyDecision::NoProxy => "no_proxy",
            ProxyDecision::Proxy(u) => {
                assert_eq!(
                    u.host_str(),
                    reqwest::Url::parse(url.trim()).unwrap().host_str(),
                    "custom 代理应保留原始 host"
                );
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
        assert_eq!(decision_kind("custom", "socks5://127.0.0.1:1080"), "proxy");
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
        for bad in ["not a url", "ftp://example.com:21", "file:///etc/passwd"] {
            let err = ProxyPolicy::resolve("custom", bad).unwrap_err();
            assert_eq!(err, "err.invalid_url", "url={bad:?}");
        }
    }
}
