use rusqlite::Connection;

use crate::db::releases;

async fn fetch_releases_inner(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    per_page: usize,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let url = format!(
        "{}/repos/{}/{}/releases?per_page={}",
        api_base.trim_end_matches('/'),
        owner,
        repo,
        per_page,
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| (0, format!("请求失败: {}", e)))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let reason = resp.status().canonical_reason().unwrap_or("").to_string();
        return Err((status, format!("GitHub API 返回 {} {}", status, reason)));
    }
    resp.json().await.map_err(|e| (status, format!("解析失败: {}", e)))
}

async fn fetch_releases_with_retry(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
    per_page: usize,
) -> Result<Vec<serde_json::Value>, String> {
    let max_retries = 3;
    let mut attempt = 0;
    loop {
        match fetch_releases_inner(client, owner, repo, api_base, per_page).await {
            Ok(data) => return Ok(data),
            Err((status, msg)) => {
                if status == 403 {
                    return Err(msg);
                }
                attempt += 1;
                if attempt > max_retries {
                    return Err(format!("重试{}次后仍然失败: {}", max_retries, msg));
                }
                let delay = std::time::Duration::from_secs(1 << attempt);
                log::warn!("请求失败(状态={}), {}后重试({}/{}): {}", status, delay.as_secs(), attempt, max_retries, msg);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

pub async fn fetch_releases(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    per_page: usize,
) -> Result<Vec<serde_json::Value>, String> {
    fetch_releases_with_retry(client, owner, repo, "https://api.github.com", per_page).await
}

pub async fn fetch_repo_info(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("err.repo_verify_failed|{}", e))?;
    if resp.status() == 404 {
        return Err(format!("err.repo_not_found|{}|{}", owner, repo));
    }
    if !resp.status().is_success() {
        return Err(format!("err.repo_api_error|{}", resp.status().as_u16()));
    }
    let info: serde_json::Value = resp
        .json()
        .await
        .map_err(|_| "Failed to parse repo info".to_string())?;
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
        let published = rel["published_at"].as_str().unwrap_or("");
        let body = rel["body"].as_str();
        if let Ok(id) =
            releases::insert_release(conn, source_id, tag, name, html_url, published, pre, body)
        {
            if id > 0 {
                saved.push((id, body.map(|s| s.to_string())));
                if saved.len() >= max_count {
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
    use wiremock::matchers::{method, path, query_param};

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

        let client = reqwest::Client::new();
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri(), 10).await;
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

        let client = reqwest::Client::new();
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri(), 10).await;
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

        let client = reqwest::Client::new();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10).await;
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

        let client = reqwest::Client::new();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("403"));
    }

    #[tokio::test]
    async fn test_fetch_with_retry_429_exhausted() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock)
            .await;

        let client = reqwest::Client::new();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("重试3次后仍然失败"));
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
        let sid = db::config::add_source(&conn, "github", "o", "r", "").unwrap();

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
        let sid = db::config::add_source(&conn, "github", "o", "r", "").unwrap();

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
        let sid = db::config::add_source(&conn, "github", "o", "r", "").unwrap();

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
        let sid = db::config::add_source(&conn, "github", "o", "r", "").unwrap();

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
        let sid = db::config::add_source(&conn, "github", "o", "r", "").unwrap();

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
        let sid = db::config::add_source(&conn, "github", "o", "r", "").unwrap();

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
}
