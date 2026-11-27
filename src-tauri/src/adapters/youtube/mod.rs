mod api;
mod resolve;
mod rss;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::releases;
use crate::db::sources::Source;
use crate::error::SourceError;
use crate::source::{FetchedRelease, SourceAdapter, SourceExtraMetadata};

pub(super) const YT_FEED_BASE: &str = "https://www.youtube.com/feeds/videos.xml";
pub(super) const YT_WATCH_BASE: &str = "https://www.youtube.com/watch";
pub(super) const YT_API_BASE: &str = "https://youtube.googleapis.com/youtube/v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum FeedKind {
    Video,
    Live,
}

impl FeedKind {
    pub(super) fn playlist_prefix(self) -> &'static str {
        match self {
            FeedKind::Video => "UULF",
            FeedKind::Live => "UULV",
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            FeedKind::Video => "video",
            FeedKind::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SubscribeConfig {
    pub(super) videos: bool,
    pub(super) live: bool,
    pub(super) posts: bool,
}

impl Default for SubscribeConfig {
    fn default() -> Self {
        Self {
            videos: true,
            live: true,
            posts: false,
        }
    }
}

impl SubscribeConfig {
    pub(super) fn from_source(source: &Source) -> Self {
        let default = Self::default();
        let Some(raw) = source.config.as_deref() else {
            return default;
        };
        match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(v) => Self {
                videos: v["videos"].as_bool().unwrap_or(default.videos),
                live: v["live"].as_bool().unwrap_or(default.live),
                posts: v["posts"].as_bool().unwrap_or(false),
            },
            Err(_) => default,
        }
    }

    pub(super) fn kinds(&self) -> Vec<FeedKind> {
        let mut kinds = Vec::new();
        if self.videos {
            kinds.push(FeedKind::Video);
        }
        if self.live {
            kinds.push(FeedKind::Live);
        }
        kinds
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FeedEntry {
    pub(super) video_id: String,
    pub(super) title: String,
    pub(super) published: String,
    pub(super) description: Option<String>,
    pub(super) thumbnail: Option<String>,
    pub(super) kind: FeedKind,
    pub(super) duration: Option<String>,
    pub(super) view_count: Option<i64>,
}

impl FeedEntry {
    pub(super) fn html_url(&self) -> String {
        format!("{}/?v={}", YT_WATCH_BASE, self.video_id)
    }

    pub(super) fn into_fetched_release(self) -> FetchedRelease {
        let html_url = self.html_url();
        FetchedRelease {
            tag: self.video_id,
            name: self.title,
            html_url,
            published: self.published,
            prerelease: false,
            body: self.description,
            extra: SourceExtraMetadata::Video {
                kind: self.kind.as_str().to_string(),
                thumbnail: self.thumbnail,
                duration: self.duration,
                view_count: self.view_count,
            },
        }
    }
}

pub struct YoutubeAdapter;

#[async_trait::async_trait]
impl SourceAdapter for YoutubeAdapter {
    fn source_type(&self) -> &'static str {
        "youtube"
    }

    fn auth_kind(&self) -> crate::source::AuthKind {
        crate::source::AuthKind::YouTubeApiKey
    }

    fn always_fetch_history(&self) -> bool {
        true
    }

    fn ai_eligible(&self) -> bool {
        false
    }

    fn notification_source_name(
        &self,
        owner: &str,
        _repo: &str,
        description: Option<&str>,
    ) -> String {
        let d = description.unwrap_or("").trim();
        let d = d.strip_prefix("YouTube channel: ").unwrap_or(d);
        if d.is_empty() {
            owner.to_string()
        } else {
            d.to_string()
        }
    }

    fn notification_source_label(&self) -> Option<&'static str> {
        Some("YouTube")
    }

    fn notification_show_tag(&self) -> bool {
        false
    }

    fn log_source_ident(
        &self,
        owner: &str,
        _repo: &str,
        description: Option<&str>,
    ) -> (String, String) {
        let name = description.filter(|d| !d.is_empty()).unwrap_or(owner);
        (name.to_string(), String::new())
    }

    fn log_release_tag<'a>(&self, _tag_name: &'a str, release_name: &'a str) -> &'a str {
        release_name
    }

    fn release_body_label(&self) -> &'static str {
        "视频简介"
    }

    fn refresh_description_after_check(&self) -> bool {
        true
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            let entries = api::fetch_via_api(client, source, key, Some(per_page), YT_API_BASE).await?;
            return Ok(entries.into_iter().map(FeedEntry::into_fetched_release).collect());
        }
        let kinds = SubscribeConfig::from_source(source).kinds();
        if kinds.is_empty() {
            return Ok(vec![]);
        }
        let entries =
            rss::aggregate_kinds(&kinds, |kind| rss::fetch_feed_entries(client, &source.owner, kind))
                .await?;
        Ok(entries
            .into_iter()
            .take(per_page)
            .map(FeedEntry::into_fetched_release)
            .collect())
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            let entries = api::fetch_via_api(client, source, key, max_count, YT_API_BASE).await?;
            return Ok(entries.into_iter().map(FeedEntry::into_fetched_release).collect());
        }
        self.fetch(client, source, usize::MAX, token).await
    }

    async fn save(
        &self,
        db: &r2d2::Pool<crate::db::SqliteConnectionManager>,
        source: &Source,
        data: &[FetchedRelease],
        max_count: usize,
        _client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
        let db = db.clone();
        let source_id = source.id;
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = match db.get() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("err.db_lock|{}", e);
                    return vec![];
                }
            };
            save_entries(&conn, source_id, &data, max_count)
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("youtube save spawn_blocking panic: {}", e);
            vec![]
        })
    }

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        _repo: &str,
        token: Option<&str>,
    ) -> Result<String, SourceError> {
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            return api::verify_and_describe_channel_via_api(client, owner, key, YT_API_BASE).await;
        }
        resolve::verify_and_describe_channel(client, owner).await
    }

    async fn resolve_owner(
        &self,
        client: &reqwest::Client,
        owner: &str,
        token: Option<&str>,
    ) -> Result<String, SourceError> {
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            if let Ok(id) = api::resolve_channel_id_via_api(client, owner, key, YT_API_BASE).await {
                return Ok(id);
            }
        }
        resolve::resolve_channel_id(client, owner).await
    }
}

pub fn save_entries(
    conn: &Connection,
    source_id: i64,
    items: &[FetchedRelease],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    let entries: Vec<crate::db::save::SaveEntry> = items
        .iter()
        .filter(|e| !e.tag.is_empty() && !e.published.is_empty())
        .map(FetchedRelease::to_save_entry)
        .collect();
    crate::db::save::save_entries_generic(
        conn,
        source_id,
        &entries,
        max_count,
        |conn, id, entry| {
            let _ = releases::set_release_body_and_metadata(
                conn,
                id,
                entry.body.as_deref(),
                entry.metadata.as_deref(),
            );
        },
        |conn, source_id, entry| {
            let _ = releases::update_release_metadata(
                conn,
                source_id,
                &entry.tag,
                entry.body.as_deref(),
                entry.metadata.as_deref(),
            );
        },
    )
}

#[cfg(test)]
pub(super) fn make_source(config: Option<&str>) -> Source {
    Source {
        id: 1,
        source_type: "youtube".into(),
        owner: "UCabc123".into(),
        repo: String::new(),
        poll_interval_minutes: 30,
        enabled: true,
        last_checked_at: None,
        last_check_status: "unknown".into(),
        last_check_message: None,
        consecutive_failures: 0,
        last_new_count: 0,
        muted: false,
        created_at: String::new(),
        updated_at: String::new(),
        description: None,
        config: config.map(|s| s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn test_notification_source_name_uses_channel_name() {
        let adapter = YoutubeAdapter;
        assert_eq!(
            adapter.notification_source_name("UCabc123", "", Some("Test Channel")),
            "Test Channel"
        );
        assert_eq!(
            adapter.notification_source_name("UCabc123", "", Some("YouTube channel: Test Channel")),
            "Test Channel"
        );
        assert_eq!(
            adapter.notification_source_name("UCabc123", "", None),
            "UCabc123"
        );
        assert_eq!(
            adapter.notification_source_name("UCabc123", "", Some("   ")),
            "UCabc123"
        );
    }

    #[test]
    fn test_notification_show_tag_false_for_video() {
        assert!(!YoutubeAdapter.notification_show_tag());
    }

    #[test]
    fn test_notification_source_label() {
        assert_eq!(YoutubeAdapter.notification_source_label(), Some("YouTube"));
    }

    #[test]
    fn test_subscribe_config_default() {
        let s = make_source(None);
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos);
        assert!(cfg.live);
        assert!(!cfg.posts);
        assert_eq!(cfg.kinds(), vec![FeedKind::Video, FeedKind::Live]);
    }

    #[test]
    fn test_subscribe_config_partial() {
        let s = make_source(Some(r#"{"videos":true,"live":false,"posts":false}"#));
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos);
        assert!(!cfg.live);
        assert_eq!(cfg.kinds(), vec![FeedKind::Video]);
    }

    #[test]
    fn test_subscribe_config_none_subscribed() {
        let s = make_source(Some(r#"{"videos":false,"live":false,"posts":false}"#));
        assert!(SubscribeConfig::from_source(&s).kinds().is_empty());
    }

    #[test]
    fn test_subscribe_config_invalid_json_falls_back() {
        let s = make_source(Some("not-json"));
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos && cfg.live);
    }

    #[test]
    fn test_subscribe_config_missing_fields_default() {
        let s = make_source(Some(r#"{"posts":true}"#));
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos);
        assert!(cfg.live);
        assert!(cfg.posts);
    }

    fn entry_release(id: &str, title: &str, published: &str) -> FetchedRelease {
        FeedEntry {
            video_id: id.to_string(),
            title: title.to_string(),
            published: published.to_string(),
            description: Some(format!("desc-{}", id)),
            thumbnail: Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id)),
            kind: FeedKind::Video,
            duration: None,
            view_count: None,
        }
        .into_fetched_release()
    }

    #[test]
    fn test_save_entries_max_count_1_saves_latest() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_release("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_release("v2", "Two", "2024-02-01T00:00:00+00:00"),
        ];
        let saved = save_entries(&conn, sid, &data, 1);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.as_deref(), Some("desc-v3"));
        let rel = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(rel.len(), 1);
        assert_eq!(rel[0].tag_name, "v3");
        assert_eq!(rel[0].release_name, "Three");
        assert_eq!(rel[0].html_url, "https://www.youtube.com/watch/?v=v3");
        assert_eq!(rel[0].published_at, "2024-03-01T00:00:00+00:00");
        assert!(!rel[0].prerelease);
        let meta = rel[0].extra_metadata.as_ref().unwrap();
        assert!(meta.contains("\"kind\":\"video\""));
        assert!(meta.contains("hqdefault.jpg"));
    }

    #[test]
    fn test_save_entries_write_cap_backfills_older_unsaved_video() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_release("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_release("v2", "Two", "2024-02-01T00:00:00+00:00"),
        ];
        assert_eq!(save_entries(&conn, sid, &data, 1).len(), 1);
        assert_eq!(save_entries(&conn, sid, &data, 1).len(), 1);
        let mut tags: Vec<String> = db::releases::get_releases_with_state(&conn)
            .unwrap()
            .into_iter()
            .map(|r| r.tag_name)
            .collect();
        tags.sort();
        assert_eq!(tags, vec!["v2".to_string(), "v3".to_string()]);
    }

    #[test]
    fn test_save_entries_historical_skips_existing() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_release("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_release("v2", "Two", "2024-02-01T00:00:00+00:00"),
            entry_release("v1", "One", "2024-01-01T00:00:00+00:00"),
        ];
        assert_eq!(save_entries(&conn, sid, &data, 1).len(), 1);
        let saved = save_entries(&conn, sid, &data, 2);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].1.as_deref(), Some("desc-v2"));
        assert_eq!(saved[1].1.as_deref(), Some("desc-v1"));
    }

    #[test]
    fn test_save_entries_sorts_by_published_desc() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_release("v1", "One", "2024-01-01T00:00:00+00:00"),
            entry_release("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_release("v2", "Two", "2024-02-01T00:00:00+00:00"),
        ];
        let saved = save_entries(&conn, sid, &data, 3);
        assert_eq!(saved.len(), 3);
        let rel = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(rel[0].tag_name, "v3");
        assert_eq!(rel[1].tag_name, "v2");
        assert_eq!(rel[2].tag_name, "v1");
    }

    #[test]
    fn test_save_entries_skips_empty_video_id() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_release("", "NoId", "2024-01-01T00:00:00+00:00"),
            entry_release("v1", "One", "2024-01-01T00:00:00+00:00"),
        ];
        let saved = save_entries(&conn, sid, &data, 2);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.as_deref(), Some("desc-v1"));
    }
}
