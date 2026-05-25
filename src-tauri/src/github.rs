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
    let config = crate::retry::RetryConfig::default();
    crate::retry::retry_with_backoff(&config, |e: &(u16, String)| {
        if e.0 == 403 {
            return false;
        }
        log::warn!("请求失败(状态={}), 将重试: {}", e.0, e.1);
        true
    }, || async {
        fetch_releases_inner(client, owner, repo, api_base, per_page).await
    })
    .await
    .map_err(|(_, msg)| msg)
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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/repos/{}/{}/releases?per_page={}", mock.uri(), "owner", "repo", 10);
        let raw_resp = client.get(&url).send().await;
        eprintln!("DEBUG raw_resp: {:?}", raw_resp);
        let result = fetch_releases_inner(&client, "owner", "repo", &mock.uri(), 10).await;
        eprintln!("DEBUG result err: {:?}", result.as_ref().map_err(|e| &e.1));
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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = fetch_releases_with_retry(&client, "owner", "repo", &mock.uri(), 10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("429"));
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
        assert!(db::releases::has_newer_release(&conn, sid, "2024-06-05T00:00:00Z").unwrap_or(true) == false,
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
