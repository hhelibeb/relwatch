/// HTTP 客户端构建配置。
pub struct HttpClientConfig<'a> {
    pub proxy_url: &'a str,
    pub proxy_mode: &'a str,
    pub bearer_token: Option<&'a str>,
    pub timeout_secs: u64,
    pub content_type_json: bool,
}

impl<'a> Default for HttpClientConfig<'a> {
    fn default() -> Self {
        Self {
            proxy_url: "",
            proxy_mode: "none",
            bearer_token: None,
            timeout_secs: 30,
            content_type_json: false,
        }
    }
}

/// 通用 HTTP 客户端构建器，供 GitHub API 和 DeepSeek API 共用。
pub fn build_http_client(config: HttpClientConfig) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = config.bearer_token {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                .map_err(|e| format!("无效的 Bearer Token: {}", e))?,
        );
    }
    if config.content_type_json {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    let mut builder = reqwest::Client::builder()
        .user_agent("RelWatch/0.4")
        .timeout(std::time::Duration::from_secs(config.timeout_secs))
        .connect_timeout(std::time::Duration::from_secs(10));
    if config.bearer_token.is_none() && headers.is_empty() {
        // 无 headers 时不调用 default_headers（仅 user_agent）
    } else {
        builder = builder.default_headers(headers);
    }
    match config.proxy_mode {
        "none" => {
            builder = builder.no_proxy();
        }
        "system" => {
            // 不设置任何 proxy，让 reqwest 使用系统代理（Windows 默认行为）
        }
        _ => {
            // "custom" 或其他值：使用 proxy_url
            if !config.proxy_url.is_empty() {
                if let Ok(proxy) = reqwest::Proxy::all(config.proxy_url) {
                    builder = builder.proxy(proxy);
                } else {
                    return Err(format!(
                        "Invalid proxy URL: {} — 仅支持 http://、https:// 和 socks5:// 协议",
                        config.proxy_url
                    ));
                }
            } else {
                builder = builder.no_proxy();
            }
        }
    }
    builder.build().map_err(|e| e.to_string())
}
