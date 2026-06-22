//! HuggingFace 组织新模型监控。
//!
//! 对接 HuggingFace Hub 公开 API（`https://huggingface.co/api/models`），
//! 按 `createdAt` 降序拉取组织下的模型列表，映射为 `releases` 表记录。
//! 公开模型读取无需认证；HF Token 为可选设置（见设计文档 §2）。

use rusqlite::Connection;

use crate::db::releases;

const HF_API_BASE: &str = "https://huggingface.co";

/// 解析后的 HuggingFace 模型元数据。
///
/// `sort=createdAt` 实测时 `last_modified` 字段为 None，但保留字段以兼容其他排序场景。
#[derive(Debug, Clone)]
struct HfModel {
    id: String,
    #[allow(dead_code)]
    author: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    last_modified: Option<String>,
    pipeline_tag: Option<String>,
    downloads: i64,
    likes: i64,
    private: bool,
    gated: bool,
    tags: Vec<String>,
    library_name: Option<String>,
}

impl HfModel {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let id = value["id"].as_str()?.to_string();
        let created_at = value["createdAt"].as_str()?.to_string();
        Some(Self {
            id,
            author: value["author"].as_str().map(|s| s.to_string()),
            created_at,
            last_modified: value["lastModified"].as_str().map(|s| s.to_string()),
            pipeline_tag: value["pipeline_tag"].as_str().map(|s| s.to_string()),
            downloads: value["downloads"].as_i64().unwrap_or(0),
            likes: value["likes"].as_i64().unwrap_or(0),
            private: value["private"].as_bool().unwrap_or(false),
            gated: value["gated"].as_bool().unwrap_or(false),
            tags: value["tags"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            library_name: value["library_name"].as_str().map(|s| s.to_string()),
        })
    }

    /// 构造存入 `releases.body` 的元数据 JSON。
    fn metadata_json(&self) -> String {
        serde_json::json!({
            "pipeline_tag": self.pipeline_tag,
            "downloads": self.downloads,
            "likes": self.likes,
            "library_name": self.library_name,
            "tags": self.tags,
            "private": self.private,
            "gated": self.gated,
        })
        .to_string()
    }
}

/// 重试包装：403 不重试（HF 拒绝访问），其他可重试错误最多重试 3 次。
async fn with_retry<T, F, Fut>(f: F) -> Result<T, (u16, String)>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, (u16, String)>>,
{
    let config = crate::retry::RetryConfig::default();
    crate::retry::retry_with_backoff(&config, |e: &(u16, String)| {
        if e.0 == 403 {
            return false;
        }
        log::warn!("请求失败(状态={}), 将重试: {}", e.0, e.1);
        true
    }, f)
    .await
}

/// 从 Link header 中提取 `rel="next"` 的 URL（与 GitHub Link header 格式一致）。
fn parse_next_link(link_header: &str) -> Option<String> {
    for part in link_header.split(',') {
        let trimmed = part.trim();
        if trimmed.contains("rel=\"next\"") {
            let start = trimmed.find('<')?;
            let end = trimmed.find('>')?;
            return Some(trimmed[start + 1..end].to_string());
        }
    }
    None
}

/// 获取单页模型列表，返回 (models, 下一页 URL)。
async fn fetch_models_page(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let reason = resp.status().canonical_reason().unwrap_or("").to_string();
        return Err((status, format!("err.api_error|{}|{}", status, reason)));
    }

    let next_url = resp
        .headers()
        .get("link")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_next_link);

    let models: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| (status, format!("err.parse_failed|{}", e)))?;

    Ok((models, next_url))
}

async fn fetch_models_page_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    with_retry(|| async { fetch_models_page(client, url).await }).await
}

/// 构造按 createdAt 降序的单页请求 URL。
fn build_page_url(org: &str, limit: usize) -> String {
    format!(
        "{}/api/models?author={}&sort=createdAt&direction=-1&limit={}",
        HF_API_BASE, org, limit,
    )
}

/// 获取组织的模型列表（单页，按 createdAt 降序）。
///
/// 对应 `poll.rs::fetch_for_source_async` 的 huggingface 分支。
pub async fn fetch_org_models(
    client: &reqwest::Client,
    org: &str,
    limit: usize,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let url = build_page_url(org, limit);
    let (models, _) = fetch_models_page_with_retry(client, &url).await?;
    Ok(models)
}

/// 首次全量分页拉取（fetch_history 开启时）。
///
/// 复用 GitHub 的 Link header 翻页模式，按 createdAt 降序翻页直到达到 `max_count` 或无下一页。
/// 对应 `poll.rs::fetch_all_for_source_async` 的 huggingface 分支。
pub async fn fetch_all_org_models_with_limit(
    client: &reqwest::Client,
    org: &str,
    max_count: Option<usize>,
) -> Result<Vec<serde_json::Value>, (u16, String)> {
    let mut all = Vec::new();
    let mut url = build_page_url(org, 100);

    loop {
        let (models, next_url) = fetch_models_page_with_retry(client, &url).await?;
        let count = models.len();
        all.extend(models);
        log::info!(
            "分页拉取 {}: 获取 {} 条{}",
            url,
            count,
            next_url.as_ref().map(|_| "，还有下一页").unwrap_or("，已完成"),
        );

        if let Some(limit) = max_count {
            if all.len() >= limit {
                log::info!("已获取 {} 条，达到上限 {}，停止翻页", all.len(), limit);
                break;
            }
        }

        match next_url {
            Some(next) => url = next,
            None => break,
        }
    }

    Ok(all)
}

/// 验证组织是否存在（调 API 看能否正常返回 2xx）。
///
/// 注意：HF `/api/models` 对不存在的 author 也返回空数组 `[]`（不返回 404），
/// 因此这里只能验证"API 可达且返回正常"，无法区分"组织不存在"和"组织无公开模型"。
/// 成功后 description 由调用方用固定字符串生成，无需额外网络请求。
pub async fn verify_org_exists(
    client: &reqwest::Client,
    org: &str,
) -> Result<(), (u16, String)> {
    let url = format!("{}/api/models?author={}&limit=1", HF_API_BASE, org);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let reason = status.canonical_reason().unwrap_or("").to_string();
        return Err((code, format!("err.api_error|{}|{}", code, reason)));
    }
    // 确保响应体是合法 JSON 数组（区分网络异常/HTML 错误页与正常 API 响应）
    let _: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    Ok(())
}

/// 获取单个模型的 README（model card）作为人类可读内容。
///
/// HF 模型仓库根目录的 `README.md` 即 model card，是模型的主体描述文档，
/// 作用等同于 GitHub Release 的 release note。通过 `/raw/main/README.md` 获取。
/// 获取失败时返回 None（不阻塞保存流程，body 留空，译文/原文视图不显示）。
async fn fetch_readme(client: &reqwest::Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        log::warn!("获取 HF README 失败: 状态={}", resp.status());
        return None;
    }
    let text = resp.text().await.ok()?;
    if text.is_empty() {
        return None;
    }
    Some(text)
}

fn readme_url(model_id: &str) -> String {
    format!("https://huggingface.co/{}/raw/main/README.md", model_id)
}

/// 新插入的模型信息（insert 阶段产出，供 fetch README + finalize 使用）。
pub struct NewModel {
    pub id: i64,
    pub tag: String,
    pub metadata: String,
}

/// 阶段 1（同步）：解析模型列表、insert 到 releases 表，返回新插入的模型信息。
///
/// - `prerelease` 固定为 `false`（HF 模型无预发布概念）
/// - `published_at` 使用 `model.createdAt`
/// - insert 时 body 传 None，README 由阶段 2 异步拉取后由 `finalize_models` 回填
/// - 元数据 JSON 由 `finalize_models` 写入 `extra_metadata`
///
/// 行为与 `github::save_releases` 对齐：按 published_at 降序排列，`max_count=1` 时
/// 遇到已入库记录立即返回空 vec；历史模式跳过已存在记录继续。
pub fn insert_new_models(
    conn: &Connection,
    source_id: i64,
    models: &[serde_json::Value],
    max_count: usize,
) -> Vec<NewModel> {
    let mut parsed: Vec<HfModel> = models
        .iter()
        .filter_map(HfModel::from_json)
        .collect();
    parsed.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let mut inserted = Vec::new();
    for model in &parsed {
        if model.created_at.is_empty() {
            continue;
        }
        let tag = &model.id;
        let html_url = format!("https://huggingface.co/{}", tag);
        let metadata = model.metadata_json();
        if let Ok(id) = releases::insert_release(
            conn,
            source_id,
            tag,
            tag,
            &html_url,
            &model.created_at,
            false,
            None,
        ) {
            if id > 0 {
                inserted.push(NewModel {
                    id,
                    tag: tag.clone(),
                    metadata,
                });
                if inserted.len() >= max_count {
                    return inserted;
                }
                continue;
            }
        }
        // 已入库且普通模式（max_count=1）时，说明不是新模型，停止
        if max_count == 1 {
            return vec![];
        }
        // 历史模式：已存在的跳过，继续找更新的新模型
    }
    inserted
}

/// 阶段 2（异步）：并行拉取各模型的 README（model card）。
///
/// HF 模型仓库根目录的 `README.md` 即 model card，作用等同于 GitHub Release 的 release note。
/// 通过 `/raw/main/README.md` 获取。拉取失败返回 None（body 留空，译文/原文视图不显示）。
/// 并发度限制为 8，避免对 HF API 造成突发压力。
pub async fn fetch_readmes(
    client: &reqwest::Client,
    new_models: &[NewModel],
) -> Vec<Option<String>> {
    use std::collections::HashMap;
    use tokio::task::JoinSet;

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut set = JoinSet::new();
    for m in new_models {
        let url = readme_url(&m.tag);
        let client = client.clone();
        let sem = semaphore.clone();
        let tag = m.tag.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            let readme = fetch_readme(&client, &url).await;
            Some((tag, readme))
        });
    }

    let mut readmes: HashMap<String, Option<String>> = HashMap::new();
    while let Some(res) = set.join_next().await {
        if let Ok(Some((tag, readme))) = res {
            readmes.insert(tag, readme);
        }
    }

    new_models
        .iter()
        .map(|m| readmes.get(&m.tag).cloned().flatten())
        .collect()
}

/// 阶段 3（同步）：将 README 和元数据回填到 releases 表，返回 saved 列表。
///
/// 返回 `(id, body)` 对齐 `github::save_releases` 的输出，供 `mark_older_as_read` 使用。
pub fn finalize_models(
    conn: &Connection,
    new_models: Vec<NewModel>,
    readmes: Vec<Option<String>>,
) -> Vec<(i64, Option<String>)> {
    let mut saved = Vec::new();
    for (m, readme) in new_models.into_iter().zip(readmes) {
        let _ = releases::set_release_body_and_metadata(
            conn,
            m.id,
            readme.as_deref(),
            Some(&m.metadata),
        );
        saved.push((m.id, readme));
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path, query_param};

    fn sample_model(id: &str, created_at: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "author": "moonshotai",
            "createdAt": created_at,
            "pipeline_tag": "text-generation",
            "downloads": 100,
            "likes": 5,
            "private": false,
            "gated": false,
            "tags": ["transformers", "safetensors"],
            "library_name": "transformers",
        })
    }

    #[test]
    fn test_hf_model_from_json_full() {
        let v = sample_model("moonshotai/Kimi", "2024-06-01T00:00:00.000Z");
        let m = HfModel::from_json(&v).unwrap();
        assert_eq!(m.id, "moonshotai/Kimi");
        assert_eq!(m.created_at, "2024-06-01T00:00:00.000Z");
        assert_eq!(m.pipeline_tag.as_deref(), Some("text-generation"));
        assert_eq!(m.downloads, 100);
        assert_eq!(m.likes, 5);
        assert!(!m.private);
        assert!(!m.gated);
        assert_eq!(m.tags, vec!["transformers", "safetensors"]);
        assert_eq!(m.library_name.as_deref(), Some("transformers"));
    }

    #[test]
    fn test_hf_model_from_json_missing_id_returns_none() {
        let v = serde_json::json!({"createdAt": "2024-01-01T00:00:00Z"});
        assert!(HfModel::from_json(&v).is_none());
    }

    #[test]
    fn test_hf_model_from_json_missing_created_at_returns_none() {
        let v = serde_json::json!({"id": "org/model"});
        assert!(HfModel::from_json(&v).is_none());
    }

    #[test]
    fn test_hf_model_metadata_json_contains_fields() {
        let v = sample_model("org/m", "2024-01-01T00:00:00Z");
        let m = HfModel::from_json(&v).unwrap();
        let body = m.metadata_json();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["pipeline_tag"], "text-generation");
        assert_eq!(parsed["downloads"], 100);
        assert_eq!(parsed["likes"], 5);
        assert_eq!(parsed["library_name"], "transformers");
        assert_eq!(parsed["private"], false);
        assert_eq!(parsed["gated"], false);
        assert!(parsed["tags"].is_array());
    }

    #[test]
    fn test_parse_next_link_found() {
        let header = "<https://huggingface.co/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=2>; rel=\"next\", \
                       <https://huggingface.co/api/models?author=org&p=5>; rel=\"last\"";
        assert_eq!(
            parse_next_link(header).as_deref(),
            Some("https://huggingface.co/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=2")
        );
    }

    #[test]
    fn test_parse_next_link_not_found() {
        let header = "<https://huggingface.co/api/models?p=1>; rel=\"last\"";
        assert!(parse_next_link(header).is_none());
    }

    // ── insert_new_models + finalize_models 测试 ──
    // 用 helper 模拟三阶段但不拉取 README（readmes 传 None），聚焦入库与去重逻辑。

    fn model_value(id: &str, created_at: &str) -> serde_json::Value {
        sample_model(id, created_at)
    }

    /// 模拟 save 链路但跳过 README 拉取：insert 后以 None readmes finalize。
    fn insert_and_finalize(
        conn: &Connection,
        sid: i64,
        data: &[serde_json::Value],
        max_count: usize,
    ) -> Vec<(i64, Option<String>)> {
        let new_models = insert_new_models(conn, sid, data, max_count);
        let readmes: Vec<Option<String>> = new_models.iter().map(|_| None).collect();
        finalize_models(conn, new_models, readmes)
    }

    #[test]
    fn test_insert_new_models_max_count_1_saves_latest() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_value("org/m3", "2024-03-01T00:00:00Z"),
            model_value("org/m2", "2024-02-01T00:00:00Z"),
        ];
        let result = insert_and_finalize(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_insert_new_models_max_count_3_saves_all() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_value("org/m3", "2024-03-01T00:00:00Z"),
            model_value("org/m2", "2024-02-01T00:00:00Z"),
            model_value("org/m1", "2024-01-01T00:00:00Z"),
        ];
        let result = insert_and_finalize(&conn, sid, &data, 3);
        assert_eq!(result.len(), 3);
        // 最新在前
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let tags: Vec<String> = releases.iter().map(|r| r.tag_name.clone()).collect();
        assert!(tags.contains(&"org/m3".to_string()));
        assert!(tags.contains(&"org/m2".to_string()));
        assert!(tags.contains(&"org/m1".to_string()));
    }

    #[test]
    fn test_insert_new_models_max_count_1_existing_returns_empty() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_value("org/m3", "2024-03-01T00:00:00Z"),
            model_value("org/m2", "2024-02-01T00:00:00Z"),
        ];
        // 首次保存 m3
        let r1 = insert_and_finalize(&conn, sid, &data, 1);
        assert_eq!(r1.len(), 1);
        // 再次保存同样数据：m3 已存在 → 返回空
        let r2 = insert_and_finalize(&conn, sid, &data, 1);
        assert_eq!(r2.len(), 0);
    }

    #[test]
    fn test_insert_new_models_historical_skips_existing_and_continues() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_value("org/m3", "2024-03-01T00:00:00Z"),
            model_value("org/m2", "2024-02-01T00:00:00Z"),
            model_value("org/m1", "2024-01-01T00:00:00Z"),
        ];
        // 首次只保存 m3
        insert_and_finalize(&conn, sid, &data, 1);
        // 历史模式：m3 已存在跳过，m2/m1 新增
        let r2 = insert_and_finalize(&conn, sid, &data, 2);
        assert_eq!(r2.len(), 2);
    }

    #[test]
    fn test_insert_new_models_prerelease_always_false() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![model_value("org/m1", "2024-01-01T00:00:00Z")];
        insert_and_finalize(&conn, sid, &data, 1);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert!(!releases[0].prerelease, "HF 模型 prerelease 应固定为 false");
    }

    #[test]
    fn test_insert_new_models_html_url_correct() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![model_value("moonshotai/Kimi", "2024-01-01T00:00:00Z")];
        insert_and_finalize(&conn, sid, &data, 1);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(
            releases[0].html_url,
            "https://huggingface.co/moonshotai/Kimi"
        );
    }

    #[test]
    fn test_finalize_models_writes_metadata_and_body() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![model_value("org/m1", "2024-01-01T00:00:00Z")];
        // 模拟 finalize 传入 README 内容
        let new_models = insert_new_models(&conn, sid, &data, 1);
        assert_eq!(new_models.len(), 1);
        let readmes = vec![Some("# Kimi Model\n描述内容".to_string())];
        finalize_models(&conn, new_models, readmes);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].body.as_deref(), Some("# Kimi Model\n描述内容"));
        // extra_metadata 应包含元数据 JSON
        let meta = releases[0].extra_metadata.as_ref().unwrap();
        assert!(meta.contains("pipeline_tag"));
        assert!(meta.contains("downloads"));
        assert!(meta.contains("likes"));
    }

    // ── HTTP 测试 ──

    #[tokio::test]
    async fn test_fetch_org_models_200_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("author", "moonshotai"))
            .and(query_param("sort", "createdAt"))
            .and(query_param("direction", "-1"))
            .and(query_param("limit", "50"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([sample_model("org/m1", "2024-01-01T00:00:00Z")])),
            )
            .mount(&mock)
            .await;

        // 直接测 fetch_models_page 以注入 mock base
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models?author=moonshotai&sort=createdAt&direction=-1&limit=50", mock.uri());
        let (models, next) = fetch_models_page(&client, &url).await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["id"], "org/m1");
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn test_fetch_models_page_with_link_header() {
        let mock = MockServer::start().await;
        let page2 = format!("{}/api/models?author=org&p=2", mock.uri());
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([sample_model("org/m1", "2024-01-01T00:00:00Z")]))
                    .insert_header("link", format!("<{}>; rel=\"next\"", page2)),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models?author=org", mock.uri());
        let (_, next) = fetch_models_page(&client, &url).await.unwrap();
        assert_eq!(next.as_deref(), Some(page2.as_str()));
    }

    #[tokio::test]
    async fn test_fetch_models_page_403() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models?author=org", mock.uri());
        let r = fetch_models_page(&client, &url).await;
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().0, 403);
    }

    #[tokio::test]
    async fn test_verify_org_exists_success_empty_array() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("author", "emptyorg"))
            .and(query_param("limit", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&mock)
            .await;

        // 用 mock base 直接测 verify 逻辑
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models?author=emptyorg&limit=1", mock.uri());
        let resp = client.get(&url).send().await.unwrap();
        assert!(resp.status().is_success());
        let body: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert!(body.is_empty(), "空组织应返回空数组，视为验证成功");
    }

    #[tokio::test]
    async fn test_fetch_all_org_models_multi_page() {
        let mock = MockServer::start().await;
        let page2 = format!("{}/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=2", mock.uri());

        // 页 1：匹配 p=1，返回 link 指向 p=2
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("p", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([
                        sample_model("org/m1", "2024-03-01T00:00:00Z"),
                        sample_model("org/m2", "2024-02-01T00:00:00Z"),
                    ]))
                    .insert_header("link", format!("<{}>; rel=\"next\"", page2)),
            )
            .mount(&mock)
            .await;

        // 页 2：匹配 p=2，无 link header
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("p", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([
                        sample_model("org/m3", "2024-01-01T00:00:00Z"),
                    ])),
            )
            .mount(&mock)
            .await;

        // 用 mock base 调用内部翻页逻辑
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut all = Vec::new();
        let mut url = format!("{}/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=1", mock.uri());
        loop {
            let (models, next) = fetch_models_page(&client, &url).await.unwrap();
            all.extend(models);
            match next {
                Some(n) => url = n,
                None => break,
            }
        }
        assert_eq!(all.len(), 3);
    }

    // ── fetch_readme / fetch_readmes 测试 ──

    #[tokio::test]
    async fn test_fetch_readme_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/org/m1/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string("# Model\n描述"))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/org/m1/raw/main/README.md", mock.uri());
        let readme = fetch_readme(&client, &url).await;
        assert_eq!(readme.as_deref(), Some("# Model\n描述"));
    }

    #[tokio::test]
    async fn test_fetch_readme_404_returns_none() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/org/missing/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/org/missing/raw/main/README.md", mock.uri());
        let readme = fetch_readme(&client, &url).await;
        assert!(readme.is_none());
    }

    #[tokio::test]
    async fn test_fetch_readmes_parallel() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/org/m1/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string("README-1"))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/org/m2/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string("README-2"))
            .mount(&mock)
            .await;

        // fetch_readmes 内部用 readme_url() 构造真实 huggingface.co URL，
        // 测试无法重定向。这里直接验证并发拉取逻辑：用 mock URL 通过手动构造 NewModel
        // 并调用 fetch_readmes 不行（它内部用 readme_url）。改为验证 fetch_readme 并发安全性
        // 通过单独并发调用 fetch_readme。
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url1 = format!("{}/org/m1/raw/main/README.md", mock.uri());
        let url2 = format!("{}/org/m2/raw/main/README.md", mock.uri());
        let (r1, r2) = tokio::join!(
            fetch_readme(&client, &url1),
            fetch_readme(&client, &url2),
        );
        assert_eq!(r1.as_deref(), Some("README-1"));
        assert_eq!(r2.as_deref(), Some("README-2"));
    }

    #[test]
    fn test_readme_url_format() {
        assert_eq!(
            readme_url("moonshotai/Kimi"),
            "https://huggingface.co/moonshotai/Kimi/raw/main/README.md"
        );
    }
}
