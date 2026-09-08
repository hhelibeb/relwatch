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
    /// 是否跟随 HTTP 重定向（默认 true）。SSRF 防护场景（如下载任意 URL）必须传
    /// false：reqwest 自动跟随重定向时**不会**对跳转目标重新校验，攻击者可用
    /// 302 把请求导向内网。调用方需手动跟随并每跳校验（见 commands/download.rs）。
    pub follow_redirects: bool,
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
            follow_redirects: true,
        }
    }
}

/// 通用 HTTP 客户端构建器，供 GitHub API 和 DeepSeek API 共用。
///
/// **注意**：当 `set_default_auth=false`（默认）时，`bearer_token` **不会**被设为
/// default header；调用方必须在每个需要鉴权的请求上通过 `bearer_auth` 单独设置，
/// 避免共享 client 时 token 被发给无关域名。
pub fn build_http_client(config: HttpClientConfig) -> Result<reqwest::Client, String> {
    build_http_client_with_dns(config, &[])
}

/// `build_http_client` + DNS 固定（`ClientBuilder::resolve_to_addrs`）。
///
/// SSRF 场景（下载任意 URL / media 网关）使用：先把目标域名解析并校验为公网 IP，
/// 再把“校验通过的那组 IP”固定给 client。这样后续请求**不会再次发起 DNS 解析**，
/// 堵住「校验时解析 A、请求时重新解析成 B」的 DNS 重绑定（TOCTOU）绕过。
///
/// `overrides`: `(域名, 已校验公网的 IP 列表)`。固定后 reqwest 只连这些 IP；
/// 连接端口不受 override 影响——此处统一填端口 0，reqwest 实际连接的端口
/// 始终由目标 URL 决定（含显式非默认端口，如 `http://host:8080`）。
fn build_http_client_with_dns(
    config: HttpClientConfig,
    overrides: &[(&str, &[std::net::IpAddr])],
) -> Result<reqwest::Client, String> {
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
    if !config.follow_redirects {
        // SSRF 防护：禁用自动重定向，调用方手动跟随并每跳重新校验目标地址
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    if headers.is_empty() {
        // 无 headers 时不调用 default_headers（仅 user_agent）
    } else {
        builder = builder.default_headers(headers);
    }
    for (domain, ips) in overrides {
        let addrs: Vec<std::net::SocketAddr> = ips
            .iter()
            .map(|ip| std::net::SocketAddr::new(*ip, 0))
            .collect();
        builder = builder.resolve_to_addrs(domain, &addrs);
    }
    // 三态语义唯一来源（见 net::ProxyPolicy）：none/custom+空 → 直连，
    // custom → 显式代理，system/未知 → 由 reqwest 追加系统代理。
    // 代理 URL 非法（协议不支持/解析失败）时返回 i18n 错误码，由调用方翻译。
    match crate::net::ProxyPolicy::resolve(config.proxy_mode, config.proxy_url)? {
        crate::net::ProxyDecision::NoProxy => {
            builder = builder.no_proxy();
        }
        crate::net::ProxyDecision::Proxy(url) => {
            // resolve 已保证 scheme ∈ {http,https,socks5} 且 Url 可解析；
            // Proxy::all 仅在这些约束不满足时失败，此分支实际不可达，防御性兜底。
            let proxy = reqwest::Proxy::all(url.to_string())
                .map_err(|_| "err.invalid_url".to_string())?;
            builder = builder.proxy(proxy);
        }
        crate::net::ProxyDecision::System => {
            // 不设置任何 proxy，让 reqwest 使用系统代理（Windows 默认行为）
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
/// 返回「目标域名 + 已通过公网校验的全部 IP」，供调用方把 IP 固定给 reqwest client
/// （`resolve_to_addrs`），使**实际连接使用的 IP 与校验通过的 IP 是同一组**——
/// 堵住「校验时 DNS 解析一次、请求时 reqwest 再解析一次」的 DNS 重绑定（TOCTOU）绕过。
///
/// - host 为 IP 字面量：直接 `is_private_or_reserved` 判定，合法时返回该 IP；
/// - host 为域名：DNS 解析**全部**地址，任一落在私网即拒绝（fail-closed）；
/// - DNS 解析失败（故障/NXDOMAIN/无网络）：**拒绝**（fail-closed）——此前 fail-open
///   放行会绕过 IPv6 zone-id 等无法解析的私网形式，且解析失败时请求本身也无法成功，
///   放行没有实际收益，反而留下 SSRF 绕过面。
///
/// 错误统一为 `err.*` 格式（i18n 由调用方负责）。
pub async fn resolve_public_host(url: &str) -> Result<(String, Vec<std::net::IpAddr>), String> {
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
            Ok((host, vec![ip]))
        };
    }
    // 域名：解析全部地址，任一私网即拒绝（fail-closed）。
    // tokio 的 lookup_host 对纯字符串只接受 IP:port 字面量，域名需传 (host, port)
    // 元组（owned String + u16，无借用）；port 不影响解析结果。
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host.clone(), port))
        .await
        .map_err(|e| format!("err.dns_resolve_failed|{}", e))?;
    let verified = collect_public_ips(addrs.map(|a| a.ip()))?;
    Ok((host, verified))
}

/// 把一组 DNS 解析结果收敛为「已通过公网校验的 IP 列表」（fail-closed）：
/// - 任一地址落在私网/保留段 → 整批拒绝，不看其余地址；
/// - 解析成功但一个地址都没返回 → 同样拒绝（否则会把空列表固定给 client，
///   语义上等于"校验通过"，与 fail-closed 自述不符）。
fn collect_public_ips<I: IntoIterator<Item = std::net::IpAddr>>(
    addrs: I,
) -> Result<Vec<std::net::IpAddr>, String> {
    let mut verified = Vec::new();
    for ip in addrs {
        if is_private_or_reserved(ip) {
            return Err("err.private_url_blocked".to_string());
        }
        if !verified.contains(&ip) {
            verified.push(ip);
        }
    }
    if verified.is_empty() {
        return Err("err.dns_resolve_failed|no addresses".to_string());
    }
    Ok(verified)
}

/// 便捷版：仅校验，不返回 IP（无 DNS 固定需求的调用方用，语义同旧 `ensure_public_url`）。
/// 当前仅测试直接使用，生产路径统一走 `resolve_public_host`（带 DNS 固定）。
#[cfg_attr(not(test), allow(dead_code))]
pub async fn ensure_public_url(url: &str) -> Result<(), String> {
    resolve_public_host(url).await.map(|_| ())
}

/// 下载 URL 的原始字节（剪贴板图片等场景），限制最大 `max_bytes` 防止异常响应撑爆内存。
/// scheme 校验由调用方负责；错误统一为 `err.*` i18n 格式。
///
/// 注意：`fetch_url_bytes`（commands/download.rs）与 media 网关共用
/// `fetch_public_bytes`（带 SSRF 逐跳校验）；本函数保留为通用下载原语
/// （wiremock 测试覆盖响应处理逻辑），跟随 reqwest 默认重定向，仅适用于
/// 调用方已自行校验目标的场景。
#[cfg_attr(not(test), allow(dead_code))]
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

/// 带 SSRF 逐跳校验的公开资源下载核心：供「下载任意 URL」与 media:// 协议共用。
///
/// - 要求 URL 为 http/https；
/// - 手动跟随重定向（最多 10 跳），**每一跳先 `resolve_public_host` 解析并校验为
///   公网 IP，再以 `resolve_to_addrs` 把该组 IP 固定到本次请求的 client**——这样
///   实际连接的 IP 与校验通过的 IP 完全一致，堵住 DNS 重绑定（TOCTOU）绕过（评审 P1-2）；
///   禁自动重定向是因为 reqwest 自动跟随不会对跳转目标重新校验，恶意服务器可用 302
///   把请求导向内网（如 169.254.169.254 云元数据）；
/// - 响应体不得超过 `max_bytes`。
///
/// 代理语义：`config` 携带 proxy_url/proxy_mode（`HttpClientConfig` 复用），每次请求
/// 都按此重新构建 client 并附加 DNS 固定；`follow_redirects` 固定为 false。
///
/// # 为什么不在外部传入已建好的 client
///
/// 旧实现接收调用方构建好的 `&reqwest::Client`，但该 client 已固定 DNS 解析器，无法再
/// 附加 `resolve_to_addrs`，只能依赖“校验后让 reqwest 再解析一次” —— 这正是 TOCTOU 的
/// 根因。改为接收配置、内部重建，代价是每次下载重建一次 client（连接池/句柄开销可忽略，
/// 与 media 网关既有的“每请求重建”行为一致）。
pub async fn fetch_public_bytes(
    config: &HttpClientConfig<'_>,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let (bytes, _content_type) = fetch_public_with_headers(config, url, max_bytes).await?;
    Ok(bytes)
}

/// `fetch_public_bytes` 的带 Content-Type 版本。media:// 需要把远端响应的
/// Content-Type 原样透传给 Chromium，否则某些 `<img>` 场景可能被误判。
///
/// # custom 代理模式的边界（威胁模型）
///
/// 当 `proxy_mode=custom` 时，实际 DNS 解析发生在**代理端**：reqwest 把请求交给代理，
/// 代理再解析目标域名——本函数的 `resolve_to_addrs` 固定对代理转发不生效，恶意代理可以
/// 在转发时把目标解析到内网。这意味着 DNS 重绑定防护仅在直连（`none`/`system`）下
/// 完整成立；custom 代理下本函数仍做逐跳公网校验（fail-closed），但**无法约束代理内部
/// 的二次解析**。该限制是代理架构固有（代理本身是可信出口），已在评审中记为残余面。
pub async fn fetch_public_with_headers(
    config: &HttpClientConfig<'_>,
    url: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<String>), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("err.invalid_url".to_string());
    }
    let mut current = url.to_string();
    for _ in 0..10 {
        // 解析 + 公网校验，拿到校验通过的 IP 列表（fail-closed）
        let (host, ips) = resolve_public_host(&current).await?;
        // 为本次请求构建带 DNS 固定的 client：reqwest 将直连校验过的 IP，不再二次解析。
        // 注意域名请求的 Host/SNI 仍为原始域名，证书校验不受影响。
        let client = build_http_client_with_dns(
            HttpClientConfig {
                proxy_url: config.proxy_url,
                proxy_mode: config.proxy_mode,
                bearer_token: config.bearer_token,
                timeout_secs: config.timeout_secs,
                content_type_json: false,
                set_default_auth: false,
                follow_redirects: false,
            },
            &[(&host, &ips)],
        )?;
        let resp = client
            .get(&current)
            .send()
            .await
            .map_err(|e| format!("err.request_failed|{}", e))?;
        if let Some(loc) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
        {
            current = reqwest::Url::parse(&current)
                .map_err(|_| "err.invalid_url".to_string())?
                .join(loc)
                .map_err(|_| "err.invalid_url".to_string())?
                .to_string();
            // 只跟随 http/https 跳转（join 已保证绝对 URL 合法，这里再收紧 scheme）
            if !(current.starts_with("https://") || current.starts_with("http://")) {
                return Err("err.invalid_url".to_string());
            }
            continue;
        }
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("err.download_failed|HTTP {}", status.as_u16()));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > max_bytes {
                return Err(format!("err.download_failed|file too large ({} bytes)", len));
            }
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
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
        return Ok((bytes.to_vec(), content_type));
    }
    Err("err.download_failed|too many redirects".to_string())
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
    async fn test_resolve_public_host_ip_literal_returns_pinned_ip() {
        // IP 字面量：直接放行，返回的固定 IP 就是该字面量（无需再解析）
        let (host, ips) = resolve_public_host("https://8.8.8.8/x").await.unwrap();
        assert_eq!(host, "8.8.8.8");
        assert_eq!(ips, vec!["8.8.8.8".parse::<std::net::IpAddr>().unwrap()]);
    }

    #[tokio::test]
    async fn test_resolve_public_host_rejects_private_ip_literal() {
        let err = resolve_public_host("http://127.0.0.1:8080/x").await.unwrap_err();
        assert!(err.contains("err.private_url_blocked"), "{}", err);
    }

    #[tokio::test]
    async fn test_resolve_public_host_dns_failure_fails_closed() {
        // .invalid 保留域名解析必失败：fail-closed（不返回空 IP 列表）
        let err = resolve_public_host("https://ssrf-test.invalid/x").await.unwrap_err();
        assert!(err.contains("err.dns_resolve_failed"), "{}", err);
    }

    #[tokio::test]
    async fn test_resolve_public_host_rejects_invalid() {
        for url in ["not-a-url", "ftp://example.com/x", "file:///etc/passwd"] {
            assert!(
                resolve_public_host(url).await.is_err(),
                "{} 应拒绝",
                url
            );
        }
    }

    #[tokio::test]
    async fn test_resolve_public_host_ipv6_literal() {
        // IPv6 字面量同样返回去重后的固定 IP（公网）
        let (host, ips) = resolve_public_host("https://[2606:4700::1111]/x").await.unwrap();
        assert_eq!(host, "2606:4700::1111");
        assert_eq!(ips, vec!["2606:4700::1111".parse::<std::net::IpAddr>().unwrap()]);
    }

    #[tokio::test]
    async fn test_resolve_public_host_rejects_private_ipv6_literal() {
        let err = resolve_public_host("https://[::1]:443/x").await.unwrap_err();
        assert!(err.contains("err.private_url_blocked"), "{}", err);
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
        // IP 字面量不依赖 DNS，稳定放行；域名路径见下方 fail-closed 测试
        let allowed = [
            "https://8.8.8.8/x",
            "https://104.16.1.1/x",
        ];
        for url in allowed {
            assert!(ensure_public_url(url).await.is_ok(), "{} 应放行", url);
        }
    }

    #[tokio::test]
    async fn test_ensure_public_url_fails_closed_on_dns_failure() {
        // RFC 2606 保留域名 .invalid 解析必失败：fail-closed 应返回错误而不是放行
        let err = ensure_public_url("https://ssrf-test.invalid/x").await.unwrap_err();
        assert!(
            err.contains("err.dns_resolve_failed"),
            "DNS 解析失败应 fail-closed 拒绝: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_ensure_public_url_rejects_invalid() {
        let invalid = ["not-a-url", "ftp://example.com/x", "file:///etc/passwd"];
        for url in invalid {
            assert!(ensure_public_url(url).await.is_err(), "{} 应拒绝", url);
        }
    }

    #[test]
    fn test_collect_public_ips_rejects_empty() {
        // 解析「成功但零地址」不能当成校验通过：否则会把空列表固定给 client，
        // 语义上等于放行（与 fail-closed 自述不符）
        let err = collect_public_ips(Vec::new()).unwrap_err();
        assert!(err.contains("err.dns_resolve_failed"), "{}", err);
    }

    #[test]
    fn test_collect_public_ips_rejects_private_among_public() {
        // 任一私网即整批拒绝：不允许「挑出公网地址继续用」
        let ips: Vec<std::net::IpAddr> = vec![
            "93.184.216.34".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
            "8.8.8.8".parse().unwrap(),
        ];
        let err = collect_public_ips(ips).unwrap_err();
        assert!(err.contains("err.private_url_blocked"), "{}", err);
    }

    #[tokio::test]
    async fn test_dns_override_pins_request_to_verified_ip() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // 本地起一个极简 HTTP 服务，把一个域名固定到 127.0.0.1。
        // 若 `resolve_to_addrs` 未生效，`pinned.invalid` 会走真实 DNS 直接失败——
        // 因此「请求成功」即证明实际连接用的是被固定的 IP，没有发生二次解析。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut acc = Vec::new();
            let mut buf = [0u8; 1024];
            while !acc.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                acc.extend_from_slice(&buf[..n]);
            }
            let resp = b"HTTP/1.1 200 OK\r\ncontent-length: 6\r\n\r\npinned";
            sock.write_all(resp).await.unwrap();
            sock.flush().await.unwrap();
        });

        let pinned: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let pinned_addrs: [std::net::IpAddr; 1] = [pinned];
        let overrides: [(&str, &[std::net::IpAddr]); 1] = [("pinned.invalid", &pinned_addrs)];
        let client = build_http_client_with_dns(
            HttpClientConfig {
                timeout_secs: 5,
                follow_redirects: false,
                ..Default::default()
            },
            &overrides,
        )
        .unwrap();
        let resp = client
            .get(format!("http://pinned.invalid:{}/x", port))
            .send()
            .await
            .expect("请求应命中被固定的 127.0.0.1，而不是重新解析 DNS");
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.text().await.unwrap(), "pinned");
    }
}
