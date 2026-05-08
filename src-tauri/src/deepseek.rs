use rusqlite::Connection;
use tauri::Manager;

use crate::crypto;
use crate::db;
use crate::db::settings::{
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_API_KEY,
    KEY_DEEPSEEK_PROXY, KEY_PROXY_URL,
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

pub fn build_client(api_key: &str, proxy_url: &str) -> Result<reqwest::blocking::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    let mut builder = reqwest::blocking::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(10));
    if !proxy_url.is_empty() {
        if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build().map_err(|e| e.to_string())
}

fn call_summary(
    client: &reqwest::blocking::Client,
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
        "max_tokens": 300,
        "response_format": {"type": "json_object"}
    });
    let resp = client
        .post(format!(
            "{}/v1/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .json(&body_json)
        .send()
        .map_err(|e| format!("DeepSeek 请求失败: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        return Err(format!("DeepSeek API 返回错误 {}: {}", status, text));
    }
    let json: serde_json::Value = resp.json().map_err(|e| format!("解析响应失败: {}", e))?;
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let parsed: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析摘要 JSON 失败: {} — 原始内容: {}", e, content))?;
    let summary = parsed["summary"].as_str().unwrap_or("").to_string();
    let importance = parsed["importance"].as_str().unwrap_or("中").to_string();
    if summary.is_empty() {
        return Err("摘要为空".to_string());
    }
    Ok((summary, importance))
}

pub fn generate_summaries_for_new(
    app: &tauri::AppHandle,
    saved: &[(i64, Option<String>)],
) {
    let (enabled, model, base_url, api_key, proxy_url);

    {
        let state = app.state::<AppState>();
        let conn = state.db.get().unwrap();
        let cfg = read_config(&conn);
        enabled = cfg.0;
        model = cfg.1;
        base_url = cfg.2;
        api_key = cfg.3;
        let deepseek_proxy_enabled =
            db::settings::get_setting(&conn, KEY_DEEPSEEK_PROXY)
                .ok()
                .flatten()
                .map(|v| v == "true")
                .unwrap_or(false);
        proxy_url = if deepseek_proxy_enabled {
            db::settings::get_setting(&conn, KEY_PROXY_URL)
                .ok()
                .flatten()
                .unwrap_or_default()
        } else {
            String::new()
        };
    }

    if !enabled {
        return;
    }
    let api_key = match api_key {
        Some(k) => k,
        None => return,
    };
    let client = match build_client(&api_key, &proxy_url) {
        Ok(c) => c,
        Err(e) => {
            log::error!("创建 DeepSeek 客户端失败: {}", e);
            return;
        }
    };

    for (release_id, body) in saved {
        let body_text = match body {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        let truncated: String = body_text.chars().take(4000).collect();
        match call_summary(&client, &model, &base_url, &truncated) {
            Ok((summary, importance)) => {
                let state = app.state::<AppState>();
                let conn = state.db.get().unwrap();
                if let Err(e) = db::releases::set_ai_summary(
                    &conn, *release_id, &summary, &importance,
                ) {
                    log::error!("保存摘要失败 id={}: {}", release_id, e);
                } else {
                    let rel = db::releases::get_release(&conn, *release_id).ok().flatten();
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
            }
        }
    }
}
