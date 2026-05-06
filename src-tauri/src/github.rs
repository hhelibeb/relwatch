use rusqlite::Connection;

use crate::db::releases;

pub fn fetch_releases(
    client: &reqwest::blocking::Client,
    owner: &str,
    repo: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases?per_page=10",
        owner, repo
    );
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 {} {}", resp.status().as_u16(), resp.status().canonical_reason().unwrap_or("")));
    }
    resp.json().map_err(|e| format!("解析失败: {}", e))
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
