pub fn build_http_client(proxy_url: &str, github_token: Option<&str>) -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent("RelWatch/0.4")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10));
    if !proxy_url.is_empty() {
        if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
            builder = builder.proxy(proxy);
        } else {
            return Err(format!("Invalid proxy URL: {}", proxy_url));
        }
    }
    if let Some(token) = github_token {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                .map_err(|e| format!("无效的 GitHub Token: {}", e))?,
        );
        builder = builder.default_headers(headers);
    }
    builder.build().map_err(|e| e.to_string())
}
