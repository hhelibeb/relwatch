use crate::error::SourceError;
use super::{FeedEntry, FeedKind, YT_FEED_BASE};

pub(super) struct FeedParseResult {
    pub(super) channel_title: String,
    pub(super) entries: Vec<FeedEntry>,
}

#[derive(Clone, Copy, PartialEq)]
enum TextTarget {
    None,
    FeedTitle,
    EntryTitle,
    VideoId,
    Published,
    Description,
}

fn unescape_text(decoded: &str, is_ref: bool) -> Result<String, String> {
    let raw = if is_ref {
        format!("&{};", decoded)
    } else {
        decoded.to_string()
    };
    Ok(quick_xml::escape::unescape(&raw)
        .map_err(|e| format!("err.rss_parse|{}", e))?
        .into_owned())
}

fn apply_text_content(
    channel_title: &mut String,
    cur: &mut Option<FeedEntry>,
    target: TextTarget,
    text: &str,
) {
    match target {
        TextTarget::FeedTitle => *channel_title = text.trim().to_string(),
        TextTarget::EntryTitle => {
            if let Some(entry) = cur.as_mut() {
                entry.title.push_str(text);
            }
        }
        TextTarget::VideoId => {
            if let Some(entry) = cur.as_mut() {
                entry.video_id.push_str(text);
            }
        }
        TextTarget::Published => {
            if let Some(entry) = cur.as_mut() {
                entry.published.push_str(text);
            }
        }
        TextTarget::Description => {
            if let Some(entry) = cur.as_mut() {
                if !text.trim().is_empty() {
                    entry.description.get_or_insert_with(String::new).push_str(text);
                }
            }
        }
        TextTarget::None => {}
    }
}

pub(super) fn parse_feed(xml: &str, kind: FeedKind) -> Result<FeedParseResult, String> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf: Vec<u8> = Vec::new();

    let mut channel_title = String::new();
    let mut entries: Vec<FeedEntry> = Vec::new();

    let mut in_entry = false;
    let mut in_media_group = false;
    let mut target = TextTarget::None;
    let mut cur: Option<FeedEntry> = None;

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("err.rss_parse|{}", e))?;
        match ev {
            Event::Start(e) => match e.local_name().as_ref() {
                b"entry" if !in_entry => {
                    in_entry = true;
                    target = TextTarget::None;
                    cur = Some(FeedEntry {
                        video_id: String::new(),
                        title: String::new(),
                        published: String::new(),
                        description: None,
                        thumbnail: None,
                        kind,
                        duration: None,
                        view_count: None,
                    });
                }
                b"group" if in_entry => in_media_group = true,
                b"title" if in_entry && !in_media_group => target = TextTarget::EntryTitle,
                b"title" if !in_entry => target = TextTarget::FeedTitle,
                b"videoId" if in_entry => target = TextTarget::VideoId,
                b"published" if in_entry => target = TextTarget::Published,
                b"description" if in_media_group => target = TextTarget::Description,
                _ => {}
            },
            Event::Empty(e) if in_media_group && e.local_name().as_ref() == b"thumbnail" => {
                if let Some(entry) = cur.as_mut() {
                    for attr in e.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"url" {
                            if let Ok(v) =
                                attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            {
                                entry.thumbnail = Some(v.into_owned());
                            }
                        }
                    }
                }
            }
            Event::Text(t) => {
                let decoded = t.decode().map_err(|e| format!("err.rss_parse|{}", e))?;
                let text = quick_xml::escape::unescape(&decoded)
                    .map_err(|e| format!("err.rss_parse|{}", e))?;
                apply_text_content(&mut channel_title, &mut cur, target, &text);
            }
            Event::GeneralRef(t) => {
                let decoded = t.decode().map_err(|e| format!("err.rss_parse|{}", e))?;
                let text = unescape_text(&decoded, true)?;
                apply_text_content(&mut channel_title, &mut cur, target, &text);
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"entry" if in_entry => {
                    if let Some(mut entry) = cur.take() {
                        entry.video_id = entry.video_id.trim().to_string();
                        entry.published = entry.published.trim().to_string();
                        entry.title = entry.title.trim().to_string();
                        if let Some(d) = entry.description.as_mut() {
                            let trimmed = d.trim().to_string();
                            if trimmed.is_empty() {
                                entry.description = None;
                            } else {
                                *d = trimmed;
                            }
                        }
                        if !entry.video_id.is_empty() && !entry.published.is_empty() {
                            entries.push(entry);
                        }
                    }
                    in_entry = false;
                    target = TextTarget::None;
                }
                b"group" if in_media_group => in_media_group = false,
                b"title" | b"videoId" | b"published" | b"description" => {
                    target = TextTarget::None;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(FeedParseResult {
        channel_title,
        entries,
    })
}

pub(super) fn feed_url(channel_id: &str, kind: FeedKind) -> Result<String, SourceError> {
    let prefix = kind.playlist_prefix();
    let tail = channel_id
        .strip_prefix("UC")
        .ok_or_else(|| SourceError::coded(0, "err.youtube_invalid_channel", [channel_id]))?;
    Ok(format!("{}?playlist_id={}{}", YT_FEED_BASE, prefix, tail))
}

pub(super) async fn fetch_feed_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, SourceError> {
    crate::http::fetch_text_with_retry(client, url).await
}

pub(super) async fn fetch_feed_entries(
    client: &reqwest::Client,
    channel_id: &str,
    kind: FeedKind,
) -> Result<Vec<FeedEntry>, SourceError> {
    let url = feed_url(channel_id, kind)?;
    let xml = fetch_feed_with_retry(client, &url).await?;
    let parsed = parse_feed(&xml, kind).map_err(|e| SourceError::coded(0, "err.parse_failed", [e]))?;
    Ok(parsed.entries)
}

pub(super) async fn aggregate_kinds<F, Fut>(
    kinds: &[FeedKind],
    mut fetch_one: F,
) -> Result<Vec<FeedEntry>, SourceError>
where
    F: FnMut(FeedKind) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<FeedEntry>, SourceError>>,
{
    let mut out = Vec::new();
    let mut last_err: Option<SourceError> = None;
    for &kind in kinds {
        match fetch_one(kind).await {
            Ok(entries) => out.extend(entries),
            Err(e) if e.status() == 404 => {
                log::warn!(
                    "YouTube {} feed 不存在（频道可能没有该类型内容）",
                    kind.as_str()
                );
            }
            Err(e) => last_err = Some(e),
        }
    }
    if out.is_empty() {
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_feed(videos: &[(&str, &str, &str, Option<&str>)]) -> String {
        let mut xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
  <title>Test Channel</title>
  <link rel="self" href="https://www.youtube.com/feeds/videos.xml?channel_id=UCabc"/>
"#,
        );
        for (id, title, published, desc) in videos {
            xml.push_str(&format!(
                r#"  <entry>
    <id>yt:video:{}</id>
    <yt:videoId>{}</yt:videoId>
    <yt:channelId>UCabc</yt:channelId>
    <title>{}</title>
    <link rel="alternate" href="https://www.youtube.com/watch?v={}"/>
    <published>{}</published>
    <media:group>
      <media:title>{}</media:title>
      <media:thumbnail url="https://i.ytimg.com/vi/{}/hqdefault.jpg" width="480" height="360"/>
      <media:description>{}</media:description>
    </media:group>
  </entry>
"#,
                id,
                id,
                title,
                id,
                published,
                title,
                id,
                desc.unwrap_or(""),
            ));
        }
        xml.push_str("</feed>");
        xml
    }

    #[test]
    fn test_feed_url_video_prefix() {
        let url = feed_url("UCabcdefghijklmnopqrst", FeedKind::Video).unwrap();
        assert_eq!(
            url,
            "https://www.youtube.com/feeds/videos.xml?playlist_id=UULFabcdefghijklmnopqrst"
        );
    }

    #[test]
    fn test_feed_url_live_prefix() {
        let url = feed_url("UCabcdefghijklmnopqrst", FeedKind::Live).unwrap();
        assert_eq!(
            url,
            "https://www.youtube.com/feeds/videos.xml?playlist_id=UULVabcdefghijklmnopqrst"
        );
    }

    #[test]
    fn test_feed_url_invalid_channel() {
        let err = feed_url("XXabcdefghijklmnopqrst", FeedKind::Video).unwrap_err();
        assert_eq!(err.status(), 0);
        assert!(err.to_ipc_string().contains("err.youtube_invalid_channel"));
    }

    #[tokio::test]
    async fn test_aggregate_kinds_skips_404_kind() {
        let kinds = vec![FeedKind::Video, FeedKind::Live];
        let result = aggregate_kinds(&kinds, |kind| async move {
            match kind {
                FeedKind::Video => Ok(vec![FeedEntry {
                    video_id: "v1".into(),
                    title: "V".into(),
                    published: "2024-01-01T00:00:00+00:00".into(),
                    description: None,
                    thumbnail: None,
                    kind,
                    duration: None,
                    view_count: None,
                }]),
                FeedKind::Live => Err(SourceError::api_error(404, "Not Found")),
            }
        })
        .await;
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].video_id, "v1");
    }

    #[tokio::test]
    async fn test_aggregate_kinds_all_404_returns_empty() {
        let kinds = vec![FeedKind::Video, FeedKind::Live];
        let result =
            aggregate_kinds(&kinds, |_| async { Err(SourceError::api_error(404, "not found")) })
                .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_aggregate_kinds_propagates_real_error() {
        let kinds = vec![FeedKind::Video, FeedKind::Live];
        let result = aggregate_kinds(&kinds, |kind| async move {
            match kind {
                FeedKind::Video => Err(SourceError::api_error(500, "Internal Server Error")),
                FeedKind::Live => Err(SourceError::api_error(404, "not found")),
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 500);
    }

    #[test]
    fn test_parse_feed_basic() {
        let xml = sample_feed(&[(
            "aaa111",
            "Hello World",
            "2024-01-01T00:00:00+00:00",
            Some("视频描述"),
        )]);
        let parsed = parse_feed(&xml, FeedKind::Video).unwrap();
        assert_eq!(parsed.channel_title, "Test Channel");
        assert_eq!(parsed.entries.len(), 1);
        let e = &parsed.entries[0];
        assert_eq!(e.video_id, "aaa111");
        assert_eq!(e.title, "Hello World");
        assert_eq!(e.published, "2024-01-01T00:00:00+00:00");
        assert_eq!(e.description.as_deref(), Some("视频描述"));
        assert_eq!(
            e.thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/aaa111/hqdefault.jpg")
        );
        assert_eq!(e.kind, FeedKind::Video);
        assert_eq!(e.html_url(), "https://www.youtube.com/watch/?v=aaa111");
        let meta = serde_json::from_str::<serde_json::Value>(
            &e.clone().into_fetched_release().extra.to_metadata_json().unwrap(),
        )
        .unwrap();
        assert_eq!(meta["kind"], "video");
        assert_eq!(meta["thumbnail"], "https://i.ytimg.com/vi/aaa111/hqdefault.jpg");
    }

    #[test]
    fn test_parse_feed_multiple_entries_and_live_kind() {
        let xml = sample_feed(&[
            ("vid2", "Second", "2024-01-02T00:00:00+00:00", Some("b")),
            ("vid1", "First", "2024-01-01T00:00:00+00:00", None),
        ]);
        let parsed = parse_feed(&xml, FeedKind::Live).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].video_id, "vid2");
        assert_eq!(parsed.entries[1].video_id, "vid1");
        assert!(parsed.entries[1].description.is_none());
        assert_eq!(parsed.entries[0].kind, FeedKind::Live);
        let meta = serde_json::from_str::<serde_json::Value>(
            &parsed.entries[0]
                .clone()
                .into_fetched_release()
                .extra
                .to_metadata_json()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(meta["kind"], "live");
    }

    #[test]
    fn test_parse_feed_unescapes_entities() {
        let xml = sample_feed(&[(
            "v1",
            "A &amp; B &lt;tag&gt;",
            "2024-01-01T00:00:00+00:00",
            Some("x &amp; y"),
        )]);
        let parsed = parse_feed(&xml, FeedKind::Video).unwrap();
        assert_eq!(parsed.entries[0].title, "A & B <tag>");
        assert_eq!(parsed.entries[0].description.as_deref(), Some("x & y"));
    }

    #[test]
    fn test_parse_feed_missing_video_id_skipped() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Chan</title>
  <entry>
    <title>No ID</title>
    <published>2024-01-01T00:00:00+00:00</published>
  </entry>
</feed>"#;
        let parsed = parse_feed(xml, FeedKind::Video).unwrap();
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn test_parse_feed_malformed_returns_error() {
        assert!(parse_feed("<feed><entry></feed>", FeedKind::Video).is_err());
    }

    #[tokio::test]
    async fn test_fetch_feed_with_retry_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/feeds/videos.xml"))
            .and(query_param("playlist_id", "UULFabcdefghijklmnopqrst"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sample_feed(&[(
                "aaa111",
                "Hello",
                "2024-01-01T00:00:00+00:00",
                Some("d"),
            )])))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let xml = fetch_feed_with_retry(
            &client,
            &format!("{}/feeds/videos.xml?playlist_id=UULFabcdefghijklmnopqrst", mock.uri()),
        )
        .await
        .unwrap();
        let parsed = parse_feed(&xml, FeedKind::Video).unwrap();
        assert_eq!(parsed.entries.len(), 1);
    }
}
