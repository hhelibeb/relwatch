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

// ── 通用分页拉取 + 重试（从 github.rs / huggingface.rs 下沉，消除逐字符重复）──

/// 从 Link header 中提取 `rel="next"` 的 URL。
/// Link header 格式:
/// `<https://api.github.com/repos/.../releases?per_page=100&page=2>; rel="next", ...`
pub fn parse_next_link(link_header: &str) -> Option<String> {
    for part in link_header.split(',') {
        let trimmed = part.trim();
        if trimmed.contains("rel=\"next\"") {
            let start = trimmed.find('<')?;
            let end = trimmed.find('>')?;
            return Some(trimmed[start + 1..end].to_string());
        }
    }
    None
}

/// 获取单页，返回 (items, 下一页 URL)。
/// 与 `github::fetch_releases_page` / `huggingface::fetch_models_page` 行为一致。
async fn fetch_page(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let reason = resp.status().canonical_reason().unwrap_or("").to_string();
        return Err((status, format!("err.api_error|{}|{}", status, reason)));
    }

    let next_url = resp
        .headers()
        .get("link")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_next_link);

    let items: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| (status, format!("err.parse_failed|{}", e)))?;

    Ok((items, next_url))
}

/// 重试包装：`should_retry` 返回 false 的错误不重试，其他可重试错误最多重试 3 次。
/// `should_retry` 由调用方传入，保留 source 间差异（如 403 不重试规则）。
async fn with_retry<T, F, Fut>(
    should_retry: impl Fn(&(u16, String)) -> bool,
    f: F,
) -> Result<T, (u16, String)>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, (u16, String)>>,
{
    let config = crate::retry::RetryConfig::default();
    crate::retry::retry_with_backoff(&config, should_retry, f).await
}

/// 默认重试判断：403 不重试（source 拒绝访问），其他可重试错误重试。
fn default_should_retry(e: &(u16, String)) -> bool {
    if e.0 == 403 {
        return false;
    }
    log::warn!("请求失败(状态={}), 将重试: {}", e.0, e.1);
    true
}

/// 单页拉取 + 默认重试。返回 (items, 下一页 URL)。
pub async fn fetch_page_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    with_retry(default_should_retry, || async { fetch_page(client, url).await }).await
}

/// 翻页拉取直到满足 `max_count` 或无下一页。复用 `fetch_page_with_retry`。
/// - `None` = 不设上限（拉取全部）
/// - `Some(n)` = 拉取至少 n 条后停止
pub async fn paginated_fetch(
    client: &reqwest::Client,
    first_url: String,
    max_count: Option<usize>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let mut all = Vec::new();
    let mut url = first_url;

    loop {
        let (items, next_url) = fetch_page_with_retry(client, &url).await?;
        let count = items.len();
        all.extend(items);
        log::info!(
            "分页拉取 {}: 获取 {} 条{}",
            url,
            count,
            next_url.as_ref().map(|_| "，还有下一页").unwrap_or("，已完成"),
        );

        if let Some(limit) = max_count {
            if all.len() >= limit {
                log::info!("已获取 {} 条，达到上限 {}，停止翻页", all.len(), limit);
                break;
            }
        }

        match next_url {
            Some(next) => url = next,
            None => break,
        }
    }

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_next_link_found() {
        let header = "<https://api.github.com/repos/o/r/releases?per_page=100&page=2>; rel=\"next\", \
                       <https://api.github.com/repos/o/r/releases?per_page=100&page=4>; rel=\"last\"";
        assert_eq!(
            parse_next_link(header).as_deref(),
            Some("https://api.github.com/repos/o/r/releases?per_page=100&page=2")
        );
    }

    #[test]
    fn test_parse_next_link_found_hf_format() {
        let header = "<https://huggingface.co/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=2>; rel=\"next\", \
                       <https://huggingface.co/api/models?author=org&p=5>; rel=\"last\"";
        assert_eq!(
            parse_next_link(header).as_deref(),
            Some("https://huggingface.co/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=2")
        );
    }

    #[test]
    fn test_parse_next_link_not_found() {
        let header = "<https://api.github.com/repos/o/r/releases?per_page=100&page=1>; rel=\"last\"";
        assert!(parse_next_link(header).is_none());
    }

    #[test]
    fn test_parse_next_link_empty() {
        assert!(parse_next_link("").is_none());
    }

    #[test]
    fn test_parse_next_link_no_brackets() {
        let header = "rel=\"next\"";
        assert!(parse_next_link(header).is_none());
    }
}
