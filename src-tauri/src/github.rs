use rusqlite::Connection;

use crate::db::releases;
use crate::db::sources::Source;
use crate::http;
use crate::source::SourceAdapter;

const GH_API_BASE: &str = "https://api.github.com";

/// GitHub 监控源适配器。实现 `SourceAdapter` trait，
/// 把 fetch / save / verify 收敛到统一接口。
pub struct GithubAdapter;

#[async_trait::async_trait]
impl SourceAdapter for GithubAdapter {
    fn source_type(&self) -> &'static str {
        "github"
    }

    fn auth_kind(&self) -> crate::source::AuthKind {
        crate::source::AuthKind::GitHubToken
    }

    /// GitHub 检查成功后刷新仓库描述。
    fn refresh_description_after_check(&self) -> bool {
        true
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)> {
        fetch_releases(client, &source.owner, &source.repo, per_page, token).await
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)> {
        fetch_all_releases_with_limit(client, &source.owner, &source.repo, max_count, token).await
    }

    async fn save(
        &self,
        db: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
        source: &Source,
        data: &[serde_json::Value],
        max_count: usize,
        _client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
        // github save 是同步的，用 spawn_blocking 转包避免在 async 上下文阻塞
        // （与 Phase 2 的 spawn_blocking 改造顺接）。
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
    ) -> Result<String, (u16, String)> {
        fetch_repo_info(client, owner, repo, token).await
    }
}

async fn fetch_releases_inner(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    per_page: usize,
    token: Option<&str>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let url = format!(
        "{}/repos/{}/{}/releases?per_page={}",
        api_base.trim_end_matches('/'),
        owner,
        repo,
        per_page,
    );
    // token 按请求设置，避免共享 client 时 GitHub Token 作为 default header 泄露给 huggingface.co
    http::fetch_page_with_retry(client, &url, token)
        .await
        .map(|(releases, _)| releases)
}

async fn fetch_releases_with_retry(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    per_page: usize,
    token: Option<&str>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    fetch_releases_inner(client, owner, repo, api_base, per_page, token).await
}

pub async fn fetch_releases(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    per_page: usize,
    token: Option<&str>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    fetch_releases_with_retry(client, owner, repo, GH_API_BASE, per_page, token).await
}

// ── 分页拉取（复用 http::paginated_fetch）────────────────

/// 拉取 releases 直到满足 max_count，自动翻页。
/// - `None` = 不设上限（拉取全部）
/// - `Some(n)` = 拉取至少 n 条后停止
pub async fn fetch_all_releases_with_limit(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    max_count: Option<usize>,
    token: Option<&str>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    fetch_all_releases_inner(client, owner, repo, GH_API_BASE, max_count, token).await
}

async fn fetch_all_releases_inner(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    max_count: Option<usize>,
    token: Option<&str>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let first_url = format!(
        "{}/repos/{}/{}/releases?per_page=100",
        api_base.trim_end_matches('/'),
        owner, repo,
    );
    http::paginated_fetch(client, first_url, max_count, token).await
}

pub async fn fetch_repo_info(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<String, (u16, String)> {
    let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let mut req = client.get(&url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| (0, format!("err.repo_verify_failed|{}", e)))?;
    if resp.status() == 404 {
        return Err((404, format!("err.repo_not_found|{}|{}", owner, repo)));
    }
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        return Err((code, format!("err.repo_api_error|{}", code)));
    }
    let info: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    Ok(info["description"].as_str().unwrap_or("").to_string())
}

pub fn save_releases(
    conn: &Connection,
    source_id: i64,
    gh_releases: &[serde_json::Value],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    let check_pre = crate::db::settings::get_setting(
        conn,
        crate::db::settings::KEY_CHECK_PRERELEASES,
    )
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(false);

    // 按 published_at 降序排列，确保最新发布排在最前
    let mut sorted: Vec<&serde_json::Value> = gh_releases.iter().collect();
    sorted.sort_by(|a, b| {
        let pa = a["published_at"].as_str().unwrap_or("");
        let pb = b["published_at"].as_str().unwrap_or("");
        pb.cmp(pa)
    });

    let mut saved = Vec::new();
    for rel in &sorted {
        let pre = rel["prerelease"].as_bool().unwrap_or(false);
        if pre && !check_pre {
            continue;
        }
        let tag = rel["tag_name"].as_str().unwrap_or("");
        let name = rel["name"].as_str().unwrap_or("");
        let html_url = rel["html_url"].as_str().unwrap_or("");
        let published = match rel["published_at"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let body = rel["body"].as_str();
        if let Ok(id) =
            releases::insert_release(conn, source_id, tag, name, html_url, published, pre, body)
        {
            if id > 0 {
                saved.push((id, body.map(|s| s.to_string())));
                if max_count > 0 && saved.len() >= max_count {
                    return saved;
                }
                continue;
            }
        }
        // 已入库且普通模式（max_count=1）时，说明不是新版，停止
        if max_count == 1 {
            return vec![];
        }
        // 历史模式：已存在的跳过，继续找更新/更旧的新版
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};

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
        assert_eq!(releases[0]["tag_name"], "v1.0.0");
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
        assert_eq!(result.unwrap_err().0, 403);
    }

    #[tokio::test]
    async fn test_fetch_with_retry_429_then_200() {
        let mock = MockServer::start().await;
        // First request: 429
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        // Subsequent: 200
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
        assert_eq!(result.unwrap_err().0, 403);
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
        assert_eq!(result.unwrap_err().0, 429);
    }

    // ── parse_next_link 测试已移至 http.rs（函数下沉后归属处）──

    // ── fetch_all_releases 分页测试 ──

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
        // Single page, no Link header
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param_is_missing("page"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(1, 3)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0]["tag_name"], "v1");
        assert_eq!(releases[2]["tag_name"], "v3");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_with_limit() {
        let mock = MockServer::start().await;

        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());

        // Page 1: 3 items, Link: next=page2
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param_is_missing("page"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(releases_page(1, 3))
                .insert_header("link", build_next_link(&page2_url)))
            .mount(&mock)
            .await;

        // Page 2: 3 items, no Link
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(4, 3)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // max_count=4: page 1 = 3 items (< 4), fetch page 2; total = 6 (>= 4), stop
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), Some(4), None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        // 应至少获取 4 条；实际拿到 6 条（翻了一整页后检查到 >=4 才停）
        assert!(releases.len() >= 4, "应至少获取 4 条，实际 {}", releases.len());
        assert_eq!(releases[0]["tag_name"], "v1");
        assert_eq!(releases[3]["tag_name"], "v4");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_multi_page() {
        let mock = MockServer::start().await;

        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());

        // Page 1: items 1-2, Link: next=page2
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param_is_missing("page"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(releases_page(1, 2))
                .insert_header("link", build_next_link(&page2_url)))
            .mount(&mock)
            .await;

        // Page 2: items 3-4, no Link (last page)
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(3, 2)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 4);
        assert_eq!(releases[0]["tag_name"], "v1");
        assert_eq!(releases[3]["tag_name"], "v4");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_three_pages() {
        let mock = MockServer::start().await;

        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());
        let page3_url = format!("{}/repos/o/r/releases?per_page=100&page=3", mock.uri());

        // Page 1: items 1-2, Link: next=page2
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param_is_missing("page"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(releases_page(1, 2))
                .insert_header("link", build_next_link(&page2_url)))
            .mount(&mock)
            .await;

        // Page 2: items 3-4, Link: next=page3
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(releases_page(3, 2))
                .insert_header("link", build_next_link(&page3_url)))
            .mount(&mock)
            .await;

        // Page 3: items 5-6, no Link
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param("page", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(releases_page(5, 2)))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_ok());
        let releases = result.unwrap();
        assert_eq!(releases.len(), 6);
        assert_eq!(releases[0]["tag_name"], "v1");
        assert_eq!(releases[5]["tag_name"], "v6");
    }

    #[tokio::test]
    async fn test_fetch_all_releases_api_error_stops() {
        let mock = MockServer::start().await;

        let page2_url = format!("{}/repos/o/r/releases?per_page=100&page=2", mock.uri());

        // Page 1: ok, Link: next=page2
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param_is_missing("page"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(releases_page(1, 2))
                .insert_header("link", build_next_link(&page2_url)))
            .mount(&mock)
            .await;

        // Page 2: 403 error
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases")).and(query_param("per_page", "100")).and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_all_releases_inner(&client, "o", "r", &mock.uri(), None, None).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 403);
    }

    fn rel(tag: &str, date: &str, pre: bool, body: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "name": tag,
            "html_url": format!("https://github.com/o/r/releases/tag/{}", tag),
            "published_at": date,
            "prerelease": pre,
            "body": body,
        })
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
    fn test_save_releases_max_count_1_existing_returns_empty() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

        let data = vec![
            rel("v3.0.0", "2024-03-01T00:00:00Z", false, Some("v3 body")),
            rel("v2.0.0", "2024-02-01T00:00:00Z", false, Some("v2 body")),
        ];
        // First save: v3.0.0 is new
        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);

        // Second save with same data: v3.0.0 already exists, should return empty
        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 0);
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
        // First save v3.0.0
        let result = save_releases(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);

        // Historical mode: v3.0.0 exists (skip), v2.0.0 is new, v1.0.0 is new
        // max_count=2 should return v2.0.0 and v1.0.0
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
        // Skip prerelease, take v3.0.0 and v2.0.0
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

    /// 模拟用户描述的场景：
    /// 1. 上次检查保存了 v1.15.6（max_count=1）
    /// 2. 几天后重启，API 返回 v1.15.10 ~ v1.15.6
    /// 3. 使用 fetch_history_count=4 → 应保存 v1.15.10/9/8/7，v1.15.6 跳过
    /// 4. 模拟 poll_all_sources_async 中的标记已读逻辑
    #[test]
    fn test_intermediate_versions_all_saved_and_marked_read() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "anomalyco", "opencode", "").unwrap();

        // ── 第一次检查：max_count=1，只保存 v1.15.6 ──
        let data1 = vec![
            rel("v1.15.6", "2024-06-01T00:00:00Z", false, Some("v1.15.6 body")),
        ];
        let r1 = save_releases(&conn, sid, &data1, 1);
        assert_eq!(r1.len(), 1, "首次检查应保存 1 个版本");
        let _id_156 = r1[0].0;

        // ── 重启后：API 返回 v1.15.6 ~ v1.15.10 ──
        let data2 = vec![
            rel("v1.15.10", "2024-06-05T00:00:00Z", false, Some("v1.15.10 body")),
            rel("v1.15.9",  "2024-06-04T00:00:00Z", false, Some("v1.15.9 body")),
            rel("v1.15.8",  "2024-06-03T00:00:00Z", false, Some("v1.15.8 body")),
            rel("v1.15.7",  "2024-06-02T00:00:00Z", false, Some("v1.15.7 body")),
            rel("v1.15.6",  "2024-06-01T00:00:00Z", false, Some("v1.15.6 body")),
        ];
        // 使用 fetch_history_count=4
        let fetch_history_count = 4usize;
        let r2 = save_releases(&conn, sid, &data2, fetch_history_count);
        assert_eq!(r2.len(), 4, "应保存 v1.15.10/9/8/7，跳过已存在的 v1.15.6");

        // 验证顺序：最新的在前
        assert_eq!(r2[0].1.as_deref(), Some("v1.15.10 body"));
        assert_eq!(r2[1].1.as_deref(), Some("v1.15.9 body"));
        assert_eq!(r2[2].1.as_deref(), Some("v1.15.8 body"));
        assert_eq!(r2[3].1.as_deref(), Some("v1.15.7 body"));

        // ── 模拟 poll_all_sources_async 中的标记已读逻辑 ──
        if r2.len() > 1 {
            for (id, _) in r2.iter().skip(1) {
                db::releases::set_notification_state(&conn, *id, "clicked", None).unwrap();
            }
        }

        // ── 验证 DB 中有 5 条记录 ──
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases.len(), 5, "DB 应有 5 条记录");
        let tags: Vec<&str> = releases.iter().map(|r| r.tag_name.as_str()).collect();
        assert!(tags.contains(&"v1.15.10"));
        assert!(tags.contains(&"v1.15.9"));
        assert!(tags.contains(&"v1.15.8"));
        assert!(tags.contains(&"v1.15.7"));
        assert!(tags.contains(&"v1.15.6"));

        // ── 验证通知状态 ──
        let get_status = |tag: &str| -> String {
            releases.iter()
                .find(|r| r.tag_name == tag)
                .map(|r| r.notification_status.clone())
                .unwrap()
        };
        assert_eq!(get_status("v1.15.10"), "pending", "最新版应是 pending");
        assert_eq!(get_status("v1.15.9"),  "clicked",  "中间版应标记为已读");
        assert_eq!(get_status("v1.15.8"),  "clicked",  "中间版应标记为已读");
        assert_eq!(get_status("v1.15.7"),  "clicked",  "中间版应标记为已读");
        // v1.15.6 在首次检查时就是 pending，不会被重新标记
        assert_eq!(get_status("v1.15.6"), "pending", "首次保存的版本保留原有状态");
    }

    /// 复现用户的 bug：多轮 poll 场景
    /// 第一轮：检测到 v1.15.10（通知），v1.15.10 已入库
    /// 第二轮：检测到 v1.15.9、v1.15.7（中间版本），
    ///         因为 v1.15.10 已在库中，v1.15.9 不应通知
    #[test]
    fn test_intermediate_versions_not_notified_when_newer_exists() {
        let conn = db::init::init_memory_db().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let sid = db::sources::add_source(&conn, "github", "anomalyco", "opencode", "").unwrap();

        // ── 第一轮：检测到 v1.15.10（最新），通知 ──
        let data1 = vec![
            rel("v1.15.10", "2024-06-05T00:00:00Z", false, Some("v1.15.10 body")),
        ];
        let r1 = save_releases(&conn, sid, &data1, 1);
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].1.as_deref(), Some("v1.15.10 body"));
        // v1.15.10 是全局最新 → 保持 pending（通知）
        // 模拟 poll_all_sources_async 的标记逻辑
        assert!(!db::releases::has_newer_release(&conn, sid, "2024-06-05T00:00:00Z").unwrap_or(true),
            "v1.15.10 应是全局最新");

        // ── 第二轮：检测到 v1.15.9、v1.15.7（比 v1.15.10 旧）──
        let data2 = vec![
            rel("v1.15.9",  "2024-06-04T00:00:00Z", false, Some("v1.15.9 body")),
            rel("v1.15.7",  "2024-06-02T00:00:00Z", false, Some("v1.15.7 body")),
        ];
        let r2 = save_releases(&conn, sid, &data2, 2);
        assert_eq!(r2.len(), 2);

        // 模拟 poll_all_sources_async 的标记逻辑（新版）
        let latest_id = r2[0].0;
        let has_newer = db::releases::get_release(&conn, latest_id).unwrap()
            .and_then(|r| db::releases::has_newer_release(&conn, sid, &r.published_at).ok())
            .unwrap_or(false);
        assert!(has_newer, "v1.15.9 比库中 v1.15.10 旧，应检测到更新版本存在");

        // 有更新版本存在 → 全部标记为已读
        for (id, _) in &r2 {
            db::releases::set_notification_state(&conn, *id, "clicked", None).unwrap();
        }

        // ── 验证通知状态 ──
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let get_status = |tag: &str| -> String {
            releases.iter()
                .find(|r| r.tag_name == tag)
                .map(|r| r.notification_status.clone())
                .unwrap()
        };
        // 后续保存的中间版本全部 clicked
        assert_eq!(get_status("v1.15.9"),  "clicked", "已有更新版本在库，v1.15.9 不应通知");
        assert_eq!(get_status("v1.15.7"),  "clicked", "已有更新版本在库，v1.15.7 不应通知");
        // v1.15.10 保留原有状态
        assert_eq!(get_status("v1.15.10"), "pending", "v1.15.10 是全局最新，保持 pending");

        // ── 验证 DB 有三条记录 ──
        assert_eq!(releases.len(), 3);
    }
}
