use rusqlite::Connection;

use crate::db::sources::Source;
use crate::error::SourceError;
use crate::http;
use crate::source::{FetchedRelease, SourceAdapter, SourceExtraMetadata};

const GH_API_BASE: &str = "https://api.github.com";

pub struct GithubAdapter;

#[async_trait::async_trait]
impl SourceAdapter for GithubAdapter {
    fn source_type(&self) -> &'static str {
        "github"
    }

    fn auth_kind(&self) -> crate::source::AuthKind {
        crate::source::AuthKind::GitHubToken
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
        fetch_releases(client, &source.owner, &source.repo, per_page, token).await
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        fetch_all_releases_with_limit(client, &source.owner, &source.repo, max_count, token).await
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
            save_releases(&conn, source_id, &data, max_count)
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("github save spawn_blocking panic: {}", e);
            vec![]
        })
    }

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        repo: &str,
        token: Option<&str>,
    ) -> Result<String, SourceError> {
        fetch_repo_info(client, owner, repo, token).await
    }
}

fn parse_github_release(rel: &serde_json::Value) -> Option<FetchedRelease> {
    let published = rel["published_at"].as_str().filter(|s| !s.is_empty())?;
    Some(FetchedRelease {
        tag: rel["tag_name"].as_str().unwrap_or("").to_string(),
        name: rel["name"].as_str().unwrap_or("").to_string(),
        html_url: rel["html_url"].as_str().unwrap_or("").to_string(),
        published: published.to_string(),
        prerelease: rel["prerelease"].as_bool().unwrap_or(false),
        body: rel["body"].as_str().map(|s| s.to_string()),
        extra: SourceExtraMetadata::None,
    })
}

async fn fetch_releases_inner(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    per_page: usize,
    token: Option<&str>,
) -> Result<Vec<FetchedRelease>, SourceError> {
    let url = format!(
        "{}/repos/{}/{}/releases?per_page={}",
        api_base.trim_end_matches('/'),
        owner,
        repo,
        per_page,
    );
    http::fetch_page_with_retry(client, &url, token)
        .await
        .map(|(releases, _)| releases.iter().filter_map(parse_github_release).collect())
}

async fn fetch_releases_with_retry(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    per_page: usize,
    token: Option<&str>,
) -> Result<Vec<FetchedRelease>, SourceError> {
    fetch_releases_inner(client, owner, repo, api_base, per_page, token).await
}

pub async fn fetch_releases(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    per_page: usize,
    token: Option<&str>,
) -> Result<Vec<FetchedRelease>, SourceError> {
    fetch_releases_with_retry(client, owner, repo, GH_API_BASE, per_page, token).await
}

pub async fn fetch_all_releases_with_limit(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    max_count: Option<usize>,
    token: Option<&str>,
) -> Result<Vec<FetchedRelease>, SourceError> {
    fetch_all_releases_inner(client, owner, repo, GH_API_BASE, max_count, token).await
}

async fn fetch_all_releases_inner(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    max_count: Option<usize>,
    token: Option<&str>,
) -> Result<Vec<FetchedRelease>, SourceError> {
    let first_url = format!(
        "{}/repos/{}/{}/releases?per_page=100",
        api_base.trim_end_matches('/'),
        owner,
        repo,
    );
    let raw = http::paginated_fetch(client, first_url, max_count, token).await?;
    Ok(raw.iter().filter_map(parse_github_release).collect())
}

pub async fn fetch_repo_info(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<String, SourceError> {
    let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let mut req = client.get(&url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| SourceError::coded(0, "err.repo_verify_failed", [e.to_string()]))?;
    if resp.status() == 404 {
        return Err(SourceError::coded(404, "err.repo_not_found", [owner, repo]));
    }
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        return Err(SourceError::coded(code, "err.repo_api_error", [code.to_string()]));
    }
    let info: serde_json::Value = crate::http::read_json_limited(resp, crate::http::MAX_JSON_BYTES)
        .await
        .map_err(|e| SourceError::raw(0, e))?;
    Ok(info["description"].as_str().unwrap_or("").to_string())
}

pub fn save_releases(
    conn: &Connection,
    source_id: i64,
    gh_releases: &[FetchedRelease],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    let check_pre = crate::db::sources::source_config_flag(conn, source_id, "check_prereleases")
        .or_else(|| {
            crate::db::settings::get_setting(conn, crate::db::settings::KEY_CHECK_PRERELEASES)
                .ok()
                .flatten()
                .map(|v| v == "true")
        })
        .unwrap_or(false);

    let entries: Vec<crate::db::save::SaveEntry> = gh_releases
        .iter()
        .filter(|rel| !rel.prerelease || check_pre)
        .filter(|rel| !rel.published.is_empty())
        .map(FetchedRelease::to_save_entry)
        .collect();

    crate::db::save::save_entries_generic(
        conn,
        source_id,
        &entries,
        max_count,
        |_, _, _| {},
        |_, _, _| {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_releases() -> serde_json::Value {
        serde_json::json!([{
            "tag_name": "v1.0.0",
            "name": "Version 1.0.0",
            "html_url": "https://github.com/owner/repo/releases/tag/v1.0.0",
            "published_at": "2024-01-01T00:00:00Z",
            "prerelease": false,
            "body": "Some release notes"
        }])
    }

    #[tokio::test]
    async fn test_fetch_inner_200_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .and(query_param("per_page", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_releases()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri(), 10, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "v1.0.0");
    }

    #[tokio::test]
    async fn test_fetch_inner_403() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri(), 10, None).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 403);
    }

    #[tokio::test]
    async fn test_fetch_with_retry_429_then_200() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_releases()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fetch_with_retry_403_no_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10, None).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 403);
    }

    #[tokio::test]
    async fn test_fetch_with_retry_429_exhausted() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10, None).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 429);
    }

    fn releases_page(start: u32, count: u32) -> serde_json::Value {
        let items: Vec<serde_json::Value> = (start..start + count)
            .map(|i| {
                serde_json::json!({
                    "tag_name": format!("v{}", i),
                    "name": format!("Version {}", i),
                    "html_url": format!("https://github.com/o/r/releases/tag/v{}", i),
                    "published_at": format!("2024-01-{:02}T00:00:00Z", i),
                    "prerelease": false,
                    "body": format!("Release notes for v{}", i),
                })
            })
            .collect();
        serde_json::json!(items)
    }

    fn build_next_link(next_url: &str) -> String {
        format!("<{}>; rel=\"next\"", next_url)
    }

    #[tokio::test]
    async fn test_fetch_all_releases_single_page() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param_is_missing("page"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(1, 3)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0].tag, "v1");
        assert_eq!(releases[2].tag, "v3");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_with_limit() {
        let mock = MockServer::start().await;
        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param_is_missing("page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(releases_page(1, 3))
                    .insert_header("link", build_next_link(&page2_url)),
            )
            .mount(&mock)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(4, 3)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), Some(4), None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert!(releases.len() >= 4, "应至少获取 4 条，实际 {}", releases.len());
        assert_eq!(releases[0].tag, "v1");
        assert_eq!(releases[3].tag, "v4");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_multi_page() {
        let mock = MockServer::start().await;
        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param_is_missing("page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(releases_page(1, 2))
                    .insert_header("link", build_next_link(&page2_url)),
            )
            .mount(&mock)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(3, 2)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 4);
        assert_eq!(releases[0].tag, "v1");
        assert_eq!(releases[3].tag, "v4");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_three_pages() {
        let mock = MockServer::start().await;
        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());
        let page3_url = format!("{}/repos/o/r/releases?per_page=100&page=3", mock.uri());

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param_is_missing("page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(releases_page(1, 2))
                    .insert_header("link", build_next_link(&page2_url)),
            )
            .mount(&mock)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(releases_page(3, 2))
                    .insert_header("link", build_next_link(&page3_url)),
            )
            .mount(&mock)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(5, 2)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 6);
        assert_eq!(releases[0].tag, "v1");
        assert_eq!(releases[5].tag, "v6");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_api_error_stops() {
        let mock = MockServer::start().await;
        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param_is_missing("page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(releases_page(1, 2))
                    .insert_header("link", build_next_link(&page2_url)),
            )
            .mount(&mock)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 403);
    }

    fn rel(tag: &str, date: &str, pre: bool, body: Option<&str>) -> FetchedRelease {
        FetchedRelease {
            tag: tag.to_string(),
            name: tag.to_string(),
            html_url: format!("https://github.com/o/r/releases/tag/{}", tag),
            published: date.to_string(),
            prerelease: pre,
            body: body.map(|s| s.to_string()),
            extra: SourceExtraMetadata::None,
        }
    }

    #[test]
    fn test_save_releases_max_count_1() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
        ];
        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.as_deref(), Some("v3 body"));
    }

    #[test]
    fn test_save_releases_max_count_3() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
            rel("v1.0.0", "2024-01-01T00:00:00Z", false, Some("v1 body")),
        ];
        let result = save_releases(&conn, sid, &data, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].1.as_deref(), Some("v3 body"));
        assert_eq!(result[1].1.as_deref(), Some("v2 body"));
        assert_eq!(result[2].1.as_deref(), Some("v1 body"));
    }

    #[test]
    fn test_save_releases_write_cap_backfills_older_unsaved_release() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
        ];
        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);

        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);
        let mut tags: Vec<String> = db::releases::get_releases_with_state(&conn)
            .unwrap()
            .into_iter()
            .map(|r| r.tag_name)
            .collect();
        tags.sort();
        assert_eq!(tags, vec!["v2.0.0".to_string(), "v3.0.0".to_string()]);
    }

    #[test]
    fn test_save_releases_historical_skips_existing_and_continues() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
            rel("v1.0.0", "2024-01-01T00:00:00Z", false, Some("v1 body")),
        ];
        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);

        let result = save_releases(&conn, sid, &data, 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1.as_deref(), Some("v2 body"));
        assert_eq!(result[1].1.as_deref(), Some("v1 body"));
    }

    #[test]
    fn test_save_releases_skips_prerelease_when_disabled() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v4.0.0-pre", "2024-04-01T00:00:00Z", true, Some("pre body")),
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
        ];
        let result = save_releases(&conn, sid, &data, 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1.as_deref(), Some("v3 body"));
        assert_eq!(result[1].1.as_deref(), Some("v2 body"));
    }

    #[test]
    fn test_save_releases_includes_prerelease_when_enabled() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "true").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v4.0.0-pre", "2024-04-01T00:00:00Z", true, Some("pre body")),
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
        ];
        let result = save_releases(&conn, sid, &data, 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1.as_deref(), Some("pre body"));
        assert_eq!(result[1].1.as_deref(), Some("v3 body"));
    }

    #[test]
    fn test_intermediate_versions_all_saved_and_marked_read() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "example-org", "example-repo", "").unwrap();

        let data1 = vec![rel("v1.15.6", "2024-06-01T00:00:00Z", false, Some("v1.15.6 body"))];
        let r1 = save_releases(&conn, sid, &data1, 1);
        assert_eq!(r1.len(), 1);

        let data2 = vec![
            rel("v1.15.10", "2024-06-05T00:00:00Z", false, Some("v1.15.10 body")),
            rel("v1.15.9", "2024-06-04T00:00:00Z", false, Some("v1.15.9 body")),
            rel("v1.15.8", "2024-06-03T00:00:00Z", false, Some("v1.15.8 body")),
            rel("v1.15.7", "2024-06-02T00:00:00Z", false, Some("v1.15.7 body")),
            rel("v1.15.6", "2024-06-01T00:00:00Z", false, Some("v1.15.6 body")),
        ];
        let r2 = save_releases(&conn, sid, &data2, 4);
        assert_eq!(r2.len(), 4);
        assert_eq!(r2[0].1.as_deref(), Some("v1.15.10 body"));
        assert_eq!(r2[1].1.as_deref(), Some("v1.15.9 body"));
        assert_eq!(r2[2].1.as_deref(), Some("v1.15.8 body"));
        assert_eq!(r2[3].1.as_deref(), Some("v1.15.7 body"));

        if r2.len() > 1 {
            for (id, _) in r2.iter().skip(1) {
                db::releases::set_notification_state(&conn, *id, "clicked", None).unwrap();
            }
        }

        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 5);
        let get_status = |tag: &str| -> String {
            releases
                .iter()
                .find(|r| r.tag_name == tag)
                .map(|r| r.notification_status.clone())
                .unwrap()
        };
        assert_eq!(get_status("v1.15.10"), "pending");
        assert_eq!(get_status("v1.15.9"), "clicked");
        assert_eq!(get_status("v1.15.8"), "clicked");
        assert_eq!(get_status("v1.15.7"), "clicked");
        assert_eq!(get_status("v1.15.6"), "pending");
    }

    #[test]
    fn test_intermediate_versions_not_notified_when_newer_exists() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "example-org", "example-repo", "").unwrap();

        let data1 = vec![rel("v1.15.10", "2024-06-05T00:00:00Z", false, Some("v1.15.10 body"))];
        let r1 = save_releases(&conn, sid, &data1, 1);
        assert_eq!(r1.len(), 1);
        assert!(!db::releases::has_newer_release(&conn, sid, "2024-06-05T00:00:00Z").unwrap_or(true));

        let data2 = vec![
            rel("v1.15.9", "2024-06-04T00:00:00Z", false, Some("v1.15.9 body")),
            rel("v1.15.7", "2024-06-02T00:00:00Z", false, Some("v1.15.7 body")),
        ];
        let r2 = save_releases(&conn, sid, &data2, 2);
        assert_eq!(r2.len(), 2);

        let latest_id = r2[0].0;
        let has_newer = db::releases::get_release(&conn, latest_id)
            .unwrap()
            .and_then(|r| db::releases::has_newer_release(&conn, sid, &r.published_at).ok())
            .unwrap_or(false);
        assert!(has_newer);

        for (id, _) in &r2 {
            db::releases::set_notification_state(&conn, *id, "clicked", None).unwrap();
        }

        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let get_status = |tag: &str| -> String {
            releases
                .iter()
                .find(|r| r.tag_name == tag)
                .map(|r| r.notification_status.clone())
                .unwrap()
        };
        assert_eq!(get_status("v1.15.9"), "clicked");
        assert_eq!(get_status("v1.15.7"), "clicked");
        assert_eq!(get_status("v1.15.10"), "pending");
        assert_eq!(releases.len(), 3);
    }
}
