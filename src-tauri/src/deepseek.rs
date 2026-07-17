use rusqlite::Connection;

use crate::crypto;
use crate::db;
use crate::db::settings::{
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_API_KEY,
    KEY_DEEPSEEK_PROXY_BYPASS, KEY_DEEPSEEK_PROMPT, KEY_DEEPSEEK_TRANSLATE_RELEASE, KEY_PROXY_URL,
    KEY_LANGUAGE,
    DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_PROMPT_EDITABLE,
    DEFAULT_DEEPSEEK_TRANSLATE_PROMPT, DEFAULT_DEEPSEEK_TRANSLATE_RELEASE,
    DEEPSEEK_PROMPT_FIXED_SUFFIX,
};

pub fn read_config(conn: &Connection) -> (bool, String, String, Option<String>, String) {
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
    let api_key = {
        let encrypted = db::settings::get_setting(conn, KEY_DEEPSEEK_API_KEY)
            .ok()
            .flatten()
            .filter(|v| !v.is_empty());
        match encrypted {
            Some(enc) => {
                if let Some((plain, new_v2)) = crypto::decrypt_with_migration(&enc) {
                    if let Some(new_val) = &new_v2 {
                        if let Err(e) = db::settings::set_setting(conn, KEY_DEEPSEEK_API_KEY, new_val) {
                            log::warn!("迁移 v1→v2 DeepSeek API Key 回写失败: {}", e);
                        }
                    }
                    Some(plain)
                } else {
                    None
                }
            }
            None => None,
        }
    };
    let prompt = db::settings::get_setting_str(conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE)
        .unwrap_or_else(|_| DEFAULT_DEEPSEEK_PROMPT_EDITABLE.to_string());
    (enabled, model, base_url, api_key, prompt)
}

/// 读取翻译开关与目标语言。返回 (translate_enabled, target_lang)。
/// target_lang 由 UI 语言推断：zh-CN → 中文，en-US → English，其他 → 中文。
pub fn read_translate_config(conn: &Connection) -> (bool, String) {
    let translate_enabled = db::settings::get_setting(conn, KEY_DEEPSEEK_TRANSLATE_RELEASE)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(DEFAULT_DEEPSEEK_TRANSLATE_RELEASE == "true");
    let language = db::settings::get_setting(conn, KEY_LANGUAGE)
        .ok()
        .flatten()
        .unwrap_or_default();
    let target_lang = match language.as_str() {
        "en-US" => "English".to_string(),
        _ => "中文".to_string(),
    };
    (translate_enabled, target_lang)
}

pub fn build_client(api_key: &str, proxy_url: &str, proxy_mode: &str) -> Result<reqwest::Client, String> {
    // DeepSeek 所有请求都打同一 API 域名，token 作为 default header 安全。
    crate::http::build_http_client(crate::http::HttpClientConfig {
        proxy_url,
        proxy_mode,
        bearer_token: Some(api_key),
        timeout_secs: 60,
        content_type_json: true,
        set_default_auth: true,
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
    prompt_template: &str,
    body_text: &str,
) -> Result<(String, String), String> {
    // 组装完整提示词：可编辑部分 + 固定 JSON 格式约束
    let editable = if prompt_template.is_empty() {
        DEFAULT_DEEPSEEK_PROMPT_EDITABLE.to_string()
    } else {
        prompt_template.to_string()
    };
    let full_prompt = format!("{}\n\n{}", editable, DEEPSEEK_PROMPT_FIXED_SUFFIX);
    let prompt = full_prompt.replace("{}", body_text);
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

/// 调用 AI 检测文本主体语言。仅取 body 前 500 字符以节省 token。
/// 返回 AI 判断的语言名称（如 "中文"/"English"/"日本語"）。
/// 用于翻译前判断是否需要翻译：若检测语言 == 目标语言则跳过。
async fn call_detect_language(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    text_sample: &str,
) -> Result<String, String> {
    let prompt = format!(
        "请判断以下文本的主体语言，仅用一个词回答语言名称（如 中文、English、日本語、Français 等），不要输出其他任何内容。\n\n文本：\n{}",
        text_sample
    );
    let body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.0,
        "max_tokens": 20
    });
    let config = crate::retry::RetryConfig::default();
    crate::retry::retry_with_backoff(&config, |e: &(u16, String)| {
        if e.0 == 429 {
            log::warn!("DeepSeek 语言检测限流(429), 将重试");
            return true;
        }
        false
    }, || async {
        let resp = client
            .post(format!("{}/v1/chat/completions", base_url.trim_end_matches('/')))
            .json(&body_json)
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
            if content.is_empty() {
                return Err((0, "语言检测结果为空".to_string()));
            }
            return Ok(content);
        }
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Err((status, format!("DeepSeek API 返回错误 {}: {}", status, text)))
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

/// 调用 AI 翻译 release note 全文。纯文本输出（非 JSON），
/// 保留 Markdown 结构。`target_lang` 为目标语言（如 "中文"/"English"）。
async fn call_translate(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    target_lang: &str,
    body_text: &str,
) -> Result<String, String> {
    let prompt = DEFAULT_DEEPSEEK_TRANSLATE_PROMPT
        .replace("{lang}", target_lang)
        .replace("{}", body_text);
    let body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.3,
        "max_tokens": 8000
    });

    let config = crate::retry::RetryConfig::default();
    crate::retry::retry_with_backoff(&config, |e: &(u16, String)| {
        if e.0 == 429 {
            log::warn!("DeepSeek 翻译限流(429), 将重试");
            return true;
        }
        false
    }, || async {
        let resp = client
            .post(format!("{}/v1/chat/completions", base_url.trim_end_matches('/')))
            .json(&body_json)
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
            if content.is_empty() {
                return Err((0, "翻译结果为空".to_string()));
            }
            return Ok(content);
        }
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Err((status, format!("DeepSeek API 返回错误 {}: {}", status, text)))
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
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
) {
    let (enabled, model, base_url, api_key, prompt, proxy_url, proxy_mode);

    {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("数据库连接失败: {}", e);
                return;
            }
        };
        let cfg = read_config(&conn);
        enabled = cfg.0;
        model = cfg.1;
        base_url = cfg.2;
        api_key = cfg.3;
        prompt = cfg.4;
        let bypass = db::settings::get_setting(&conn, KEY_DEEPSEEK_PROXY_BYPASS)
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(false);
        if bypass {
            proxy_url = String::new();
            proxy_mode = "none".to_string();
        } else {
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

    let semaphore = deepseek_semaphore.clone();
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
        let prompt = prompt.clone();
        let db = db_pool.clone();
        let db2 = db_pool.clone();
        let release_id = *release_id;
        let sem_clone = semaphore.clone();

        handles.push(tokio::spawn(async move {
            let _permit = match sem_clone.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("信号量获取失败: {}", e);
                    return;
                }
            };
            match call_summary(&client, &model, &base_url, &prompt, &truncated).await {
                Ok((summary, importance)) => {
                    // 同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
                    let _ = tokio::task::spawn_blocking(move || {
                        let conn = match db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("数据库连接失败: {}", e);
                                return;
                            }
                        };
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
                    })
                    .await;
                }
                Err(e) => {
                    log::error!("生成摘要失败 id={}: {}", release_id, e);
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(conn) = db2.get() {
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
                    })
                    .await;
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }
}

/// 为新增的 releases 生成全文翻译。与摘要任务共用 `deepseek_semaphore`，
/// 保证 AI 请求总并发不变。
/// - `force=false`：仅在 `deepseek_translate_release=true` 且已配置 API key 时生效（轮询自动场景）
/// - `force=true`：绕过 `translate_enabled` 开关，只要 AI 已启用且配置 key 即翻译（手动单条场景）
pub async fn generate_translations_for_new(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    force: bool,
) {
    let (enabled, model, base_url, api_key, proxy_url, proxy_mode, translate_enabled, target_lang);

    {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("数据库连接失败: {}", e);
                return;
            }
        };
        let cfg = read_config(&conn);
        enabled = cfg.0;
        model = cfg.1;
        base_url = cfg.2;
        api_key = cfg.3;
        // cfg.4 = prompt，翻译不使用用户可编辑的摘要 prompt
        let bypass = db::settings::get_setting(&conn, KEY_DEEPSEEK_PROXY_BYPASS)
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(false);
        if bypass {
            proxy_url = String::new();
            proxy_mode = "none".to_string();
        } else {
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
        let (t_enabled, t_lang) = read_translate_config(&conn);
        translate_enabled = t_enabled;
        target_lang = t_lang;
    }

    if !enabled || (!translate_enabled && !force) {
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

    let semaphore = deepseek_semaphore.clone();
    let mut handles = Vec::new();
    for (release_id, body) in saved {
        let body_text = match body {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        // 翻译全文比摘要耗 token 多，放宽截断上限到 12000 字符
        let truncated: String = body_text.chars().take(12000).collect();
        let client = client.clone();
        let model = model.clone();
        let base_url = base_url.clone();
        let target_lang = target_lang.clone();
        let db = db_pool.clone();
        let db2 = db_pool.clone();
        let release_id = *release_id;
        let sem_clone = semaphore.clone();

        handles.push(tokio::spawn(async move {
            let _permit = match sem_clone.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("信号量获取失败: {}", e);
                    return;
                }
            };
            // 语言检测短路：取 body 前 500 字符让 AI 判断主体语言，
            // 若与目标语言一致则直接把原文写入 body_translated，跳过翻译调用。
            // 检测失败时不阻塞翻译（视为语言不一致，照常翻译）。
            let sample: String = truncated.chars().take(500).collect();
            if let Ok(detected) = call_detect_language(&client, &model, &base_url, &sample).await {
                if detected.trim() == target_lang {
                    // 同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
                    let target_lang_log = target_lang.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        let conn = match db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("数据库连接失败: {}", e);
                                return;
                            }
                        };
                        if let Err(e) = db::releases::set_body_translated(&conn, release_id, &truncated) {
                            log::error!("保存译文失败 id={}: {}", release_id, e);
                        } else {
                            log::info!("跳过翻译(语言一致): id={} lang={}", release_id, target_lang_log);
                        }
                    })
                    .await;
                    return;
                }
            }
            match call_translate(&client, &model, &base_url, &target_lang, &truncated).await {
                Ok(translated) => {
                    let _ = tokio::task::spawn_blocking(move || {
                        let conn = match db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("数据库连接失败: {}", e);
                                return;
                            }
                        };
                        if let Err(e) = db::releases::set_body_translated(&conn, release_id, &translated) {
                            log::error!("保存译文失败 id={}: {}", release_id, e);
                        } else {
                            let rel = db::releases::get_release(&conn, release_id).ok().flatten();
                            match rel {
                                Some(r) => db::logs::write_log(
                                    &conn,
                                    "INFO",
                                    &format!("AI 译文已生成: {}/{} {}", r.owner, r.repo, r.tag_name),
                                ),
                                None => db::logs::write_log(
                                    &conn,
                                    "INFO",
                                    &format!("AI 译文已生成: id={}", release_id),
                                ),
                            }
                        }
                    })
                    .await;
                }
                Err(e) => {
                    log::error!("生成译文失败 id={}: {}", release_id, e);
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(conn) = db2.get() {
                            let _ = db::releases::increment_translate_retry_count(&conn, release_id);
                            let rel = db::releases::get_release(&conn, release_id).ok().flatten();
                            match rel {
                                Some(r) => db::logs::write_log(
                                    &conn,
                                    "ERROR",
                                    &format!("AI 译文生成失败: {}/{} {}: {}", r.owner, r.repo, r.tag_name, e),
                                ),
                                None => db::logs::write_log(
                                    &conn,
                                    "ERROR",
                                    &format!("AI 译文生成失败: id={}: {}", release_id, e),
                                ),
                            }
                        }
                    })
                    .await;
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
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
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
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
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
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
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
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
        assert!(result.is_err());
        // 非429不重试，错误不应包含"重试"
        assert!(!result.unwrap_err().contains("重试"));
    }

    // ── 翻译任务测试 ──────────────────────────────────────

    fn sample_translate_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": "这是译文内容"
                }
            }]
        })
    }

    #[tokio::test]
    async fn test_call_translate_200_success() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_translate(&client, "test-model", &mock.uri(), "中文", "Some release body").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "这是译文内容");
    }

    #[tokio::test]
    async fn test_call_translate_429_then_200() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_translate(&client, "test-model", &mock.uri(), "中文", "body").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_call_translate_empty_content_errors() {
        let mock = MockServer::start().await;
        let empty_resp = serde_json::json!({
            "choices": [{ "message": { "content": "   " } }]
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_resp))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_translate(&client, "test-model", &mock.uri(), "中文", "body").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("为空"));
    }

    #[test]
    fn test_read_translate_config_defaults_disabled() {
        use crate::db::init::init_memory_db;
        let conn = init_memory_db().unwrap();
        let (enabled, lang) = read_translate_config(&conn);
        assert!(!enabled, "默认未启用翻译");
        // 无 language 设置时 fallback 为中文
        assert_eq!(lang, "中文");
    }

    #[test]
    fn test_read_translate_config_enabled_and_lang() {
        use crate::db::init::init_memory_db;
        use crate::db::settings;
        let conn = init_memory_db().unwrap();
        settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
        settings::set_setting(&conn, KEY_LANGUAGE, "en-US").unwrap();
        let (enabled, lang) = read_translate_config(&conn);
        assert!(enabled);
        assert_eq!(lang, "English");
    }

    // ── 语言检测测试 ────────────────────────────────

    fn sample_detect_response(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": lang
                }
            }]
        })
    }

    #[tokio::test]
    async fn test_call_detect_language_english() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("English")))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_detect_language(&client, "test-model", &mock.uri(), "Fixed a bug").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "English");
    }

    #[tokio::test]
    async fn test_call_detect_language_chinese() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("中文")))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_detect_language(&client, "test-model", &mock.uri(), "修复了一个问题").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "中文");
    }

    #[tokio::test]
    async fn test_call_detect_language_empty_content_errors() {
        let mock = MockServer::start().await;
        let empty_resp = serde_json::json!({
            "choices": [{ "message": { "content": "" } }]
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_resp))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_detect_language(&client, "test-model", &mock.uri(), "some text").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("为空"));
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

    // ── 编排函数 generate_summaries_for_new / generate_translations_for_new 集成测试 ──
    //
    // 注入 init_memory_pool + wiremock，直接驱动真实的公开编排函数，覆盖
    // enabled / api_key / 成功写摘要 / 失败重试计数 / 空体跳过 /
    // 翻译 force 绕过开关 / 语言检测短路写原文 / 翻译失败重试 等编排分支。
    // 这条链路原先无测试覆盖（CI 不可达），至此补齐。

    fn enable_deepseek(conn: &rusqlite::Connection, base_url: &str) {
        crate::crypto::set_test_master_key();
        db::settings::set_setting(conn, KEY_DEEPSEEK_ENABLED, "true").unwrap();
        db::settings::set_setting(conn, KEY_DEEPSEEK_BASE_URL, base_url).unwrap();
        db::settings::set_setting(conn, KEY_DEEPSEEK_API_KEY, &crate::crypto::encrypt("test-key")).unwrap();
    }

    fn insert_release_with_body(conn: &rusqlite::Connection, body: &str) -> i64 {
        let sid = db::sources::add_source(conn, "github", "o", "r", "").unwrap();
        db::releases::insert_release(
            conn, sid, "v1", "v1", "https://github.com/o/r/releases/tag/v1",
            "2024-01-01T00:00:00Z", false, Some(body),
        ).unwrap()
    }

    fn retry_count(conn: &rusqlite::Connection, id: i64) -> i64 {
        conn.query_row(
            "SELECT COALESCE(retry_count, 0) FROM releases WHERE id = ?1",
            rusqlite::params![id], |r| r.get(0),
        ).unwrap()
    }

    fn translate_retry_count(conn: &rusqlite::Connection, id: i64) -> i64 {
        conn.query_row(
            "SELECT COALESCE(translate_retry_count, 0) FROM releases WHERE id = ?1",
            rusqlite::params![id], |r| r.get(0),
        ).unwrap()
    }

    /// 未启用 DeepSeek：编排函数应早返回，不发起任何请求、不写摘要。
    #[tokio::test]
    async fn test_generate_summaries_disabled_no_call() {
        let _mock = MockServer::start().await; // 不挂 mock：若误调用会落到错误分支
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("body".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none(), "未启用 AI 不应写摘要");
        assert_eq!(retry_count(&conn, id), 0, "未启用不应触发请求");
    }

    /// 已启用但未配置 api_key：应早返回，不发起请求。
    #[tokio::test]
    async fn test_generate_summaries_no_api_key_no_call() {
        let _mock = MockServer::start().await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            db::settings::set_setting(&conn, KEY_DEEPSEEK_ENABLED, "true").unwrap();
            // 故意不设置 api_key
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("body".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none());
        assert_eq!(retry_count(&conn, id), 0, "无 api_key 不应触发请求");
    }

    /// 成功路径：mock 200 返回摘要 JSON，应写回 ai_summary / ai_importance 并重置 retry_count。
    #[tokio::test]
    async fn test_generate_summaries_success_writes_summary() {
        let mock = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "release body content")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("release body content".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(rel.ai_summary.as_deref(), Some("测试摘要内容"));
        assert_eq!(rel.ai_importance.as_deref(), Some("中"));
        assert_eq!(retry_count(&conn, id), 0, "成功后 retry_count 应被重置");
    }

    /// 失败路径：mock 500（非 429 不重试），应递增 retry_count 且不写摘要。
    #[tokio::test]
    async fn test_generate_summaries_error_increments_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("release body".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none(), "失败不应写摘要");
        assert!(retry_count(&conn, id) >= 1, "失败应递增 retry_count");
    }

    /// body 为 None：应被 continue 跳过，不发起请求（retry_count 保持 0 证明未触发错误分支）。
    #[tokio::test]
    async fn test_generate_summaries_skips_none_body() {
        let mock = MockServer::start().await; // 无 mock
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, None)]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none());
        assert_eq!(retry_count(&conn, id), 0, "None body 不应触发请求");
    }

    /// 翻译：translate 未启用且 force=false → 早返回，不翻译、不触发请求。
    #[tokio::test]
    async fn test_generate_translations_disabled_no_force_noop() {
        let mock = MockServer::start().await; // 无 mock
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            // KEY_DEEPSEEK_TRANSLATE_RELEASE 默认 false
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("body".to_string()))], false).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.body_translated.is_none(), "translate 未启用且非 force 不应翻译");
        assert_eq!(translate_retry_count(&conn, id), 0);
    }

    /// 翻译：force=true 绕过 translate 开关。detect 返回非目标语言 → 继续翻译并写译文。
    #[tokio::test]
    async fn test_generate_translations_force_bypasses_switch() {
        let mock = MockServer::start().await;
        // 单 mock：detect 得 "English"(≠ 默认中文 → 继续翻译)，translate 得 "English" 作为译文
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("English")))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            // translate_release 未启用，靠 force=true 绕过
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("release body".to_string()))], true).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.body_translated.is_some(), "force=true 应绕过开关执行翻译");
        assert_eq!(translate_retry_count(&conn, id), 0);
    }

    /// 翻译：语言检测 == 目标语言 → 短路跳过翻译，直接写原文为 body_translated。
    #[tokio::test]
    async fn test_generate_translations_language_match_writes_original() {
        let mock = MockServer::start().await;
        // detect 返回 "中文" == 默认 target_lang("中文") → 跳过翻译，写原文
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("中文")))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            db::settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
            insert_release_with_body(&conn, "原文内容")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("原文内容".to_string()))], false).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(rel.body_translated.as_deref(), Some("原文内容"), "语言一致应直接写原文跳过翻译");
    }

    /// 翻译失败：detect 与 translate 均 500（detect 失败不阻塞翻译），应递增 translate_retry_count。
    #[tokio::test]
    async fn test_generate_translations_error_increments_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            db::settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("release body".to_string()))], false).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.body_translated.is_none(), "翻译失败不应写译文");
        assert!(translate_retry_count(&conn, id) >= 1, "翻译失败应递增 translate_retry_count");
    }
}
