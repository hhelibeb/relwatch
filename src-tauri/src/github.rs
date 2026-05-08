use rusqlite::Connection;

use crate::db::releases;

async fn fetch_releases_inner(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    api_base: &str,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let url = format!(
        "{}/repos/{}/{}/releases?per_page=10",
        api_base.trim_end_matches('/'),
        owner,
        repo
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
) -> Result<Vec<serde_json::Value>, String> {
    let max_retries = 3;
    let mut attempt = 0;
    loop {
        match fetch_releases_inner(client, owner, repo, api_base).await {
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
) -> Result<Vec<serde_json::Value>, String> {
    fetch_releases_with_retry(client, owner, repo, "https://api.github.com").await
}

pub fn save_releases(
    conn: &Connection,
    source_id: i64,
    gh_releases: &[serde_json::Value],
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

    // 找到第一条符合条件的 release 即返回（后面的比它旧，无需处理）
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
                return vec![(id, body.map(|s| s.to_string()))];
            }
        }
        // 已入库说明不是新版，后面的都比它旧，停止
        return vec![];
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri()).await;
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
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri()).await;
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
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri()).await;
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
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri()).await;
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
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("重试3次后仍然失败"));
    }
}
