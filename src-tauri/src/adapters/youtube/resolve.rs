use crate::error::SourceError;
use super::rss::{feed_url, fetch_feed_with_retry, parse_feed};
use super::FeedKind;

pub(super) fn extract_channel_id_from_input(input: &str) -> Option<String> {
    let input = input.trim();
    if input.starts_with("UC") && input.len() >= 12 {
        let end = input
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(input.len());
        let id = &input[..end];
        if id.len() >= 12 {
            return Some(id.to_string());
        }
    }
    if let Some(idx) = input.find("channel/") {
        let rest = &input[idx + "channel/".len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(rest.len());
        let id = &rest[..end];
        if id.starts_with("UC") && id.len() >= 12 {
            return Some(id.to_string());
        }
    }
    None
}

pub(super) fn channel_page_url(input: &str) -> Result<String, SourceError> {
    let input = input.trim();
    if input.starts_with("http://") || input.starts_with("https://") {
        let after_scheme = &input[input.find("://").unwrap() + 3..];
        let host = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            host.as_str(),
            "youtube.com" | "www.youtube.com" | "m.youtube.com"
        ) {
            return Err(SourceError::coded(
                400,
                "err.youtube_invalid_channel",
                ["non-youtube-host"],
            ));
        }
        return Ok(input.to_string());
    }
    Ok(format!(
        "https://www.youtube.com/@{}",
        input.trim_start_matches('@')
    ))
}

pub(super) fn extract_channel_id_from_html(html: &str) -> Option<String> {
    if let Some(idx) = html.find("\"channelId\"") {
        let after = &html[idx + "\"channelId\"".len()..];
        if let Some(colon) = after.find(':') {
            let val = &after[colon + 1..];
            let val = val.trim_start_matches([' ', '"']);
            if val.starts_with("UC") {
                let end = val
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                    .unwrap_or(val.len());
                let id = &val[..end];
                if id.len() >= 12 {
                    return Some(id.to_string());
                }
            }
        }
    }
    if let Some(idx) = html.find("itemprop=\"channelId\"") {
        let rest = &html[idx..];
        if let Some(cid) = rest.find("content=\"") {
            let start = idx + cid + "content=\"".len();
            let end = html[start..].find('"')?;
            let id = &html[start..start + end];
            if id.starts_with("UC") && id.len() >= 12 {
                return Some(id.to_string());
            }
        }
    }
    if let Some(idx) = html.find("rel=\"canonical\"") {
        let rest = &html[idx..];
        if let Some(href) = rest.find("href=\"") {
            let start = idx + href + "href=\"".len();
            let end = html[start..].find('"')?;
            let url = &html[start..start + end];
            return extract_channel_id_from_input(url);
        }
    }
    None
}

pub(super) async fn verify_and_describe_channel(
    client: &reqwest::Client,
    channel_id: &str,
) -> Result<String, SourceError> {
    let page_url = format!("https://www.youtube.com/channel/{}", channel_id);
    if let Ok(html) = crate::http::fetch_text_with_retry(client, &page_url).await {
        if let Some(name) = crate::http::extract_og_title(&html) {
            return Ok(name);
        }
    }
    let url = feed_url(channel_id, FeedKind::Video)?;
    let xml = fetch_feed_with_retry(client, &url).await?;
    let parsed = parse_feed(&xml, FeedKind::Video)
        .map_err(|e| SourceError::coded(0, "err.parse_failed", [e]))?;
    if parsed.channel_title.is_empty() {
        return Err(SourceError::coded(
            404,
            "err.youtube_channel_not_found",
            [channel_id],
        ));
    }
    Ok(parsed.channel_title)
}

pub async fn resolve_channel_id(
    client: &reqwest::Client,
    input: &str,
) -> Result<String, SourceError> {
    if let Some(id) = extract_channel_id_from_input(input) {
        return Ok(id);
    }
    let page_url = channel_page_url(input)?;
    let html = crate::http::fetch_text(client, &page_url).await?;
    extract_channel_id_from_html(&html)
        .ok_or_else(|| SourceError::coded(404, "err.youtube_channel_not_found", [input]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_extract_channel_id_from_input_direct() {
        assert_eq!(
            extract_channel_id_from_input("UCabcdefghijklmnopqrst").as_deref(),
            Some("UCabcdefghijklmnopqrst")
        );
    }

    #[test]
    fn test_extract_channel_id_from_input_channel_url() {
        assert_eq!(
            extract_channel_id_from_input("https://www.youtube.com/channel/UCabcdefghijklmnopqrst")
                .as_deref(),
            Some("UCabcdefghijklmnopqrst")
        );
    }

    #[test]
    fn test_extract_channel_id_from_input_channel_url_with_suffix() {
        assert_eq!(
            extract_channel_id_from_input("youtube.com/channel/UCabcdefghijklmnopqrst/featured")
                .as_deref(),
            Some("UCabcdefghijklmnopqrst")
        );
    }

    #[test]
    fn test_extract_channel_id_from_input_handle_not_resolved() {
        assert!(extract_channel_id_from_input("@somehandle").is_none());
    }

    #[test]
    fn test_channel_page_url_variants() {
        assert_eq!(
            channel_page_url("@handle").unwrap(),
            "https://www.youtube.com/@handle"
        );
        assert_eq!(
            channel_page_url("handle").unwrap(),
            "https://www.youtube.com/@handle"
        );
        assert_eq!(
            channel_page_url("https://www.youtube.com/c/name").unwrap(),
            "https://www.youtube.com/c/name"
        );
        assert_eq!(
            channel_page_url("youtube.com/user/old").unwrap(),
            "https://www.youtube.com/@youtube.com/user/old"
        );
    }

    #[test]
    fn test_channel_page_url_rejects_non_youtube_host() {
        assert!(channel_page_url("http://127.0.0.1:8080/x").is_err());
        assert!(channel_page_url("https://evil.example.com/x").is_err());
        assert!(channel_page_url("https://youtube.com.evil.example.com/x").is_err());
        assert!(channel_page_url("http://youtube.com@evil.example.com/x").is_err());
        assert!(channel_page_url("https://www.youtube.com:443/@handle").is_ok());
        assert!(channel_page_url("https://m.youtube.com/@handle").is_ok());
        assert!(channel_page_url("https://www.youtube.com/c/name").is_ok());
    }

    #[test]
    fn test_extract_channel_id_from_html_json() {
        let html = r#"<script>var ytInitialData = {"header":{"channelId":"UCabcdefghijklmnopqrst"}};</script>"#;
        assert_eq!(
            extract_channel_id_from_html(html).as_deref(),
            Some("UCabcdefghijklmnopqrst")
        );
    }

    #[test]
    fn test_extract_channel_id_from_html_meta() {
        let html = r#"<meta itemprop="channelId" content="UCabcdefghijklmnopqrst">"#;
        assert_eq!(
            extract_channel_id_from_html(html).as_deref(),
            Some("UCabcdefghijklmnopqrst")
        );
    }

    #[test]
    fn test_extract_channel_id_from_html_canonical() {
        let html = r#"<link rel="canonical" href="https://www.youtube.com/channel/UCabcdefghijklmnopqrst">"#;
        assert_eq!(
            extract_channel_id_from_html(html).as_deref(),
            Some("UCabcdefghijklmnopqrst")
        );
    }

    #[test]
    fn test_extract_channel_id_from_html_not_found() {
        assert!(extract_channel_id_from_html("<html><body>nothing</body></html>").is_none());
    }

    #[test]
    fn test_extract_og_title_variants() {
        let html = r#"<html><head><meta property="og:title" content="示例频道"></head></html>"#;
        assert_eq!(crate::http::extract_og_title(html).as_deref(), Some("示例频道"));
        let html2 = r#"<meta property="og:title" content="A &amp; B">"#;
        assert_eq!(crate::http::extract_og_title(html2).as_deref(), Some("A & B"));
        assert!(crate::http::extract_og_title("<html>no og title</html>").is_none());
        assert!(crate::http::extract_og_title("").is_none());
        assert!(crate::http::extract_og_title("<meta property=\"og:title\" content=\"\">").is_none());
    }

    #[tokio::test]
    async fn test_resolve_channel_id_direct_no_http() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let id = resolve_channel_id(&client, "UCabcdefghijklmnopqrst")
            .await
            .unwrap();
        assert_eq!(id, "UCabcdefghijklmnopqrst");
    }

    #[tokio::test]
    async fn test_resolve_channel_id_via_page() {
        let mock = MockServer::start().await;
        let html = r#"<html><meta itemprop="channelId" content="UCResolvedFromPage12345678"></html>"#;
        Mock::given(method("GET"))
            .and(path("/@somehandle"))
            .respond_with(ResponseTemplate::new(200).set_body_string(html))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let resp = client.get(format!("{}/@somehandle", mock.uri())).send().await.unwrap();
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert_eq!(
            extract_channel_id_from_html(&body).as_deref(),
            Some("UCResolvedFromPage12345678")
        );
    }
}
