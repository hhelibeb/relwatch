/// HTTP 客户端构建配置。
pub struct HttpClientConfig<'a> {
    pub proxy_url: &'a str,
    pub proxy_mode: &'a str,
    pub bearer_token: Option<&'a str>,
    pub timeout_secs: u64,
    pub content_type_json: bool,
    /// 为 true 时把 `bearer_token` 作为 client 的 **default header**（对所有域名生效）。
    /// 仅 DeepSeek 这种「所有请求都打同一 API 域名」的场景可安全设 true。
    /// GitHub 监控层与 HuggingFace 共用 client 抓取时必须设 false（默认），
    /// 否则 GitHub Token 会以 default header 形式泄露给 huggingface.co。
    /// GitHub 的 token 改由 `http::fetch_page_with_retry` / `paginated_fetch` 的
    /// `token` 参数按请求设置（仅对 github 请求生效）。
    pub set_default_auth: bool,
}

impl<'a> Default for HttpClientConfig<'a> {
    fn default() -> Self {
        Self {
            proxy_url: "",
            proxy_mode: "none",
            bearer_token: None,
            timeout_secs: 30,
            content_type_json: false,
            set_default_auth: false,
        }
    }
}

/// 通用 HTTP 客户端构建器，供 GitHub API 和 DeepSeek API 共用。
///
/// **注意**：当 `set_default_auth=false`（默认）时，`bearer_token` **不会**被设为
/// default header；调用方必须在每个需要鉴权的请求上通过 `bearer_auth` 单独设置，
/// 避免共享 client 时 token 被发给无关域名。
pub fn build_http_client(config: HttpClientConfig) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    if config.set_default_auth {
        if let Some(token) = config.bearer_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                    .map_err(|e| format!("无效的 Bearer Token: {}", e))?,
            );
        }
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
    if headers.is_empty() {
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

/// 获取单页，返回 (items, 下一页 URL)。`token` 按请求设置 Authorization（仅作用于
/// 本次请求的 URL，不会泄露给其它域名）。HuggingFace 等无需鉴权的源传 `None`。
/// 与 `huggingface::fetch_models_page` 行为一致。
async fn fetch_page(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    let mut req = client.get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req
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

/// 默认非 2xx 错误映射：`err.api_error|status|reason`。
/// `body` 仅在自定义映射（如解析 API 错误 JSON）时使用，默认映射忽略。
pub fn default_api_error(status: u16, _body: &str) -> (u16, String) {
    let reason = reqwest::StatusCode::from_u16(status)
        .map(|s| s.canonical_reason().unwrap_or("").to_string())
        .unwrap_or_default();
    (status, format!("err.api_error|{}|{}", status, reason))
}

/// GET 并取文本（无重试）：统一「send → 状态映射 → text」原语（M3）。
/// 供 XML / HTML / JSON 各路径复用，避免 youtube/bilibili 各自重写
/// 「send → is_success → err.api_error」块；`build_req` 注入自定义 header
/// （Cookie/UA/Referer 等），`map_err` 做 source 特定错误映射（如 B 站 412、
/// YouTube key/配额）。非 2xx 时先取 body 再映射，供需要解析错误体的映射器。
pub async fn get_text<B, M>(
    client: &reqwest::Client,
    url: &str,
    build_req: B,
    map_err: M,
) -> Result<String, (u16, String)>
where
    B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    M: Fn(u16, &str) -> (u16, String),
{
    let resp = build_req(client.get(url))
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    if !status.is_success() {
        return Err(map_err(status.as_u16(), &text));
    }
    Ok(text)
}

/// 带重试的 `get_text`：XML/HTML/JSON 路径统一复用重试骨架与错误格式化（M3）。
pub async fn get_text_with_retry<B, M, R>(
    client: &reqwest::Client,
    url: &str,
    build_req: B,
    should_retry: R,
    map_err: M,
) -> Result<String, (u16, String)>
where
    B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    M: Fn(u16, &str) -> (u16, String),
    R: Fn(&(u16, String)) -> bool,
{
    with_retry(should_retry, || async {
        get_text(client, url, &build_req, &map_err).await
    })
    .await
}

/// 便捷版 `get_text`：默认请求（无额外 header）、默认错误映射。
pub async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, (u16, String)> {
    get_text(client, url, |r| r, default_api_error).await
}

/// 便捷版 `get_text_with_retry`：默认 header、默认重试规则（403 不重试）、默认错误映射。
pub async fn fetch_text_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, (u16, String)> {
    get_text_with_retry(client, url, |r| r, default_should_retry, default_api_error).await
}

/// 单页拉取 + 默认重试。返回 (items, 下一页 URL)。
/// `token` 为 `Some` 时仅对本次请求 URL 设置 Authorization（见 `fetch_page`）。
pub async fn fetch_page_with_retry(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    with_retry(default_should_retry, || async { fetch_page(client, url, token).await }).await
}

/// 翻页拉取直到满足 `max_count` 或无下一页。复用 `fetch_page_with_retry`。
/// - `None` = 不设上限（拉取全部）
/// - `Some(n)` = 拉取至少 n 条后停止
///
/// `token` 同 `fetch_page_with_retry`，按请求设置，避免泄露给无关域名。
pub async fn paginated_fetch(
    client: &reqwest::Client,
    first_url: String,
    max_count: Option<usize>,
    token: Option<&str>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let mut all = Vec::new();
    let mut url = first_url;

    // 翻页安全：记录首请求的 host，后续 next_url 必须与之同 host。
    // 防止服务器返回的 Link header 把携带 token 的请求导向任意域名
    // （见 fetch_page 的按请求 bearer_auth 注入机制）。
    let first_host = reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()));

    loop {
        let (items, next_url) = fetch_page_with_retry(client, &url, token).await?;
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
            Some(next) => {
                // 校验 next_url 与首请求同 host；不一致视为不可信（被篡改/恶意
                // Link header），fail-closed 中断翻页，避免 token 发往无关域名。
                let same_host = match (&first_host, reqwest::Url::parse(&next).ok()) {
                    (Some(fh), Some(nu)) => nu.host_str() == Some(fh.as_str()),
                    _ => false,
                };
                if !same_host {
                    log::warn!(
                        "分页拉取中断: next_url host 与首请求不一致 (首={:?}, next={})",
                        first_host,
                        next
                    );
                    return Err((0, "err.invalid_next_url".to_string()));
                }
                url = next;
            }
            None => break,
        }
    }

    Ok(all)
}

/// 判断 IP 是否属于私网/回环/链路本地/保留地址（SSRF 防护用）。
///
/// 覆盖：
/// - IPv4：`0.0.0.0/8`、`10.0.0.0/8`、`100.64.0.0/10`（CGNAT）、`127.0.0.0/8`、
///   `169.254.0.0/16`（含云元数据 `169.254.169.254`）、`172.16.0.0/12`、`192.168.0.0/16`、
///   `198.18.0.0/15`、`224.0.0.0/4`（组播）与 `240.0.0.0/4`（保留）
/// - IPv6：`::`、`::1`、`fc00::/7`（ULA）、`fe80::/10`（链路本地）、`ff00::/8`（组播），
///   以及 IPv4-mapped（`::ffff:x.x.x.x`，还原为 IPv4 判定）
pub fn is_private_or_reserved(ip: std::net::IpAddr) -> bool {
    // IPv4-mapped IPv6 还原为 IPv4 判定，避免 `::ffff:192.168.1.1` 绕过
    let ip = match ip {
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => std::net::IpAddr::V4(v4),
            None => std::net::IpAddr::V6(v6),
        },
        v4 => v4,
    };
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            match o[0] {
                0 => true,                                 // 0.0.0.0/8
                10 => true,                                // 10.0.0.0/8
                100 => (64..=127).contains(&o[1]),         // 100.64.0.0/10
                127 => true,                               // 127.0.0.0/8
                169 => o[1] == 254,                        // 169.254.0.0/16
                172 => (16..=31).contains(&o[1]),          // 172.16.0.0/12
                192 => o[1] == 168,                        // 192.168.0.0/16
                198 => o[1] == 18 || o[1] == 19,           // 198.18.0.0/15
                224..=255 => true,                         // 组播 + 保留
                _ => false,
            }
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 ULA
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 链路本地
        }
    }
}

/// 校验 URL 目标为公网地址，拒绝私网/回环/链路本地/保留地址（SSRF 防护）。
///
/// - host 为 IP 字面量：直接 `is_private_or_reserved` 判定；
/// - host 为域名：DNS 解析**全部**地址，任一落在私网即拒绝（fail-closed）；
/// - DNS 解析失败（故障/无网络）：**放行**（fail-open），交由后续请求自然失败，
///   避免 DNS 瞬时抖动误伤正常下载。
pub async fn ensure_public_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "err.invalid_url".to_string())?;
    // 仅允许 http/https
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("err.invalid_url".to_string());
    }
    let Some(host) = parsed.host_str() else {
        return Err("err.invalid_url".to_string());
    };
    // host_str() 对 IPv6 字面量返回带括号形式（如 "[::1]"），去括号后统一判定
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let host = host.to_string(); // owned，避免跨 await 借用 parsed
    // IP 字面量：无需 DNS
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return if is_private_or_reserved(ip) {
            Err("err.private_url_blocked".to_string())
        } else {
            Ok(())
        };
    }
    // 域名：解析全部地址，任一私网即拒绝（fail-closed）；解析失败放行（fail-open）。
    // tokio 的 lookup_host 对纯字符串只接受 IP:port 字面量，域名需传 (host, port)
    // 元组（owned String + u16，无借用）；port 不影响解析结果。
    let port = parsed.port_or_known_default().unwrap_or(443);
    let lookup = tokio::net::lookup_host((host, port)).await;
    match lookup {
        Ok(addrs) => {
            for addr in addrs {
                if is_private_or_reserved(addr.ip()) {
                    return Err("err.private_url_blocked".to_string());
                }
            }
            Ok(())
        }
        // fail-open：解析失败放行
        Err(_) => Ok(()),
    }
}

/// 下载 URL 的原始字节（剪贴板图片等场景），限制最大 `max_bytes` 防止异常响应撑爆内存。
/// scheme 校验由调用方负责；错误统一为 `err.*` i18n 格式。
pub async fn download_bytes(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("err.request_failed|{}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("err.download_failed|HTTP {}", status.as_u16()));
    }
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(format!("err.download_failed|file too large ({} bytes)", len));
        }
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("err.request_failed|{}", e))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "err.download_failed|file too large ({} bytes)",
            bytes.len()
        ));
    }
    Ok(bytes.to_vec())
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

    // ── Token 泄露防护回归测试（问题1）──
    // 守护"GitHub Token 不得随 HF 请求泄露"的契约：
    // token=None 时请求**不携带** Authorization header（HF 场景）；
    // token=Some 时**携带** Authorization（GitHub 场景），且按请求设置、不依赖 default header。
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path, header_exists, query_param};

    #[tokio::test]
    async fn test_fetch_page_no_token_omits_authorization() {
        // 反证：挂一个"只在 Authorization header 存在时才返回 200"的 mock。
        // token=None 的请求只要不带 Authorization，就会落到 wiremock 默认的未匹配响应（非 2xx），
        // fetch_page_with_retry 以错误返回，从而证明没有携带 Authorization。
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(header_exists("authorization"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([])),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models", mock.uri());
        let result = fetch_page_with_retry(&client, &url, None).await;
        assert!(result.is_err(), "token=None 不应携带 Authorization，故不应命中要求该 header 的 mock");
    }

    #[tokio::test]
    async fn test_download_bytes_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/img.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8, 2, 3, 255]))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/img.png", mock.uri());
        let result = download_bytes(&client, &url, 1024).await;
        assert_eq!(result.unwrap(), vec![1u8, 2, 3, 255]);
    }

    #[tokio::test]
    async fn test_download_bytes_http_error() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing.png"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/missing.png", mock.uri());
        let err = download_bytes(&client, &url, 1024).await.unwrap_err();
        assert!(err.starts_with("err.download_failed|"), "意外的错误格式: {}", err);
    }

    #[tokio::test]
    async fn test_download_bytes_too_large() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 2048]))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/big.bin", mock.uri());
        let err = download_bytes(&client, &url, 1024).await.unwrap_err();
        assert!(err.contains("too large"), "应报大小超限: {}", err);
    }

    #[tokio::test]
    async fn test_fetch_page_with_token_sends_authorization() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(header_exists("authorization"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([])),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/repos/o/r/releases", mock.uri());
        let result = fetch_page_with_retry(&client, &url, Some("ghp_secret")).await;
        assert!(result.is_ok(), "token=Some 时应带 Authorization 命中要求该 header 的 mock");
    }

    // ── 翻页安全回归测试：next_url 必须与首请求同 host ──

    #[tokio::test]
    async fn test_paginated_fetch_same_host_follows_next() {
        let mock = MockServer::start().await;
        // page1：返回 1 条 + Link rel=next 指向同 host 的 page2
        Mock::given(method("GET"))
            .and(path("/api/v1/items"))
            .and(query_param("page", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([{"id": 1}]))
                    .insert_header(
                        "link",
                        format!("<{}/api/v1/items?page=2>; rel=\"next\"", mock.uri()),
                    ),
            )
            .mount(&mock)
            .await;
        // page2：返回 1 条，无 Link header → 翻页结束
        Mock::given(method("GET"))
            .and(path("/api/v1/items"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id": 2}])))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let first_url = format!("{}/api/v1/items?page=1", mock.uri());
        let result = paginated_fetch(&client, first_url, None, None).await;
        let items = result.expect("同 host 翻页应成功");
        assert_eq!(items.len(), 2, "应拉取 page1+page2 共 2 条");
    }

    #[tokio::test]
    async fn test_paginated_fetch_rejects_cross_host_next() {
        // 恶意场景：server A 的 Link header 指向不同 host 的 server B。
        // wiremock 只能绑 127.0.0.1，这里用原生 TCP listener 绑 127.0.0.2（回环段）
        // 充当 evil 地址，靠连接计数断言请求（含 token）从未外发。
        use std::net::Ipv4Addr;
        let evil_listener = std::net::TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), 0)).unwrap();
        let evil_port = evil_listener.local_addr().unwrap().port();
        let evil_url = format!("http://127.0.0.2:{}/api/v1/items?page=2", evil_port);
        let mock = MockServer::start().await;
        // server A 返回 Link rel=next 指向不同 host 的 evil 地址
        Mock::given(method("GET"))
            .and(path("/api/v1/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([{"id": 1}]))
                    .insert_header("link", format!("<{}>; rel=\"next\"", evil_url)),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let first_url = format!("{}/api/v1/items?page=1", mock.uri());
        let result = paginated_fetch(&client, first_url, None, Some("ghp_secret")).await;
        assert!(result.is_err(), "跨 host next_url 应 fail-closed 返回错误");

        // 关键断言：evil 地址未收到任何连接（token 未随恶意 next_url 外发）
        evil_listener.set_nonblocking(true).unwrap();
        match evil_listener.accept() {
            Ok(_) => panic!("evil 地址不应收到任何连接"),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("accept 出错: {}", e),
        }
    }

    #[tokio::test]
    async fn test_paginated_fetch_rejects_unparseable_next() {
        let mock = MockServer::start().await;
        // Link header 里的 next 不是合法 URL
        Mock::given(method("GET"))
            .and(path("/api/v1/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([{"id": 1}]))
                    .insert_header("link", "<not-a-url>; rel=\"next\""),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let first_url = format!("{}/api/v1/items?page=1", mock.uri());
        let result = paginated_fetch(&client, first_url, None, Some("ghp_secret")).await;
        assert!(result.is_err(), "无法解析的 next_url 应 fail-closed 返回错误");
    }

    // ── SSRF 防护：私网/回环/链路本地/保留地址判定 ──

    #[test]
    fn test_is_private_or_reserved() {
        use std::net::{IpAddr, Ipv4Addr};

        let private: &[IpAddr] = &[
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),          // CGNAT 下界
            IpAddr::V4(Ipv4Addr::new(100, 127, 255, 254)),     // CGNAT 上界
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),     // 云元数据
            IpAddr::V4(Ipv4Addr::new(172, 16, 0, 0)),          // 172.16/12 下界
            IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255)),      // 172.16/12 上界
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)),           // 组播
            IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)),     // 保留
            "::1".parse().unwrap(),
            "::".parse().unwrap(),
            "fc00::1".parse().unwrap(),
            "fd12:3456::1".parse().unwrap(),
            "fe80::1".parse().unwrap(),
            "ff02::1".parse().unwrap(),
            // IPv4-mapped 私网应被还原为 IPv4 判定
            "::ffff:192.168.1.1".parse().unwrap(),
            "::ffff:10.0.0.1".parse().unwrap(),
        ];
        for ip in private {
            assert!(is_private_or_reserved(*ip), "应判为私网/保留: {}", ip);
        }

        let public: &[IpAddr] = &[
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(104, 16, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
            "2606:4700::1111".parse().unwrap(),
            "2001:4860:4860::8888".parse().unwrap(),
            "::ffff:8.8.8.8".parse().unwrap(),
        ];
        for ip in public {
            assert!(!is_private_or_reserved(*ip), "不应判为私网: {}", ip);
        }
    }

    #[tokio::test]
    async fn test_ensure_public_url_blocks_private() {
        let blocked = [
            "http://127.0.0.1:8080/x",
            "http://10.0.0.5/x",
            "http://192.168.1.1/x",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]:443/x",
            "https://[::ffff:192.168.1.1]/x",
            "http://localhost:8080/x",
        ];
        for url in blocked {
            match ensure_public_url(url).await {
                Err(e) => assert!(
                    e.contains("err.private_url_blocked"),
                    "{} 错误码不正确: {}",
                    url,
                    e
                ),
                Ok(()) => panic!("{} 应被拒绝", url),
            }
        }
    }

    #[tokio::test]
    async fn test_ensure_public_url_allows_public() {
        let allowed = [
            "https://example.com/a.png", // 域名：解析到公网 IP；无网环境解析失败按 fail-open 放行
            "https://8.8.8.8/x",
            "https://104.16.1.1/x",
        ];
        for url in allowed {
            assert!(ensure_public_url(url).await.is_ok(), "{} 应放行", url);
        }
    }

    #[tokio::test]
    async fn test_ensure_public_url_rejects_invalid() {
        let invalid = ["not-a-url", "ftp://example.com/x", "file:///etc/passwd"];
        for url in invalid {
            assert!(ensure_public_url(url).await.is_err(), "{} 应拒绝", url);
        }
    }
}
