use rusqlite::Connection;

use crate::db;
use crate::db::releases;
use crate::db::sources::Source;
use crate::error::SourceError;
use crate::http;
use crate::source::{FetchedRelease, SourceAdapter, SourceExtraMetadata};

const HF_API_BASE: &str = "https://huggingface.co";
const MAX_README_BYTES: usize = 4 * 1024 * 1024;

pub struct HuggingFaceAdapter;

#[async_trait::async_trait]
impl SourceAdapter for HuggingFaceAdapter {
    fn source_type(&self) -> &'static str {
        "huggingface"
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        _token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        fetch_org_models(client, &source.owner, per_page).await
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        _token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        fetch_all_org_models_with_limit(client, &source.owner, max_count).await
    }

    async fn save(
        &self,
        db: &r2d2::Pool<crate::db::SqliteConnectionManager>,
        source: &Source,
        data: &[FetchedRelease],
        max_count: usize,
        client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
        save_huggingface_models(db, source, data, max_count, client).await
    }

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        _repo: &str,
        _token: Option<&str>,
    ) -> Result<String, SourceError> {
        verify_org_exists(client, owner).await?;
        Ok(format!("HuggingFace organization: {}", owner))
    }
}

#[derive(Debug, Clone)]
struct HfModel {
    id: String,
    created_at: String,
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
            created_at,
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

    fn into_fetched_release(self) -> FetchedRelease {
        let html_url = format!("https://huggingface.co/{}", self.id);
        FetchedRelease {
            tag: self.id.clone(),
            name: self.id,
            html_url,
            published: self.created_at,
            prerelease: false,
            body: None,
            extra: SourceExtraMetadata::HuggingFace {
                pipeline_tag: self.pipeline_tag,
                downloads: self.downloads,
                likes: self.likes,
                library_name: self.library_name,
                tags: self.tags,
                private: self.private,
                gated: self.gated,
            },
        }
    }
}

async fn fetch_models_page(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<FetchedRelease>, Option<String>), SourceError> {
    let (raw, next) = http::fetch_page_with_retry(client, url, None).await?;
    let models = raw
        .iter()
        .filter_map(HfModel::from_json)
        .map(HfModel::into_fetched_release)
        .collect();
    Ok((models, next))
}

fn build_page_url(org: &str, limit: usize) -> String {
    format!(
        "{}/api/models?author={}&sort=createdAt&direction=-1&limit={}",
        HF_API_BASE, org, limit,
    )
}

async fn save_huggingface_models(
    db: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    source: &db::sources::Source,
    data: &[FetchedRelease],
    max_count: usize,
    client: &reqwest::Client,
) -> Vec<(i64, Option<String>)> {
    let new_models = {
        let pool = db.clone();
        let source_id = source.id;
        let data = data.to_vec();
        let inserted = tokio::task::spawn_blocking(move || match pool.get() {
            Ok(conn) => insert_new_models(&conn, source_id, &data, max_count),
            Err(e) => {
                log::error!("err.db_lock|{}", e);
                vec![]
            }
        })
        .await;
        match inserted {
            Ok(models) => models,
            Err(e) => {
                log::error!("huggingface save spawn_blocking panic: {}", e);
                return vec![];
            }
        }
    };
    if new_models.is_empty() {
        return vec![];
    }
    let readmes = fetch_readmes(client, &new_models).await;
    let pool = db.clone();
    let finalized = tokio::task::spawn_blocking(move || match pool.get() {
        Ok(conn) => finalize_models(&conn, new_models, readmes),
        Err(e) => {
            log::error!("err.db_lock|{}", e);
            vec![]
        }
    })
    .await;
    match finalized {
        Ok(saved) => saved,
        Err(e) => {
            log::error!("huggingface save spawn_blocking panic: {}", e);
            vec![]
        }
    }
}

pub async fn fetch_org_models(
    client: &reqwest::Client,
    org: &str,
    limit: usize,
) -> Result<Vec<FetchedRelease>, SourceError> {
    let url = build_page_url(org, limit);
    let (models, _) = fetch_models_page(client, &url).await?;
    Ok(models)
}

pub async fn fetch_all_org_models_with_limit(
    client: &reqwest::Client,
    org: &str,
    max_count: Option<usize>,
) -> Result<Vec<FetchedRelease>, SourceError> {
    let first_url = build_page_url(org, 100);
    let raw = http::paginated_fetch(client, first_url, max_count, None).await?;
    Ok(raw
        .iter()
        .filter_map(HfModel::from_json)
        .map(HfModel::into_fetched_release)
        .collect())
}

pub async fn verify_org_exists(client: &reqwest::Client, org: &str) -> Result<(), SourceError> {
    let url = format!("{}/api/models?author={}&limit=1", HF_API_BASE, org);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| crate::http::describe_request_source_error(&e))?;
    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let reason = status.canonical_reason().unwrap_or("").to_string();
        return Err(SourceError::api_error(code, reason));
    }
    let _: Vec<serde_json::Value> =
        crate::http::read_json_limited(resp, crate::http::MAX_JSON_BYTES)
            .await
            .map_err(|e| SourceError::raw(0, e))?;
    Ok(())
}

async fn fetch_readme(client: &reqwest::Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        log::warn!("获取 HF README 失败: 状态={}", resp.status());
        return None;
    }
    let bytes = crate::http::read_body_limited(resp, MAX_README_BYTES)
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if text.is_empty() {
        return None;
    }
    Some(text)
}

fn readme_url(model_id: &str) -> String {
    format!("https://huggingface.co/{}/raw/main/README.md", model_id)
}

pub struct NewModel {
    pub id: i64,
    pub tag: String,
    pub metadata: String,
}

pub fn insert_new_models(
    conn: &Connection,
    source_id: i64,
    models: &[FetchedRelease],
    max_count: usize,
) -> Vec<NewModel> {
    let entries: Vec<crate::db::save::SaveEntry> = models
        .iter()
        .filter(|m| !m.published.is_empty())
        .map(FetchedRelease::to_save_entry)
        .collect();

    let mut inserted = Vec::new();
    crate::db::save::save_entries_generic(
        conn,
        source_id,
        &entries,
        max_count,
        |_conn, id, entry| {
            inserted.push(NewModel {
                id,
                tag: entry.tag.clone(),
                metadata: entry.metadata.clone().unwrap_or_default(),
            });
        },
        |_, _, _| {},
    );
    inserted
}

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
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    fn model_release(id: &str, created_at: &str) -> FetchedRelease {
        HfModel::from_json(&sample_model(id, created_at))
            .unwrap()
            .into_fetched_release()
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
        let rel = model_release("org/m", "2024-01-01T00:00:00Z");
        let body = rel.extra.to_metadata_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["pipeline_tag"], "text-generation");
        assert_eq!(parsed["downloads"], 100);
        assert_eq!(parsed["likes"], 5);
        assert_eq!(parsed["library_name"], "transformers");
        assert_eq!(parsed["private"], false);
        assert_eq!(parsed["gated"], false);
        assert!(parsed["tags"].is_array());
    }

    fn insert_and_finalize(
        conn: &Connection,
        sid: i64,
        data: &[FetchedRelease],
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
            model_release("org/m3", "2024-03-01T00:00:00Z"),
            model_release("org/m2", "2024-02-01T00:00:00Z"),
        ];
        let result = insert_and_finalize(&conn, sid, &data, 1);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_insert_new_models_max_count_3_saves_all() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_release("org/m3", "2024-03-01T00:00:00Z"),
            model_release("org/m2", "2024-02-01T00:00:00Z"),
            model_release("org/m1", "2024-01-01T00:00:00Z"),
        ];
        let result = insert_and_finalize(&conn, sid, &data, 3);
        assert_eq!(result.len(), 3);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        let tags: Vec<String> = releases.iter().map(|r| r.tag_name.clone()).collect();
        assert!(tags.contains(&"org/m3".to_string()));
        assert!(tags.contains(&"org/m2".to_string()));
        assert!(tags.contains(&"org/m1".to_string()));
    }

    #[test]
    fn test_insert_new_models_write_cap_backfills_older_unsaved_model() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_release("org/m3", "2024-03-01T00:00:00Z"),
            model_release("org/m2", "2024-02-01T00:00:00Z"),
        ];
        let r1 = insert_and_finalize(&conn, sid, &data, 1);
        assert_eq!(r1.len(), 1);
        let r2 = insert_and_finalize(&conn, sid, &data, 1);
        assert_eq!(r2.len(), 1);
        let mut tags: Vec<String> = db::releases::get_releases_with_state(&conn)
            .unwrap()
            .into_iter()
            .map(|r| r.tag_name)
            .collect();
        tags.sort();
        assert_eq!(tags, vec!["org/m2".to_string(), "org/m3".to_string()]);
    }

    #[test]
    fn test_insert_new_models_historical_skips_existing_and_continues() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![
            model_release("org/m3", "2024-03-01T00:00:00Z"),
            model_release("org/m2", "2024-02-01T00:00:00Z"),
            model_release("org/m1", "2024-01-01T00:00:00Z"),
        ];
        insert_and_finalize(&conn, sid, &data, 1);
        let r2 = insert_and_finalize(&conn, sid, &data, 2);
        assert_eq!(r2.len(), 2);
    }

    #[test]
    fn test_insert_new_models_prerelease_always_false() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![model_release("org/m1", "2024-01-01T00:00:00Z")];
        insert_and_finalize(&conn, sid, &data, 1);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert!(!releases[0].prerelease);
    }

    #[test]
    fn test_insert_new_models_html_url_correct() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![model_release("moonshotai/Kimi", "2024-01-01T00:00:00Z")];
        insert_and_finalize(&conn, sid, &data, 1);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].html_url, "https://huggingface.co/moonshotai/Kimi");
    }

    #[test]
    fn test_finalize_models_writes_metadata_and_body() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "huggingface", "moonshotai", "", "").unwrap();
        let data = vec![model_release("org/m1", "2024-01-01T00:00:00Z")];
        let new_models = insert_new_models(&conn, sid, &data, 1);
        assert_eq!(new_models.len(), 1);
        let readmes = vec![Some("# Kimi Model\n描述内容".to_string())];
        finalize_models(&conn, new_models, readmes);
        let releases = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(releases[0].body.as_deref(), Some("# Kimi Model\n描述内容"));
        let meta = releases[0].extra_metadata.as_ref().unwrap();
        assert!(meta.contains("pipeline_tag"));
        assert!(meta.contains("downloads"));
        assert!(meta.contains("likes"));
    }

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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models?author=moonshotai&sort=createdAt&direction=-1&limit=50", mock.uri());
        let (models, next) = fetch_models_page(&client, &url).await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].tag, "org/m1");
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
        assert_eq!(r.unwrap_err().status(), 403);
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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/api/models?author=emptyorg&limit=1", mock.uri());
        let resp = client.get(&url).send().await.unwrap();
        assert!(resp.status().is_success());
        let body: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn test_fetch_all_org_models_multi_page() {
        let mock = MockServer::start().await;
        let page2 = format!("{}/api/models?author=org&sort=createdAt&direction=-1&limit=100&p=2", mock.uri());

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

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url1 = format!("{}/org/m1/raw/main/README.md", mock.uri());
        let url2 = format!("{}/org/m2/raw/main/README.md", mock.uri());
        let (r1, r2) = tokio::join!(fetch_readme(&client, &url1), fetch_readme(&client, &url2));
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
