use crate::db::sources::Source;
use crate::error::{SourceError, SourceErrorKind};
use super::resolve::extract_channel_id_from_input;
use super::{FeedEntry, FeedKind, SubscribeConfig};

const YT_API_PAGE_SIZE: usize = 50;

fn map_api_error(status: u16, body: &str) -> SourceError {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err) = v.get("error") {
            let reason = err
                .get("errors")
                .and_then(|e| e.get(0))
                .and_then(|e| e.get("reason"))
                .and_then(|r| r.as_str());
            if let Some(r) = reason {
                if r.contains("keyInvalid")
                    || r.contains("referrerNotAllowed")
                    || r.contains("ipRefererBlocked")
                {
                    return SourceError::new(
                        status,
                        SourceErrorKind::YoutubeApiKeyInvalid {
                            detail: Some(r.to_string()),
                        },
                    );
                }
                if r.contains("quotaExceeded") || r.contains("dailyLimitExceeded") {
                    return SourceError::new(
                        status,
                        SourceErrorKind::YoutubeApiQuota {
                            detail: Some(r.to_string()),
                        },
                    );
                }
            }
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            return SourceError::api_error(status, msg);
        }
    }
    SourceError::api_error(status, body)
}

async fn api_get_json_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<serde_json::Value, SourceError> {
    let text = crate::http::get_text_with_retry(
        client,
        url,
        |r| r,
        |e: &SourceError| {
            if matches!(e.status(), 400 | 401 | 403) {
                return false;
            }
            log::warn!("YouTube API 请求失败(状态={}), 将重试: {}", e.status(), e);
            true
        },
        map_api_error,
    )
    .await?;
    serde_json::from_str(&text)
        .map_err(|e| SourceError::coded(0, "err.parse_failed", [e.to_string()]))
}

fn extract_handle_from_input(input: &str) -> Result<Option<String>, SourceError> {
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
        let rest = after_scheme.split_once('/').map(|(_, r)| r).unwrap_or("");
        if let Some(handle) = rest.strip_prefix('@') {
            let handle = handle.split(['/', '?', '#']).next().unwrap_or("");
            if !handle.is_empty() {
                return Ok(Some(handle.to_string()));
            }
        }
        return Ok(None);
    }
    if let Some(handle) = input.strip_prefix('@') {
        if !handle.is_empty() {
            return Ok(Some(handle.to_string()));
        }
    }
    if !input.is_empty() {
        return Ok(Some(input.to_string()));
    }
    Ok(None)
}

pub(super) async fn resolve_channel_id_via_api(
    client: &reqwest::Client,
    input: &str,
    api_key: &str,
    api_base: &str,
) -> Result<String, SourceError> {
    if let Some(id) = extract_channel_id_from_input(input) {
        return Ok(id);
    }
    let Some(handle) = extract_handle_from_input(input)? else {
        return Err(SourceError::coded(400, "err.youtube_invalid_channel", [input]));
    };
    let url = format!(
        "{}/channels?part=id&forHandle={}&key={}",
        api_base, handle, api_key
    );
    let data = api_get_json_with_retry(client, &url).await?;
    let id = data
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|it| it.get("id"))
        .and_then(|id| id.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    id.ok_or_else(|| SourceError::coded(404, "err.youtube_channel_not_found", [input]))
}

pub(super) async fn verify_and_describe_channel_via_api(
    client: &reqwest::Client,
    channel_id: &str,
    api_key: &str,
    api_base: &str,
) -> Result<String, SourceError> {
    let url = format!(
        "{}/channels?part=snippet&id={}&key={}",
        api_base, channel_id, api_key
    );
    let data = api_get_json_with_retry(client, &url).await?;
    let title = data
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|it| it.get("snippet"))
        .and_then(|s| s.get("title"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    title.ok_or_else(|| SourceError::coded(404, "err.youtube_channel_not_found", [channel_id]))
}

async fn api_get_uploads_playlist_id(
    client: &reqwest::Client,
    channel_id: &str,
    api_key: &str,
    api_base: &str,
) -> Result<String, SourceError> {
    let url = format!(
        "{}/channels?part=contentDetails&id={}&key={}",
        api_base, channel_id, api_key
    );
    let data = api_get_json_with_retry(client, &url).await?;
    let uploads = data
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|it| it.get("contentDetails"))
        .and_then(|c| c.get("relatedPlaylists"))
        .and_then(|r| r.get("uploads"))
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    uploads.ok_or_else(|| SourceError::coded(404, "err.youtube_channel_not_found", [channel_id]))
}

async fn api_get_playlist_items_page(
    client: &reqwest::Client,
    playlist_id: &str,
    api_key: &str,
    page_token: Option<&str>,
    api_base: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), SourceError> {
    let mut url = format!(
        "{}/playlistItems?part=snippet&playlistId={}&maxResults={}&key={}",
        api_base, playlist_id, YT_API_PAGE_SIZE, api_key
    );
    if let Some(tok) = page_token {
        url.push_str(&format!("&pageToken={}", tok));
    }
    let data = api_get_json_with_retry(client, &url).await?;
    let items = data
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    let next = data
        .get("nextPageToken")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok((items, next))
}

fn best_thumbnail(thumbnails: &serde_json::Value) -> Option<String> {
    const KEYS: [&str; 5] = ["maxres", "standard", "high", "medium", "default"];
    KEYS.iter().find_map(|key| {
        thumbnails
            .get(*key)
            .and_then(|t| t.get("url"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    })
}

fn entry_from_api_item(item: &serde_json::Value, kind: FeedKind) -> Option<FeedEntry> {
    let snippet = item.get("snippet")?;
    let video_id = snippet
        .get("resourceId")?
        .get("videoId")?
        .as_str()?
        .to_string();
    let title = snippet
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let published = snippet
        .get("publishedAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if video_id.is_empty() || published.is_empty() {
        return None;
    }
    let description = snippet
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let thumbnail = snippet.get("thumbnails").and_then(best_thumbnail);
    Some(FeedEntry {
        video_id,
        title,
        published,
        description,
        thumbnail,
        kind,
        duration: None,
        view_count: None,
    })
}

#[derive(Debug, Clone, Default)]
struct VideoDetail {
    live: Option<String>,
    duration: Option<String>,
    view_count: Option<i64>,
    thumbnail: Option<String>,
}

async fn api_get_videos_details(
    client: &reqwest::Client,
    video_ids: &[String],
    api_key: &str,
    api_base: &str,
) -> Result<std::collections::HashMap<String, VideoDetail>, SourceError> {
    let mut map = std::collections::HashMap::new();
    for chunk in video_ids.chunks(YT_API_PAGE_SIZE) {
        let url = format!(
            "{}/videos?part=snippet,contentDetails,statistics&id={}&key={}",
            api_base,
            chunk.join(","),
            api_key
        );
        let data = api_get_json_with_retry(client, &url).await?;
        if let Some(items) = data.get("items").and_then(|i| i.as_array()) {
            for it in items {
                let Some(id) = it.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let mut detail = VideoDetail::default();
                if let Some(s) = it.get("snippet") {
                    detail.live = s
                        .get("liveBroadcastContent")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    detail.thumbnail = s.get("thumbnails").and_then(best_thumbnail);
                }
                if let Some(cd) = it.get("contentDetails") {
                    detail.duration = cd
                        .get("duration")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                }
                if let Some(st) = it.get("statistics") {
                    detail.view_count = st
                        .get("viewCount")
                        .and_then(|v| {
                            v.as_str()
                                .and_then(|s| s.parse::<i64>().ok())
                                .or_else(|| v.as_i64())
                        })
                        .filter(|&n| n > 0);
                }
                map.insert(id.to_string(), detail);
            }
        }
    }
    Ok(map)
}

fn apply_video_details(
    entries: &mut [FeedEntry],
    details: &std::collections::HashMap<String, VideoDetail>,
) {
    for e in entries.iter_mut() {
        if let Some(d) = details.get(&e.video_id) {
            if let Some(dur) = &d.duration {
                e.duration = Some(dur.clone());
            }
            if let Some(vc) = d.view_count {
                e.view_count = Some(vc);
            }
            if let Some(thumb) = &d.thumbnail {
                e.thumbnail = Some(thumb.clone());
            }
            if d.live.as_deref().map(|s| s != "none").unwrap_or(false) {
                e.kind = FeedKind::Live;
            }
        }
    }
}

pub(super) async fn fetch_via_api(
    client: &reqwest::Client,
    source: &Source,
    api_key: &str,
    max_count: Option<usize>,
    api_base: &str,
) -> Result<Vec<FeedEntry>, SourceError> {
    let config = SubscribeConfig::from_source(source);
    let kinds = config.kinds();
    if kinds.is_empty() {
        return Ok(vec![]);
    }
    let need_classify = kinds.len() == 1;

    let uploads = api_get_uploads_playlist_id(client, &source.owner, api_key, api_base).await?;

    let mut entries = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let (items, next) = api_get_playlist_items_page(
            client,
            &uploads,
            api_key,
            page_token.as_deref(),
            api_base,
        )
        .await?;
        for item in &items {
            if let Some(e) = entry_from_api_item(item, FeedKind::Video) {
                entries.push(e);
            }
        }
        if let Some(limit) = max_count {
            if entries.len() >= limit {
                break;
            }
        }
        match next {
            Some(tok) => page_token = Some(tok),
            None => break,
        }
    }

    let ids: Vec<String> = entries.iter().map(|e| e.video_id.clone()).collect();
    let details = api_get_videos_details(client, &ids, api_key, api_base).await?;
    if need_classify {
        let want_videos = config.videos;
        entries.retain(|e| {
            let live = details
                .get(&e.video_id)
                .and_then(|d| d.live.as_deref())
                .map(|s| s != "none")
                .unwrap_or(false);
            if want_videos {
                !live
            } else {
                live
            }
        });
    }
    apply_video_details(&mut entries, &details);

    if let Some(limit) = max_count {
        entries.truncate(limit);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::super::make_source;
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn api_item(video_id: &str, title: &str, published: &str, desc: &str) -> serde_json::Value {
        serde_json::json!({
            "snippet": {
                "publishedAt": published,
                "title": title,
                "description": desc,
                "thumbnails": {"medium": {"url": format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video_id)}},
                "resourceId": {"videoId": video_id}
            }
        })
    }

    #[test]
    fn test_best_thumbnail_prefers_highest_available() {
        let full = serde_json::json!({
            "default": {"url": "d.jpg"},
            "medium": {"url": "m.jpg"},
            "high": {"url": "h.jpg"},
            "standard": {"url": "s.jpg"},
            "maxres": {"url": "maxres.jpg"},
        });
        assert_eq!(best_thumbnail(&full).as_deref(), Some("maxres.jpg"));
        let partial = serde_json::json!({
            "default": {"url": "d.jpg"},
            "medium": {"url": "m.jpg"},
            "high": {"url": "h.jpg"},
        });
        assert_eq!(best_thumbnail(&partial).as_deref(), Some("h.jpg"));
        let blank = serde_json::json!({"maxres": {"url": ""}, "high": {"url": "h.jpg"}});
        assert_eq!(best_thumbnail(&blank).as_deref(), Some("h.jpg"));
        assert_eq!(best_thumbnail(&serde_json::json!({})), None);
    }

    #[test]
    fn test_entry_from_api_item_picks_highest_thumbnail() {
        let item = serde_json::json!({
            "snippet": {
                "publishedAt": "2024-01-01T00:00:00Z",
                "title": "T",
                "description": "",
                "thumbnails": {
                    "medium": {"url": "https://i.ytimg.com/vi/v1/mqdefault.jpg"},
                    "maxres": {"url": "https://i.ytimg.com/vi/v1/maxresdefault.jpg"},
                },
                "resourceId": {"videoId": "v1"}
            }
        });
        let e = entry_from_api_item(&item, FeedKind::Video).unwrap();
        assert_eq!(
            e.thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/v1/maxresdefault.jpg")
        );
    }

    #[test]
    fn test_apply_video_details_upgrades_thumbnail() {
        let mut entries = vec![FeedEntry {
            video_id: "v1".into(),
            title: "T".into(),
            published: "2024-01-01T00:00:00+00:00".into(),
            description: None,
            thumbnail: Some("https://i.ytimg.com/vi/v1/hqdefault.jpg".into()),
            kind: FeedKind::Video,
            duration: None,
            view_count: None,
        }];
        let mut details = std::collections::HashMap::new();
        details.insert(
            "v1".to_string(),
            VideoDetail {
                thumbnail: Some("https://i.ytimg.com/vi/v1/maxresdefault.jpg".into()),
                ..Default::default()
            },
        );
        apply_video_details(&mut entries, &details);
        assert_eq!(
            entries[0].thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/v1/maxresdefault.jpg")
        );
        let mut entries2 = entries.clone();
        let mut details2 = std::collections::HashMap::new();
        details2.insert("v1".to_string(), VideoDetail::default());
        apply_video_details(&mut entries2, &details2);
        assert_eq!(
            entries2[0].thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/v1/maxresdefault.jpg")
        );
    }

    #[test]
    fn test_map_api_error_key_invalid() {
        let body = r#"{"error":{"code":403,"message":"denied","errors":[{"reason":"ipRefererBlocked"}]}}"#;
        let err = map_api_error(403, body);
        assert_eq!(err.status(), 403);
        assert!(err.is_uncounted_failure());
        assert!(err.to_ipc_string().contains("err.youtube_api_key_invalid"));
    }

    #[test]
    fn test_map_api_error_quota() {
        let body = r#"{"error":{"code":403,"message":"quota","errors":[{"reason":"quotaExceeded"}]}}"#;
        let err = map_api_error(403, body);
        assert_eq!(err.status(), 403);
        assert!(err.is_uncounted_failure());
        assert!(err.to_ipc_string().contains("err.youtube_api_quota"));
    }

    #[test]
    fn test_map_api_error_plain() {
        let err = map_api_error(500, "boom");
        assert_eq!(err.status(), 500);
        assert!(err.to_ipc_string().contains("err.api_error|500"));
    }

    #[test]
    fn test_extract_handle_from_input_variants() {
        assert_eq!(extract_handle_from_input("@handle").unwrap(), Some("handle".to_string()));
        assert_eq!(extract_handle_from_input("handle").unwrap(), Some("handle".to_string()));
        assert_eq!(
            extract_handle_from_input("https://www.youtube.com/@handle").unwrap(),
            Some("handle".to_string())
        );
        assert_eq!(
            extract_handle_from_input("https://m.youtube.com/@h?x=1").unwrap(),
            Some("h".to_string())
        );
        assert_eq!(
            extract_handle_from_input("https://www.youtube.com/channel/UCabc123").unwrap(),
            None
        );
        assert_eq!(
            extract_handle_from_input("https://www.youtube.com/c/name").unwrap(),
            None
        );
        assert!(extract_handle_from_input("https://evil.example.com/@x").is_err());
    }

    #[test]
    fn test_entry_from_api_item_basic() {
        let item = api_item("aaa111", "Hello", "2024-01-01T00:00:00Z", "desc");
        let e = entry_from_api_item(&item, FeedKind::Video).unwrap();
        assert_eq!(e.video_id, "aaa111");
        assert_eq!(e.title, "Hello");
        assert_eq!(e.published, "2024-01-01T00:00:00Z");
        assert_eq!(e.description.as_deref(), Some("desc"));
        assert_eq!(
            e.thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/aaa111/mqdefault.jpg")
        );
        assert_eq!(e.kind, FeedKind::Video);
    }

    #[test]
    fn test_entry_from_api_item_missing_id_skipped() {
        let item = serde_json::json!({"snippet": {"title": "no id"}});
        assert!(entry_from_api_item(&item, FeedKind::Video).is_none());
    }

    #[tokio::test]
    async fn test_resolve_channel_id_via_api_handle() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .and(query_param("forHandle", "somehandle"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "UCResolvedFromApi12345678"}]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let id = resolve_channel_id_via_api(&client, "@somehandle", "test-key", &mock.uri())
            .await
            .unwrap();
        assert_eq!(id, "UCResolvedFromApi12345678");
    }

    #[tokio::test]
    async fn test_resolve_channel_id_via_api_direct_id_no_http() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let id = resolve_channel_id_via_api(&client, "UCabcdefghijklmnopqrst", "k", "http://unused")
            .await
            .unwrap();
        assert_eq!(id, "UCabcdefghijklmnopqrst");
    }

    #[tokio::test]
    async fn test_verify_and_describe_channel_via_api() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .and(query_param("id", "UCabc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"snippet": {"title": "Test Channel"}}]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let title = verify_and_describe_channel_via_api(&client, "UCabc123", "k", &mock.uri())
            .await
            .unwrap();
        assert_eq!(title, "Test Channel");
    }

    #[tokio::test]
    async fn test_verify_and_describe_channel_via_api_not_found() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": []
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = verify_and_describe_channel_via_api(&client, "UCabc123", "k", &mock.uri())
            .await
            .unwrap_err();
        assert_eq!(err.status(), 404);
        assert!(err.to_ipc_string().contains("err.youtube_channel_not_found"));
    }

    #[tokio::test]
    async fn test_fetch_via_api_basic() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .and(query_param("id", "UCabc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"contentDetails": {"relatedPlaylists": {"uploads": "UUabc123"}}}]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlistItems"))
            .and(query_param("playlistId", "UUabc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    api_item("v3", "Three", "2024-03-01T00:00:00Z", "d3"),
                    api_item("v2", "Two", "2024-02-01T00:00:00Z", "d2"),
                ]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "v3", "snippet": {"liveBroadcastContent": "none", "thumbnails": {"maxres": {"url": "https://i.ytimg.com/vi/v3/maxresdefault.jpg"}}}, "contentDetails": {"duration": "PT3M21S"}, "statistics": {"viewCount": "123456"}},
                    {"id": "v2", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT1H2M3S"}, "statistics": {"viewCount": "987654"}},
                ]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = make_source(Some(r#"{"videos":true,"live":true,"posts":false}"#));
        let entries = fetch_via_api(&client, &source, "test-key", Some(10), &mock.uri())
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].video_id, "v3");
        assert_eq!(entries[0].kind, FeedKind::Video);
        assert_eq!(entries[0].duration.as_deref(), Some("PT3M21S"));
        assert_eq!(entries[0].view_count, Some(123456));
        assert_eq!(entries[1].view_count, Some(987654));
        assert_eq!(entries[1].duration.as_deref(), Some("PT1H2M3S"));
        assert_eq!(
            entries[1].thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/v2/mqdefault.jpg")
        );
        assert_eq!(
            entries[0].thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/v3/maxresdefault.jpg")
        );
        let meta = serde_json::from_str::<serde_json::Value>(
            &entries[0]
                .clone()
                .into_fetched_release()
                .extra
                .to_metadata_json()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(meta["duration"], "PT3M21S");
        assert_eq!(meta["view_count"], 123456);
    }

    #[tokio::test]
    async fn test_fetch_via_api_classify_videos_only() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"contentDetails": {"relatedPlaylists": {"uploads": "UUabc"}}}]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlistItems"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    api_item("live1", "L", "2024-01-01T00:00:00Z", ""),
                    api_item("vid1", "V", "2024-01-02T00:00:00Z", ""),
                ]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "live1", "snippet": {"liveBroadcastContent": "completed"}, "contentDetails": {"duration": "PT2H0M0S"}},
                    {"id": "vid1", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT5M30S"}},
                ]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = make_source(Some(r#"{"videos":true,"live":false,"posts":false}"#));
        let entries = fetch_via_api(&client, &source, "k", Some(10), &mock.uri())
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].video_id, "vid1");
        assert_eq!(entries[0].kind, FeedKind::Video);
        assert_eq!(entries[0].duration.as_deref(), Some("PT5M30S"));
    }

    #[tokio::test]
    async fn test_fetch_via_api_classify_live_only() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"contentDetails": {"relatedPlaylists": {"uploads": "UUabc"}}}]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlistItems"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    api_item("live1", "L", "2024-01-01T00:00:00Z", ""),
                    api_item("vid1", "V", "2024-01-02T00:00:00Z", ""),
                ]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "live1", "snippet": {"liveBroadcastContent": "completed"}, "contentDetails": {"duration": "PT2H0M0S"}},
                    {"id": "vid1", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT5M30S"}},
                ]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = make_source(Some(r#"{"videos":false,"live":true,"posts":false}"#));
        let entries = fetch_via_api(&client, &source, "k", Some(10), &mock.uri())
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].video_id, "live1");
        assert_eq!(entries[0].kind, FeedKind::Live);
        assert_eq!(entries[0].duration.as_deref(), Some("PT2H0M0S"));
    }
}
