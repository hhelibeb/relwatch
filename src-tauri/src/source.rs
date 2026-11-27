use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::db::save::SaveEntry;
use crate::db::sources::Source;
use crate::error::SourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthKind {
    None,
    GitHubToken,
    YouTubeApiKey,
    BilibiliCookie,
}

impl AuthKind {
    pub const ALL: &'static [AuthKind] = &[
        AuthKind::GitHubToken,
        AuthKind::YouTubeApiKey,
        AuthKind::BilibiliCookie,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            AuthKind::None => "none",
            AuthKind::GitHubToken => "github_token",
            AuthKind::YouTubeApiKey => "youtube_api_key",
            AuthKind::BilibiliCookie => "bilibili_cookie",
        }
    }

    pub fn setting_key(self) -> Option<&'static str> {
        match self {
            AuthKind::None => None,
            AuthKind::GitHubToken => Some(crate::db::settings::KEY_GITHUB_TOKEN),
            AuthKind::YouTubeApiKey => Some(crate::db::settings::KEY_YOUTUBE_API_KEY),
            AuthKind::BilibiliCookie => Some(crate::db::settings::KEY_BILIBILI_COOKIE),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SourceCredentials {
    tokens: HashMap<AuthKind, String>,
}

impl SourceCredentials {
    pub fn load(conn: &rusqlite::Connection) -> Self {
        let mut tokens = HashMap::new();
        for &kind in AuthKind::ALL {
            if let Some(key) = kind.setting_key() {
                if let Some(val) = crate::credential::read_credential(conn, key) {
                    tokens.insert(kind, val);
                }
            }
        }
        Self { tokens }
    }

    pub fn with_token(mut self, kind: AuthKind, token: impl Into<String>) -> Self {
        if kind != AuthKind::None {
            self.tokens.insert(kind, token.into());
        }
        self
    }

    pub fn load_for(conn: &rusqlite::Connection, adapter: &dyn SourceAdapter) -> Option<String> {
        adapter
            .auth_kind()
            .setting_key()
            .and_then(|key| crate::credential::read_credential(conn, key))
    }

    pub fn get(&self, kind: AuthKind) -> Option<&str> {
        if kind == AuthKind::None {
            None
        } else {
            self.tokens.get(&kind).map(String::as_str)
        }
    }

    pub fn token_for(&self, adapter: &dyn SourceAdapter) -> Option<&str> {
        self.get(adapter.auth_kind())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceExtraMetadata {
    None,
    HuggingFace {
        pipeline_tag: Option<String>,
        downloads: i64,
        likes: i64,
        library_name: Option<String>,
        tags: Vec<String>,
        private: bool,
        gated: bool,
    },
    Video {
        kind: String,
        thumbnail: Option<String>,
        duration: Option<String>,
        view_count: Option<i64>,
    },
}

impl SourceExtraMetadata {
    pub fn to_metadata_json(&self) -> Option<String> {
        match self {
            SourceExtraMetadata::None => None,
            SourceExtraMetadata::HuggingFace {
                pipeline_tag,
                downloads,
                likes,
                library_name,
                tags,
                private,
                gated,
            } => Some(
                serde_json::json!({
                    "pipeline_tag": pipeline_tag,
                    "downloads": downloads,
                    "likes": likes,
                    "library_name": library_name,
                    "tags": tags,
                    "private": private,
                    "gated": gated,
                })
                .to_string(),
            ),
            SourceExtraMetadata::Video {
                kind,
                thumbnail,
                duration,
                view_count,
            } => Some(
                serde_json::json!({
                    "kind": kind,
                    "thumbnail": thumbnail,
                    "duration": duration,
                    "view_count": view_count,
                })
                .to_string(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedRelease {
    pub tag: String,
    pub name: String,
    pub html_url: String,
    pub published: String,
    pub prerelease: bool,
    pub body: Option<String>,
    pub extra: SourceExtraMetadata,
}

impl FetchedRelease {
    pub fn to_save_entry(&self) -> SaveEntry {
        SaveEntry {
            tag: self.tag.clone(),
            name: self.name.clone(),
            html_url: self.html_url.clone(),
            published: self.published.clone(),
            prerelease: self.prerelease,
            body: self.body.clone(),
            metadata: self.extra.to_metadata_json(),
        }
    }
}

type AdapterFactory = fn() -> Box<dyn SourceAdapter>;

static ADAPTERS: OnceLock<Vec<(&'static str, AdapterFactory)>> = OnceLock::new();

fn adapters() -> &'static [(&'static str, AdapterFactory)] {
    ADAPTERS.get_or_init(|| {
        vec![
            ("github", || Box::new(crate::adapters::github::GithubAdapter)),
            ("huggingface", || {
                Box::new(crate::adapters::huggingface::HuggingFaceAdapter)
            }),
            ("youtube", || Box::new(crate::adapters::youtube::YoutubeAdapter)),
            ("bilibili", || Box::new(crate::adapters::bilibili::BilibiliAdapter)),
        ]
    })
}

pub fn list_adapters() -> Vec<Box<dyn SourceAdapter>> {
    adapters().iter().map(|(_, factory)| factory()).collect()
}

pub fn tier1_body_source_types() -> Vec<&'static str> {
    list_adapters()
        .iter()
        .filter(|a| !a.ai_eligible())
        .map(|a| a.source_type())
        .collect()
}

// clippy 1.99 的 double_must_use 打在 async_trait 展开的生成代码上
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    fn source_type(&self) -> &'static str;

    fn auth_kind(&self) -> AuthKind {
        AuthKind::None
    }

    fn always_fetch_history(&self) -> bool {
        false
    }

    fn ai_eligible(&self) -> bool {
        true
    }

    fn refresh_description_after_check(&self) -> bool {
        false
    }

    fn notification_source_name(
        &self,
        owner: &str,
        _repo: &str,
        _description: Option<&str>,
    ) -> String {
        owner.to_string()
    }

    fn notification_source_label(&self) -> Option<&'static str> {
        None
    }

    fn notification_show_tag(&self) -> bool {
        true
    }

    fn log_source_ident(
        &self,
        owner: &str,
        repo: &str,
        _description: Option<&str>,
    ) -> (String, String) {
        (owner.to_string(), repo.to_string())
    }

    fn log_release_tag<'a>(&self, tag_name: &'a str, _release_name: &'a str) -> &'a str {
        tag_name
    }

    fn release_body_label(&self) -> &'static str {
        "Release Notes"
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError>;

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError>;

    async fn save(
        &self,
        db: &r2d2::Pool<crate::db::SqliteConnectionManager>,
        source: &Source,
        data: &[FetchedRelease],
        max_count: usize,
        client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)>;

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        repo: &str,
        token: Option<&str>,
    ) -> Result<String, SourceError>;

    async fn resolve_owner(
        &self,
        _client: &reqwest::Client,
        owner: &str,
        _token: Option<&str>,
    ) -> Result<String, SourceError> {
        Ok(owner.to_string())
    }
}

pub fn get_adapter(source_type: &str) -> Result<Box<dyn SourceAdapter>, SourceError> {
    adapters()
        .iter()
        .find(|(t, _)| *t == source_type)
        .map(|(_, factory)| factory())
        .ok_or_else(|| SourceError::unsupported_source(source_type))
}

pub fn source_log_ident(
    source_type: &str,
    owner: &str,
    repo: &str,
    description: Option<&str>,
) -> (String, String) {
    match get_adapter(source_type) {
        Ok(adapter) => adapter.log_source_ident(owner, repo, description),
        Err(_) => (owner.to_string(), repo.to_string()),
    }
}

pub fn release_log_ident(r: &crate::db::releases::ReleaseInfo) -> (String, String, String) {
    match get_adapter(&r.source_type) {
        Ok(adapter) => {
            let (owner, repo) =
                adapter.log_source_ident(&r.owner, &r.repo, r.source_description.as_deref());
            let tag = adapter
                .log_release_tag(&r.tag_name, &r.release_name)
                .to_string();
            (owner, repo, tag)
        }
        Err(_) => (r.owner.clone(), r.repo.clone(), r.tag_name.clone()),
    }
}

pub fn write_release_log(
    conn: &rusqlite::Connection,
    release: Option<&crate::db::releases::ReleaseInfo>,
    release_id: i64,
    key: &str,
    unknown_key: &str,
    extra: Option<serde_json::Map<String, serde_json::Value>>,
) {
    let mut args = extra.unwrap_or_default();
    args.insert("id".to_string(), serde_json::json!(release_id));
    let log_key = match release {
        Some(r) => {
            let (log_owner, log_repo, log_tag) = release_log_ident(r);
            args.insert("owner".to_string(), serde_json::json!(log_owner));
            args.insert("repo".to_string(), serde_json::json!(log_repo));
            args.insert("tag".to_string(), serde_json::json!(log_tag));
            key
        }
        None => unknown_key,
    };
    crate::db::logs::write_log_key(
        conn,
        "INFO",
        log_key,
        &serde_json::Value::Object(args).to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_kind_as_str() {
        assert_eq!(AuthKind::None.as_str(), "none");
        assert_eq!(AuthKind::GitHubToken.as_str(), "github_token");
        assert_eq!(AuthKind::YouTubeApiKey.as_str(), "youtube_api_key");
        assert_eq!(AuthKind::BilibiliCookie.as_str(), "bilibili_cookie");
    }

    #[test]
    fn test_get_adapter_unknown_type() {
        match get_adapter("gitlab") {
            Err(e) => {
                assert_eq!(e.status(), 0);
                assert!(e.to_ipc_string().contains("unsupported_source"));
            }
            Ok(_) => panic!("预期失败，实际返回 Ok"),
        }
    }

    #[test]
    fn test_token_for_picks_by_auth_kind() {
        let github = get_adapter("github").unwrap();
        let hf = get_adapter("huggingface").unwrap();
        let yt = get_adapter("youtube").unwrap();
        let bili = get_adapter("bilibili").unwrap();

        let creds = SourceCredentials::default()
            .with_token(AuthKind::GitHubToken, "ghp")
            .with_token(AuthKind::YouTubeApiKey, "yt-key")
            .with_token(AuthKind::BilibiliCookie, "sess");

        assert_eq!(creds.token_for(github.as_ref()), Some("ghp"));
        assert_eq!(creds.token_for(hf.as_ref()), None);
        assert_eq!(creds.token_for(yt.as_ref()), Some("yt-key"));
        assert_eq!(creds.token_for(bili.as_ref()), Some("sess"));

        let empty = SourceCredentials::default();
        assert_eq!(empty.token_for(yt.as_ref()), None);
        assert_eq!(empty.token_for(bili.as_ref()), None);

        assert_eq!(AuthKind::None.setting_key(), None);
        assert_eq!(
            AuthKind::GitHubToken.setting_key(),
            Some(crate::db::settings::KEY_GITHUB_TOKEN)
        );
        assert_eq!(
            AuthKind::YouTubeApiKey.setting_key(),
            Some(crate::db::settings::KEY_YOUTUBE_API_KEY)
        );
        assert_eq!(
            AuthKind::BilibiliCookie.setting_key(),
            Some(crate::db::settings::KEY_BILIBILI_COOKIE)
        );
    }

    #[test]
    fn test_list_adapters_capabilities() {
        let adapters = list_adapters();
        assert_eq!(adapters.len(), 4);

        let ineligible: Vec<&str> = adapters
            .iter()
            .filter(|a| !a.ai_eligible())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(ineligible, vec!["youtube", "bilibili"]);

        let always: Vec<&str> = adapters
            .iter()
            .filter(|a| a.always_fetch_history())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(always, vec!["youtube"]);

        let refresh: Vec<&str> = adapters
            .iter()
            .filter(|a| a.refresh_description_after_check())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(refresh, vec!["github", "youtube", "bilibili"]);

        let hide_tag: Vec<&str> = adapters
            .iter()
            .filter(|a| !a.notification_show_tag())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(hide_tag, vec!["youtube", "bilibili"]);

        let github_adapter = adapters.iter().find(|a| a.source_type() == "github").unwrap();
        assert_eq!(
            github_adapter.notification_source_name("o", "r", Some("任意描述")),
            "o"
        );
        assert_eq!(github_adapter.notification_source_label(), None);

        let yt_adapter = adapters.iter().find(|a| a.source_type() == "youtube").unwrap();
        assert_eq!(
            yt_adapter.notification_source_name("UCabc", "", Some("YouTube channel: Test Channel")),
            "Test Channel"
        );
        assert_eq!(yt_adapter.notification_source_label(), Some("YouTube"));

        let bili_adapter = adapters.iter().find(|a| a.source_type() == "bilibili").unwrap();
        assert_eq!(
            bili_adapter.notification_source_name("123456", "", Some("某UP主")),
            "某UP主"
        );
        assert_eq!(bili_adapter.notification_source_label(), Some("哔哩哔哩"));
    }

    #[test]
    fn test_bilibili_log_ident_uses_up_name_and_video_title() {
        assert_eq!(
            source_log_ident("bilibili", "123456", "", Some("测试UP主")),
            ("测试UP主".to_string(), String::new())
        );
        assert_eq!(
            source_log_ident("bilibili", "123456", "", None),
            ("123456".to_string(), String::new())
        );
        let rel = crate::db::releases::ReleaseInfo {
            id: 1,
            source_id: 1,
            source_type: "bilibili".to_string(),
            owner: "123456".to_string(),
            repo: String::new(),
            tag_name: "BV1xx411c7mD".to_string(),
            release_name: "视频标题".to_string(),
            html_url: "https://www.bilibili.com/video/BV1xx411c7mD".to_string(),
            published_at: "2026-01-01T00:00:00Z".to_string(),
            prerelease: false,
            body: None,
            detected_at: "2026-01-01T00:00:00Z".to_string(),
            notification_status: "pending".to_string(),
            snooze_until: None,
            ai_summary: None,
            ai_importance: None,
            body_translated: None,
            extra_metadata: None,
            source_description: Some("测试UP主".to_string()),
            flag: 0,
            version_bump: None,
        };
        assert_eq!(
            release_log_ident(&rel),
            ("测试UP主".to_string(), String::new(), "视频标题".to_string())
        );
    }

    #[test]
    fn test_write_release_log_uses_ident_and_handles_unknown_and_extra() {
        let conn = crate::db::init::init_memory_db().unwrap();
        let sid = crate::db::sources::add_source(
            &conn,
            "youtube",
            "UC_test_channel",
            "",
            "Test Channel",
        )
        .unwrap();
        let rid = crate::db::releases::insert_release(
            &conn,
            sid,
            "vid_abc",
            "Episode 1",
            "https://youtu.be/vid_abc",
            "2026-01-01T00:00:00Z",
            false,
            None,
        )
        .unwrap();
        let rel = crate::db::releases::get_release(&conn, rid).unwrap();

        let mut extra = serde_json::Map::new();
        extra.insert("action".to_string(), serde_json::json!("clicked"));
        write_release_log(
            &conn,
            rel.as_ref(),
            rid,
            "release.status_changed",
            "release.status_changed_unknown",
            Some(extra),
        );
        write_release_log(
            &conn,
            None,
            9999,
            "release.deleted",
            "release.deleted_unknown",
            None,
        );

        let logs = crate::db::logs::get_logs(&conn, 10).unwrap();
        let known = logs
            .iter()
            .find(|l| l.message_key.as_deref() == Some("release.status_changed"))
            .unwrap();
        let known_args: serde_json::Value =
            serde_json::from_str(known.message_args.as_deref().unwrap()).unwrap();
        assert_eq!(known_args["owner"], "Test Channel");
        assert_eq!(known_args["repo"], "");
        assert_eq!(known_args["tag"], "Episode 1");
        assert_eq!(known_args["action"], "clicked");

        let unknown = logs
            .iter()
            .find(|l| l.message_key.as_deref() == Some("release.deleted_unknown"))
            .unwrap();
        let unknown_args: serde_json::Value =
            serde_json::from_str(unknown.message_args.as_deref().unwrap()).unwrap();
        assert_eq!(unknown_args["id"], 9999);
    }
}
