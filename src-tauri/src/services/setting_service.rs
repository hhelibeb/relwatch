use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde_json::json;

use crate::adapters::bilibili::BILI_UA;
use crate::autostart;
use crate::crypto;
use crate::db::settings::{
    self, get_default_language, get_setting_bool, get_setting_i64, get_setting_str,
    normalize_auto_read_detail, strip_prompt_suffix, DEFAULT_AUTO_READ_DETAIL,
    DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MODEL,
    DEFAULT_DEEPSEEK_PROMPT_EDITABLE, DEFAULT_THEME, FONT_SCALE_MAX, FONT_SCALE_MIN,
    KEY_AUTO_READ_DETAIL, KEY_AUTO_START, KEY_BILIBILI_COOKIE, KEY_CHECK_PRERELEASES,
    KEY_DEEPSEEK_API_KEY, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_ENABLED,
    KEY_DEEPSEEK_MIN_IMPORTANCE, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_PROMPT,
    KEY_DEEPSEEK_PROXY_BYPASS, KEY_DEEPSEEK_TRANSLATE_RELEASE, KEY_ENABLE_USAGE_STATS,
    KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT, KEY_FONT_SCALE, KEY_GITHUB_TOKEN, KEY_LANGUAGE,
    KEY_LOG_RETENTION, KEY_MINIMIZE_TO_TRAY, KEY_NEXT_POLL_AT, KEY_POLL_INTERVAL,
    KEY_SHOW_IMPORTANCE, KEY_SHOW_SOURCE_TYPE_ICONS, KEY_THEME, KEY_YOUTUBE_API_KEY,
    SETTING_SPECS,
};
use crate::db::{self, DbPool};
use crate::deepseek;
use crate::error::{AppError, AppResult};
use crate::http;
use crate::types::AppSettings;

const BILI_NAV_URL: &str = "https://api.bilibili.com/x/web-interface/nav";

const CREDENTIAL_KINDS: &[(&str, &str, &str)] = &[
    ("deepseek_api_key", KEY_DEEPSEEK_API_KEY, "setting.deepseek_key_updated"),
    ("github_token", KEY_GITHUB_TOKEN, "setting.github_token_updated"),
    ("youtube_api_key", KEY_YOUTUBE_API_KEY, "setting.youtube_key_updated"),
    ("bilibili_cookie", KEY_BILIBILI_COOKIE, "setting.bilibili_cookie_updated"),
];

#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TestDeepseekPayload {
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub proxy_bypass: Option<bool>,
    pub proxy_url: Option<String>,
    pub proxy_mode: Option<String>,
}

fn json_setting_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn sanitize_settings(payload: AppSettings) -> AppSettings {
    AppSettings {
        poll_interval_minutes: payload.poll_interval_minutes.clamp(
            crate::poll::MIN_POLL_INTERVAL_MINUTES,
            crate::poll::MAX_POLL_INTERVAL_MINUTES,
        ),
        log_retention_days: payload.log_retention_days.clamp(0, 3650),
        fetch_history_count: payload.fetch_history_count.max(0),
        font_scale: payload.font_scale.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX),
        deepseek_prompt: strip_prompt_suffix(&payload.deepseek_prompt),
        auto_read_detail: normalize_auto_read_detail(&payload.auto_read_detail),
        ..payload
    }
}

pub async fn get_settings(pool: &DbPool) -> AppResult<AppSettings> {
    db::run_blocking(pool, |conn| {
        let proxy = settings::load_proxy_config(conn)?;
        Ok(AppSettings {
            poll_interval_minutes: get_setting_i64(conn, KEY_POLL_INTERVAL, 30)?,
            proxy_url: proxy.url,
            proxy_mode: proxy.mode,
            auto_start: get_setting_bool(conn, KEY_AUTO_START, false)?,
            minimize_to_tray: get_setting_bool(conn, KEY_MINIMIZE_TO_TRAY, true)?,
            log_retention_days: get_setting_i64(conn, KEY_LOG_RETENTION, 0)?,
            deepseek_enabled: get_setting_bool(conn, KEY_DEEPSEEK_ENABLED, false)?,
            deepseek_model: get_setting_str(conn, KEY_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_MODEL)?,
            deepseek_base_url: get_setting_str(
                conn,
                KEY_DEEPSEEK_BASE_URL,
                DEFAULT_DEEPSEEK_BASE_URL,
            )?,
            deepseek_api_key_set: get_setting_str(conn, KEY_DEEPSEEK_API_KEY, "")?
                .chars()
                .next()
                .is_some(),
            deepseek_proxy_bypass: get_setting_bool(conn, KEY_DEEPSEEK_PROXY_BYPASS, false)?,
            deepseek_prompt: strip_prompt_suffix(&get_setting_str(
                conn,
                KEY_DEEPSEEK_PROMPT,
                DEFAULT_DEEPSEEK_PROMPT_EDITABLE,
            )?),
            deepseek_min_importance: get_setting_str(
                conn,
                KEY_DEEPSEEK_MIN_IMPORTANCE,
                DEFAULT_DEEPSEEK_MIN_IMPORTANCE,
            )?,
            deepseek_translate_release: get_setting_bool(
                conn,
                KEY_DEEPSEEK_TRANSLATE_RELEASE,
                false,
            )?,
            check_prereleases: get_setting_bool(conn, KEY_CHECK_PRERELEASES, false)?,
            fetch_history: get_setting_bool(conn, KEY_FETCH_HISTORY, false)?,
            fetch_history_count: get_setting_i64(conn, KEY_FETCH_HISTORY_COUNT, 1)?.max(0),
            language: get_setting_str(conn, KEY_LANGUAGE, &get_default_language())?,
            theme: get_setting_str(conn, KEY_THEME, DEFAULT_THEME)?,
            font_scale: get_setting_i64(conn, KEY_FONT_SCALE, 100)?
                .clamp(FONT_SCALE_MIN, FONT_SCALE_MAX),
            show_source_type_icons: get_setting_bool(conn, KEY_SHOW_SOURCE_TYPE_ICONS, true)?,
            show_importance: get_setting_bool(conn, KEY_SHOW_IMPORTANCE, false)?,
            enable_usage_stats: get_setting_bool(conn, KEY_ENABLE_USAGE_STATS, true)?,
            auto_read_detail: normalize_auto_read_detail(&get_setting_str(
                conn,
                KEY_AUTO_READ_DETAIL,
                DEFAULT_AUTO_READ_DETAIL,
            )?),
            github_token_set: get_setting_str(conn, KEY_GITHUB_TOKEN, "")?
                .chars()
                .next()
                .is_some(),
            youtube_api_key_set: get_setting_str(conn, KEY_YOUTUBE_API_KEY, "")?
                .chars()
                .next()
                .is_some(),
            bilibili_cookie_set: get_setting_str(conn, KEY_BILIBILI_COOKIE, "")?
                .chars()
                .next()
                .is_some(),
        })
    })
    .await
}

pub async fn update_settings(
    pool: &DbPool,
    next_poll_at: &Arc<AtomicI64>,
    payload: AppSettings,
) -> AppResult<()> {
    let payload = sanitize_settings(payload);
    let next_poll_at = next_poll_at.clone();

    db::run_blocking_mut(pool, move |conn| {
        let payload_map = serde_json::to_value(&payload)
            .map_err(|e| AppError::validation_args("err.settings_serialize", [e.to_string()]))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                AppError::validation_args("err.settings_serialize", ["not-an-object"])
            })?;
        let new_values: Vec<String> = SETTING_SPECS
            .iter()
            .map(|s| {
                payload_map
                    .get(s.key)
                    .map(json_setting_value)
                    .unwrap_or_else(|| s.default.to_string())
            })
            .collect();
        let old_values: Vec<String> = SETTING_SPECS
            .iter()
            .map(|s| get_setting_str(conn, s.key, s.default))
            .collect::<Result<_, _>>()?;
        let items: Vec<(&str, &str, &str, &str)> = SETTING_SPECS
            .iter()
            .zip(old_values.iter())
            .zip(new_values.iter())
            .map(|((s, old), new)| (s.key, old.as_str(), new.as_str(), s.label))
            .collect();
        let auto_start_idx = SETTING_SPECS
            .iter()
            .position(|s| s.key == KEY_AUTO_START)
            .expect("KEY_AUTO_START 必须在 SETTING_SPECS 中");
        let auto_start_changed = old_values[auto_start_idx] != new_values[auto_start_idx];

        let (interval_changed, changes) = settings::apply_settings(conn, &items)?;
        if changes.is_empty() {
            return Ok(());
        }

        db::logs::write_log_key(
            conn,
            "INFO",
            "setting.updated",
            &json!({"changes": changes.join(", ")}).to_string(),
        );

        if interval_changed {
            let next = chrono::Utc::now().timestamp() + payload.poll_interval_minutes * 60;
            next_poll_at.store(next, Ordering::Relaxed);
            let _ = settings::set_setting(conn, KEY_NEXT_POLL_AT, &next.to_string());
            crate::poll::notify_poll_wake();
        }

        if auto_start_changed {
            if payload.auto_start {
                autostart::enable()
                    .map_err(|e| AppError::validation_args("err.autostart_enable_failed", [e]))?;
            } else {
                autostart::disable()
                    .map_err(|e| AppError::validation_args("err.autostart_disable_failed", [e]))?;
            }
        }

        Ok(())
    })
    .await
}

fn set_credential_conn(conn: &rusqlite::Connection, kind: &str, value: &str) -> AppResult<()> {
    let (key, label) = CREDENTIAL_KINDS
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, key, label)| (*key, *label))
        .ok_or_else(|| AppError::validation_args("err.unknown_credential_kind", [kind]))?;
    if value.is_empty() {
        settings::set_setting(conn, key, "")?;
    } else {
        let encrypted = crypto::encrypt(value);
        settings::set_setting(conn, key, &encrypted)?;
    }
    db::logs::write_log_key(conn, "INFO", label, &json!({}).to_string());
    Ok(())
}

pub async fn set_credential(pool: &DbPool, kind: String, value: String) -> AppResult<()> {
    db::run_blocking(pool, move |conn| set_credential_conn(conn, &kind, &value)).await
}

pub async fn test_deepseek_connection(
    pool: &DbPool,
    payload: Option<TestDeepseekPayload>,
) -> AppResult<()> {
    let (model, base_url, api_key, proxy_url, proxy_mode) = db::run_blocking(pool, move |conn| {
        let config = deepseek::read_config(conn);
        let p = payload.as_ref();
        let non_empty = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());

        let model = p.and_then(|p| non_empty(&p.model)).unwrap_or(config.model);
        let base_url = p
            .and_then(|p| non_empty(&p.base_url))
            .unwrap_or(config.base_url);
        let api_key = p.and_then(|p| non_empty(&p.api_key)).or(config.api_key);
        let bypass = match p.and_then(|p| p.proxy_bypass) {
            Some(b) => b,
            None => get_setting_bool(conn, KEY_DEEPSEEK_PROXY_BYPASS, false)?,
        };
        let (proxy_url, proxy_mode) = if bypass {
            (String::new(), "none".to_string())
        } else {
            let saved = settings::load_proxy_config(conn)?;
            (
                p.and_then(|p| non_empty(&p.proxy_url)).unwrap_or(saved.url),
                p.and_then(|p| non_empty(&p.proxy_mode)).unwrap_or(saved.mode),
            )
        };
        Ok((model, base_url, api_key, proxy_url, proxy_mode))
    })
    .await?;

    let api_key = api_key.ok_or_else(|| AppError::validation("err.deepseek_api_key_missing"))?;
    let client = deepseek::build_client(
        &api_key,
        &proxy_url,
        &proxy_mode,
        deepseek::DEEPSEEK_TIMEOUT_SECS_TEST,
    )
    .map_err(AppError::raw)?;
    let body = json!({
        "model": model,
        "messages": [
            {"role": "user", "content": "Hi"}
        ],
        "max_tokens": 10,
        "temperature": 0.0
    });
    let outcome = deepseek::chat_completion(&client, &base_url, &body)
        .await
        .map_err(|(status, msg)| {
            if status > 0 {
                AppError::validation_args("err.api_error", [status.to_string(), msg])
            } else {
                AppError::validation_args("err.request_failed", [msg])
            }
        })?;

    let usage = db::ai_usage::CallUsage::from_outcome(
        "test",
        outcome.usage,
        &outcome.content,
        deepseek::count_prompt_chars(&body),
        outcome.duration_ms,
    );
    let model_for_usage = model;
    let _ = db::run_blocking(pool, move |conn| {
        if let Err(e) = db::ai_usage::insert_call_usage(conn, None, &model_for_usage, &[usage]) {
            log::warn!("记录连接测试用量失败: {}", e);
        }
        Ok(())
    })
    .await;

    Ok(())
}

pub async fn verify_and_save_bilibili_cookie(pool: &DbPool, sessdata: &str) -> AppResult<bool> {
    verify_and_save_bilibili_cookie_with_url(pool, sessdata, BILI_NAV_URL).await
}

async fn verify_and_save_bilibili_cookie_with_url(
    pool: &DbPool,
    sessdata: &str,
    nav_url: &str,
) -> AppResult<bool> {
    let proxy = db::run_blocking(pool, |conn| {
        Ok(settings::load_proxy_config(conn).unwrap_or_default())
    })
    .await?;

    let client = http::build_http_client(http::HttpClientConfig {
        proxy_url: &proxy.url,
        proxy_mode: &proxy.mode,
        bearer_token: None,
        ..Default::default()
    })
    .map_err(AppError::raw)?;

    let nav = client
        .get(nav_url)
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Cookie", format!("SESSDATA={}", sessdata))
        .send()
        .await
        .map_err(|e| AppError::raw(http::describe_request_error(&e)))?;
    let body: serde_json::Value = http::read_json_limited(nav, http::MAX_JSON_BYTES)
        .await
        .map_err(AppError::raw)?;
    if body["data"]["isLogin"].as_bool() != Some(true) {
        return Err(AppError::validation("err.bili_login_not_logged_in"));
    }

    let encrypted = crypto::encrypt(sessdata);
    db::run_blocking(pool, move |conn| {
        settings::set_setting(conn, KEY_BILIBILI_COOKIE, &encrypted)?;
        db::logs::write_log_key(
            conn,
            "INFO",
            "setting.bilibili_cookie_updated",
            &json!({"source": "webview-login"}).to_string(),
        );
        Ok(())
    })
    .await?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_pool;
    use crate::db::settings::{KEY_PROXY_MODE, KEY_PROXY_URL};

    fn sample_settings_json() -> serde_json::Value {
        json!({
            "poll_interval_minutes": 30,
            "proxy_mode": "none",
            "proxy_url": "",
            "auto_start": false,
            "minimize_to_tray": true,
            "log_retention_days": 0,
            "deepseek_enabled": false,
            "deepseek_model": "m",
            "deepseek_base_url": "u",
            "deepseek_api_key_set": false,
            "deepseek_proxy_bypass": false,
            "deepseek_prompt": "请用中文总结",
            "deepseek_min_importance": "小",
            "deepseek_translate_release": false,
            "check_prereleases": false,
            "fetch_history": false,
            "fetch_history_count": 1,
            "language": "zh-CN",
            "theme": "system",
            "font_scale": 100,
            "show_source_type_icons": true,
            "show_importance": false,
            "enable_usage_stats": true,
            "auto_read_detail": "navigate",
            "github_token_set": false,
            "youtube_api_key_set": false,
            "bilibili_cookie_set": false,
        })
    }

    #[test]
    fn setting_specs_cover_all_app_settings_fields() {
        let payload: AppSettings = serde_json::from_value(sample_settings_json()).unwrap();
        let map = serde_json::to_value(&payload).unwrap();
        let obj = map.as_object().unwrap();

        for spec in SETTING_SPECS {
            assert!(obj.contains_key(spec.key), "missing key '{}'", spec.key);
        }
        assert_eq!(SETTING_SPECS.len(), 23);
    }

    #[test]
    fn json_setting_value_serialization() {
        assert_eq!(json_setting_value(&json!(true)), "true");
        assert_eq!(json_setting_value(&json!(false)), "false");
        assert_eq!(json_setting_value(&json!(30)), "30");
        assert_eq!(json_setting_value(&json!(0)), "0");
        assert_eq!(json_setting_value(&json!("zh-CN")), "zh-CN");
        assert_eq!(json_setting_value(&json!("")), "");
        assert_eq!(json_setting_value(&json!(null)), "");
        assert_eq!(json_setting_value(&json!([1])), "");
    }

    #[test]
    fn sanitize_settings_normalizes_on_write() {
        let mut val = sample_settings_json();
        val["auto_read_detail"] = json!("always");
        val["font_scale"] = json!(9999);
        val["log_retention_days"] = json!(-5);
        val["fetch_history_count"] = json!(-1);

        let payload: AppSettings = serde_json::from_value(val).unwrap();
        let sanitized = sanitize_settings(payload);

        assert_eq!(sanitized.auto_read_detail, "off");
        assert_eq!(sanitized.font_scale, FONT_SCALE_MAX);
        assert_eq!(sanitized.log_retention_days, 0);
        assert_eq!(sanitized.fetch_history_count, 0);
        assert_eq!(sanitized.deepseek_prompt, "请用中文总结");
    }

    #[tokio::test]
    async fn update_and_get_settings_roundtrip_updates_next_poll_and_writes_log() {
        let pool = init_memory_pool().unwrap();
        let next_poll = Arc::new(AtomicI64::new(0));

        let mut payload = get_settings(&pool).await.unwrap();
        payload.poll_interval_minutes = 45;
        payload.log_retention_days = 14;
        payload.theme = "dark".into();
        payload.deepseek_proxy_bypass = true;
        payload.auto_read_detail = "always".into();

        update_settings(&pool, &next_poll, payload).await.unwrap();

        assert!(next_poll.load(Ordering::Relaxed) > 0);
        let loaded = get_settings(&pool).await.unwrap();
        assert_eq!(loaded.poll_interval_minutes, 45);
        assert_eq!(loaded.log_retention_days, 14);
        assert_eq!(loaded.theme, "dark");
        assert!(loaded.deepseek_proxy_bypass);
        assert_eq!(loaded.auto_read_detail, "off");

        let conn = pool.get().unwrap();
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert!(logs
            .iter()
            .any(|l| l.message_key.as_deref() == Some("setting.updated")));
        assert_eq!(
            get_setting_str(&conn, KEY_PROXY_MODE, "none").unwrap(),
            "none"
        );
        assert_eq!(get_setting_str(&conn, KEY_PROXY_URL, "").unwrap(), "");
    }

    #[tokio::test]
    async fn set_credential_encrypts_clears_and_rejects_unknown_kind() {
        crypto::set_test_master_key();
        let pool = init_memory_pool().unwrap();

        let err = set_credential(&pool, "evil_kind".into(), "x".into())
            .await
            .unwrap_err()
            .to_ipc_string();
        assert_eq!(err, "err.unknown_credential_kind|evil_kind");

        set_credential(&pool, "github_token".into(), "ghp_secret".into())
            .await
            .unwrap();
        {
            let conn = pool.get().unwrap();
            let stored = settings::get_setting(&conn, KEY_GITHUB_TOKEN)
                .unwrap()
                .unwrap();
            assert!(!stored.contains("ghp_secret"));
            assert_eq!(crypto::decrypt(&stored).unwrap(), "ghp_secret");
        }
        assert!(get_settings(&pool).await.unwrap().github_token_set);

        set_credential(&pool, "github_token".into(), String::new())
            .await
            .unwrap();
        assert!(!get_settings(&pool).await.unwrap().github_token_set);
    }

    #[tokio::test]
    async fn test_deepseek_connection_uses_override_and_records_usage() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-override"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "Hello!" } }],
                "usage": { "prompt_tokens": 5, "completion_tokens": 2 }
            })))
            .mount(&mock)
            .await;

        let pool = init_memory_pool().unwrap();
        test_deepseek_connection(
            &pool,
            Some(TestDeepseekPayload {
                model: Some("deepseek-v4-flash".into()),
                base_url: Some(mock.uri()),
                api_key: Some("sk-override".into()),
                proxy_bypass: Some(true),
                proxy_url: None,
                proxy_mode: None,
            }),
        )
        .await
        .unwrap();

        let conn = pool.get().unwrap();
        let stats = db::ai_usage::get_ai_usage_stats(&conn, None, None).unwrap();
        assert_eq!(stats.by_action.len(), 1);
        assert_eq!(stats.by_action[0].action, "test");
        assert_eq!(stats.by_action[0].calls, 1);
    }

    #[tokio::test]
    async fn test_deepseek_connection_surfaces_api_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&mock)
            .await;

        let pool = init_memory_pool().unwrap();
        let err = test_deepseek_connection(
            &pool,
            Some(TestDeepseekPayload {
                model: Some("deepseek-v4-flash".into()),
                base_url: Some(mock.uri()),
                api_key: Some("bad-key".into()),
                proxy_bypass: Some(true),
                proxy_url: None,
                proxy_mode: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_ipc_string().starts_with("err.api_error|401|"));
    }

    #[tokio::test]
    async fn verify_and_save_bilibili_cookie_encrypts_when_logged_in() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        crypto::set_test_master_key();
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nav-ok"))
            .and(header("cookie", "SESSDATA=valid-cookie"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": 0,
                "data": { "isLogin": true }
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/nav-no"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": 0,
                "data": { "isLogin": false }
            })))
            .mount(&mock)
            .await;

        let pool = init_memory_pool().unwrap();
        let err = verify_and_save_bilibili_cookie_with_url(
            &pool,
            "expired",
            &format!("{}/nav-no", mock.uri()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.bili_login_not_logged_in");

        let ok = verify_and_save_bilibili_cookie_with_url(
            &pool,
            "valid-cookie",
            &format!("{}/nav-ok", mock.uri()),
        )
        .await
        .unwrap();
        assert!(ok);

        let conn = pool.get().unwrap();
        let stored = settings::get_setting(&conn, KEY_BILIBILI_COOKIE)
            .unwrap()
            .unwrap();
        assert!(!stored.contains("valid-cookie"));
        assert_eq!(crypto::decrypt(&stored).unwrap(), "valid-cookie");
    }
}
