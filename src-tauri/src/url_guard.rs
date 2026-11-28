//! 浏览器类出口的协议白名单：`open_url`（拉起系统协议处理器）与登录窗导航守卫共用一份判据。
//!
//! 只比对 host 不比对 scheme 会让 `media://bilibili.com/...` 这类自定义协议借 host 通过，
//! 而 `http://` 降级又会把 cookie 放到明文链路，所以判据统一收在这里。

use tauri::Url;

fn parse(raw: &str) -> Option<Url> {
    Url::parse(raw).ok()
}

/// 仅 https：用于需要保住凭据链路的原生 webview 窗口。
pub(crate) fn is_https_url(raw: &str) -> bool {
    parse(raw).is_some_and(|u| u.scheme() == "https")
}

/// 交给系统浏览器前用：返回解析器规范化后的 URL 而非原始字符串。
///
/// `Url::parse` 会吞掉首尾空白与全串的 `\t`/`\r`/`\n`，直接透传原始串会让
/// ShellExecuteW 拿到与校验时不同的字节。`*.localhost` 与 `localhost` 不外开，
/// 与 lib.rs 主窗导航守卫的判定保持一致（这些域在本应用里是 media 等自定义协议域）。
pub(crate) fn web_url_for_open(raw: &str) -> Option<String> {
    let url = parse(raw)?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str().unwrap_or_default();
    if host == "localhost" || host.ends_with(".localhost") {
        return None;
    }
    Some(url.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_https_url_accepts_https() {
        assert!(is_https_url("https://passport.bilibili.com/login"));
    }

    #[test]
    fn test_https_url_rejects_downgrade_and_custom_schemes() {
        for raw in [
            "http://passport.bilibili.com/login",
            "media://bilibili.com/https://evil.example/a.png",
            "tauri://localhost/index.html",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "",
        ] {
            assert!(!is_https_url(raw), "raw={:?}", raw);
        }
    }

    #[test]
    fn test_web_url_for_open_accepts_both_web_schemes() {
        assert_eq!(
            web_url_for_open("https://github.com/o/r/releases/tag/v1").as_deref(),
            Some("https://github.com/o/r/releases/tag/v1")
        );
        assert_eq!(
            web_url_for_open("http://example.com").as_deref(),
            Some("http://example.com/")
        );
    }

    #[test]
    fn test_web_url_for_open_returns_normalized_url() {
        assert_eq!(
            web_url_for_open("  https://github.com/o/r\n").as_deref(),
            Some("https://github.com/o/r")
        );
        assert_eq!(
            web_url_for_open("https://git\thub.com/o/r").as_deref(),
            Some("https://github.com/o/r")
        );
    }

    #[test]
    fn test_web_url_for_open_rejects_localhost_domains() {
        for raw in [
            "http://localhost:5173/",
            "http://media.localhost/https://i.ytimg.com/a.jpg",
            "https://evil.localhost/",
        ] {
            assert!(web_url_for_open(raw).is_none(), "raw={:?}", raw);
        }
    }

    #[test]
    fn test_web_url_for_open_rejects_system_handler_schemes() {
        for raw in [
            "file:///C:/Windows/System32/calc.exe",
            "ms-settings:privacy",
            "search-ms:query=secret",
            "media://localhost/https://i.ytimg.com/a.jpg",
            "javascript:alert(1)",
            "C:\\Windows\\System32\\calc.exe",
            "",
        ] {
            assert!(web_url_for_open(raw).is_none(), "raw={:?}", raw);
        }
    }
}
