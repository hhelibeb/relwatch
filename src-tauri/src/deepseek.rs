use rusqlite::Connection;
use tauri::Manager;

use crate::crypto;
use crate::db;
use crate::db::settings::{
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_API_KEY,
    KEY_PROXY_URL,
    DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_BASE_URL,
};
use crate::types::AppState;

pub fn read_config(conn: &Connection) -> (bool, String, String, Option<String>) {
    let enabled = db::settings::get_setting(conn, KEY_DEEPSEEK_ENABLED)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    let model = db::settings::get_setting(conn, KEY_DEEPSEEK_MODEL)
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_DEEPSEEK_MODEL.to_string());
    let base_url = db::settings::get_setting(conn, KEY_DEEPSEEK_BASE_URL)
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_DEEPSEEK_BASE_URL.to_string());
    let encrypted_key = db::settings::get_setting(conn, KEY_DEEPSEEK_API_KEY)
        .ok()
        .flatten()
        .filter(|v| !v.is_empty());
    let api_key = encrypted_key.and_then(|v| crypto::decrypt(&v));
    (enabled, model, base_url, api_key)
}

pub fn build_client(api_key: &str, proxy_url: &str, proxy_mode: &str) -> Result<reqwest::Client, String> {
    crate::http::build_http_client(crate::http::HttpClientConfig {
        proxy_url,
        proxy_mode,
        bearer_token: Some(api_key),
        timeout_secs: 60,
        content_type_json: true,
    })
}

async fn call_summary_inner(
    client: &reqwest::Client,
    _model: &str,
    base_url: &str,
    body_json: &serde_json::Value,
) -> Result<(String, String), (u16, String)> {
    let resp = client
        .post(format!(
            "{}/v1/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .json(body_json)
        .send()
        .await
        .map_err(|e| (0, format!("DeepSeek 请求失败: {}", e)))?;
    if resp.status().is_success() {
        let json: serde_json::Value = resp.json().await.map_err(|e| (0, format!("解析响应失败: {}", e)))?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| (0, format!("解析摘要 JSON 失败: {} — 原始内容: {}", e, content)))?;
        let summary = parsed["summary"].as_str().unwrap_or("").to_string();
        let importance = parsed["importance"].as_str().unwrap_or("中").to_string();
        if summary.is_empty() {
            return Err((0, "摘要为空".to_string()));
        }
        return Ok((summary, importance));
    }

    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let msg = format!("DeepSeek API 返回错误 {}: {}", status, text);
    Err((status, msg))
}

async fn call_summary(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    body_text: &str,
) -> Result<(String, String), String> {
    let prompt = format!(
        "你是一个软件版本发布摘要助手。请用中文总结下面 GitHub Release 更新内容，并评估重要度。\n\
         重要度标准：\n\
         - 大：breaking changes、重大架构变更、严重安全漏洞修复\n\
         - 中：新功能、重要 bug 修复、性能优化\n\
         - 小：小修复、文档更新、依赖升级、日常维护\n\n\
         Release 内容：\n{}\n\n\
         请严格按以下 JSON 格式返回（不要包含其他内容）：\n\
         {{\"summary\":\"简短中文摘要，2-3句话\",\"importance\":\"大|中|小\"}}",
        body_text
    );
    let body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.3,
        "max_tokens": 800,
        "response_format": {"type": "json_object"}
    });

    let config = crate::retry::RetryConfig::default();
    crate::retry::retry_with_backoff(&config, |e: &(u16, String)| {
        if e.0 == 429 {
            log::warn!("DeepSeek 限流(429), 将重试");
            return true;
        }
        false
    }, || async {
        call_summary_inner(client, model, base_url, &body_json).await
    })
    .await
    .map_err(|(status, msg)| {
        if status > 0 {
            format!("[{}] {}", status, msg)
        } else {
            msg
        }
    })
}

pub async fn generate_summaries_for_new(
    app: &tauri::AppHandle,
    saved: &[(i64, Option<String>)],
) {
    let (enabled, model, base_url, api_key, proxy_url, proxy_mode);

    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        let cfg = read_config(&conn);
        enabled = cfg.0;
        model = cfg.1;
        base_url = cfg.2;
        api_key = cfg.3;
        proxy_url = db::settings::get_setting(&conn, KEY_PROXY_URL)
            .ok()
            .flatten()
            .unwrap_or_default();
        proxy_mode = db::settings::get_setting(&conn, db::settings::KEY_PROXY_MODE)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
            });
    }

    if !enabled {
        return;
    }
    let api_key = match api_key {
        Some(k) => k,
        None => return,
    };
    let client = match build_client(&api_key, &proxy_url, &proxy_mode) {
        Ok(c) => c,
        Err(e) => {
            log::error!("创建 DeepSeek 客户端失败: {}", e);
            return;
        }
    };

    let semaphore: std::sync::Arc<tokio::sync::Semaphore>;
    {
        let state = app.state::<AppState>();
        semaphore = state.deepseek_semaphore.clone();
    }
    let mut handles = Vec::new();
    for (release_id, body) in saved {
        let body_text = match body {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        let truncated: String = body_text.chars().take(4000).collect();
        let client = client.clone();
        let model = model.clone();
        let base_url = base_url.clone();
        let app = app.clone();
        let release_id = *release_id;
        let sem_clone = semaphore.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem_clone.acquire_owned().await.unwrap();
            match call_summary(&client, &model, &base_url, &truncated).await {
                Ok((summary, importance)) => {
                    let state = app.state::<AppState>();
                    let conn = state.db.get().unwrap();
                    if let Err(e) = db::releases::set_ai_summary(
                        &conn, release_id, &summary, &importance,
                    ) {
                        log::error!("保存摘要失败 id={}: {}", release_id, e);
                    } else {
                        let rel = db::releases::get_release(&conn, release_id).ok().flatten();
                        match rel {
                            Some(r) => db::logs::write_log(
                                &conn,
                                "INFO",
                                &format!("AI 摘要已生成: {}/{} {} 重要度={}", r.owner, r.repo, r.tag_name, importance),
                            ),
                            None => db::logs::write_log(
                                &conn,
                                "INFO",
                                &format!("AI 摘要已生成: id={} 重要度={}", release_id, importance),
                            ),
                        }
                    }
                }
                Err(e) => {
                    log::error!("生成摘要失败 id={}: {}", release_id, e);
                    let state = app.state::<AppState>();
                    if let Ok(conn) = state.db.get() {
                        let _ = db::releases::increment_retry_count(&conn, release_id);
                        let rel = db::releases::get_release(&conn, release_id).ok().flatten();
                        match rel {
                            Some(r) => db::logs::write_log(
                                &conn,
                                "ERROR",
                                &format!("AI 摘要生成失败: {}/{} {}: {}", r.owner, r.repo, r.tag_name, e),
                            ),
                            None => db::logs::write_log(
                                &conn,
                                "ERROR",
                                &format!("AI 摘要生成失败: id={}: {}", release_id, e),
                            ),
                        }
                    }
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": r#"{"summary":"测试摘要内容","importance":"中"}"#
                }
            }]
        })
    }

    #[tokio::test]
    async fn test_call_summary_200_success() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), "Some release body").await;
        assert!(result.is_ok());
        let (summary, importance) = result.unwrap();
        assert_eq!(summary, "测试摘要内容");
        assert_eq!(importance, "中");
    }

    #[tokio::test]
    async fn test_call_summary_429_then_200() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), "Some release body").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_call_summary_429_exhausted() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), "Some release body").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("429"));
    }

    #[tokio::test]
    async fn test_call_summary_400_no_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), "Some release body").await;
        assert!(result.is_err());
        // 非429不重试，错误不应包含"重试"
        assert!(!result.unwrap_err().contains("重试"));
    }

    // ── Semaphore 并发限制测试 ────────────────────────────────────

    #[tokio::test]
    async fn test_semaphore_limits_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // 模拟生产代码的 Semaphore 模式：acquire_owned 在 spawn 内部
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let s = sem.clone();
            let p = peak.clone();
            let a = active.clone();

            handles.push(tokio::spawn(async move {
                // acquire 在 spawn 内部，与生产代码模式一致
                let _permit = s.acquire_owned().await.unwrap();
                let v = a.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(v, Ordering::SeqCst);
                // 保持一段时间让其他任务有机会同时运行
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                a.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let max = peak.load(Ordering::SeqCst);
        assert!(max <= 2, "并发峰值 {} 不应超过信号量限制 2", max);
        assert_eq!(max, 2, "应能同时运行 2 个任务");
    }

    #[tokio::test]
    async fn test_semaphore_single_permit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let sem = Arc::new(tokio::sync::Semaphore::new(1));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..3 {
            let s = sem.clone();
            let p = peak.clone();
            let a = active.clone();

            handles.push(tokio::spawn(async move {
                let _permit = s.acquire_owned().await.unwrap();
                let v = a.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(v, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                a.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let max = peak.load(Ordering::SeqCst);
        assert!(max <= 1, "并发峰值 {} 不应超过信号量限制 1", max);
        assert_eq!(max, 1, "应严格串行执行");
    }
}
