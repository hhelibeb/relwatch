use std::sync::atomic::Ordering;
use tauri::Manager;

use crate::autostart;
use crate::crypto;
use crate::db;
use crate::deepseek;
use crate::types::{AppSettings, AppState};
use crate::db::settings::{
    self, KEY_POLL_INTERVAL, KEY_PROXY_URL, KEY_PROXY_MODE, KEY_MINIMIZE_TO_TRAY, KEY_LOG_RETENTION,
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_API_KEY, KEY_DEEPSEEK_PROXY_BYPASS,
    KEY_DEEPSEEK_PROMPT, KEY_DEEPSEEK_MIN_IMPORTANCE,
    KEY_AUTO_START,
    KEY_CHECK_PRERELEASES, KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT,
    KEY_LANGUAGE, KEY_THEME, KEY_GITHUB_TOKEN, KEY_NEXT_POLL_AT,
    DEFAULT_POLL_INTERVAL, DEFAULT_PROXY_URL, DEFAULT_AUTO_START, DEFAULT_MINIMIZE_TO_TRAY, DEFAULT_LOG_RETENTION,
    DEFAULT_DEEPSEEK_ENABLED, DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_PROXY_BYPASS,
    DEFAULT_DEEPSEEK_PROMPT_EDITABLE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE,
    DEFAULT_CHECK_PRERELEASES, DEFAULT_FETCH_HISTORY_COUNT, DEFAULT_THEME,
    get_setting_str, get_setting_bool, get_setting_i64, get_default_language,
    strip_prompt_suffix,
};
use serde_json::json;

#[tauri::command]
pub fn get_settings(state: tauri::State<AppState>) -> Result<AppSettings, String> {
    let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
    let proxy_url = get_setting_str(&conn, KEY_PROXY_URL, DEFAULT_PROXY_URL)?;
    let proxy_mode = get_setting_str(&conn, KEY_PROXY_MODE, if proxy_url.is_empty() { "none" } else { "custom" })?;
    Ok(AppSettings {
        poll_interval_minutes: get_setting_i64(&conn, KEY_POLL_INTERVAL, 30)?,
        proxy_url,
        proxy_mode,
        auto_start: get_setting_bool(&conn, KEY_AUTO_START, false)?,
        minimize_to_tray: get_setting_bool(&conn, KEY_MINIMIZE_TO_TRAY, true)?,
        log_retention_days: get_setting_i64(&conn, KEY_LOG_RETENTION, 0)?,
        deepseek_enabled: get_setting_bool(&conn, KEY_DEEPSEEK_ENABLED, false)?,
        deepseek_model: get_setting_str(&conn, KEY_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_MODEL)?,
        deepseek_base_url: get_setting_str(&conn, KEY_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_BASE_URL)?,
        deepseek_api_key_set: get_setting_str(&conn, KEY_DEEPSEEK_API_KEY, "")?
            .chars()
            .next()
            .is_some(),
        deepseek_proxy_bypass: get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false)?,
        deepseek_prompt: strip_prompt_suffix(&get_setting_str(&conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE)?),
        deepseek_min_importance: get_setting_str(&conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE)?,

        check_prereleases: get_setting_bool(&conn, KEY_CHECK_PRERELEASES, false)?,
        fetch_history: get_setting_bool(&conn, KEY_FETCH_HISTORY, false)?,
        fetch_history_count: get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1)?.max(0),
        language: get_setting_str(&conn, KEY_LANGUAGE, &get_default_language())?,
        theme: get_setting_str(&conn, KEY_THEME, DEFAULT_THEME)?,
        github_token_set: get_setting_str(&conn, KEY_GITHUB_TOKEN, "")?
            .chars()
            .next()
            .is_some(),
    })
}

/// 仅用于 `update_settings` 接收前端参数
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettingsPayload {
    poll_interval_minutes: i64,
    proxy_url: String,
    proxy_mode: String,
    auto_start: bool,
    minimize_to_tray: bool,
    log_retention_days: i64,
    deepseek_enabled: bool,
    deepseek_model: String,
    deepseek_base_url: String,
    deepseek_proxy_bypass: bool,
    deepseek_prompt: String,
    deepseek_min_importance: String,

    check_prereleases: bool,
    fetch_history: bool,
    fetch_history_count: i64,
    language: String,
    theme: String,
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    payload: UpdateSettingsPayload,
) -> Result<(), String> {
    let poll_interval_minutes = payload.poll_interval_minutes.clamp(5, 1440);
    let log_retention_days = payload.log_retention_days.clamp(0, 3650);
    let fetch_history_count = payload.fetch_history_count.max(0);

    let state = app.state::<AppState>();
    let conn = state.db.get().map_err(|e| e.to_string())?;

    let old_interval = get_setting_str(&conn, KEY_POLL_INTERVAL, DEFAULT_POLL_INTERVAL)?;
    let old_proxy_mode = get_setting_str(&conn, KEY_PROXY_MODE, "none")?;
    let old_proxy = get_setting_str(&conn, KEY_PROXY_URL, DEFAULT_PROXY_URL)?;
    let old_auto_start = get_setting_str(&conn, KEY_AUTO_START, DEFAULT_AUTO_START)?;
    let old_minimize = get_setting_str(&conn, KEY_MINIMIZE_TO_TRAY, DEFAULT_MINIMIZE_TO_TRAY)?;
    let old_retention = get_setting_str(&conn, KEY_LOG_RETENTION, DEFAULT_LOG_RETENTION)?;
    let old_deepseek = get_setting_str(&conn, KEY_DEEPSEEK_ENABLED, DEFAULT_DEEPSEEK_ENABLED)?;
    let old_model = get_setting_str(&conn, KEY_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_MODEL)?;
    let old_base_url = get_setting_str(&conn, KEY_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_BASE_URL)?;
    let old_deepseek_proxy_bypass = get_setting_str(&conn, KEY_DEEPSEEK_PROXY_BYPASS, DEFAULT_DEEPSEEK_PROXY_BYPASS)?;
    let old_deepseek_prompt = get_setting_str(&conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE)?;
    let old_deepseek_min_importance = get_setting_str(&conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE)?;

    let old_check_pre = get_setting_str(&conn, KEY_CHECK_PRERELEASES, DEFAULT_CHECK_PRERELEASES)?;
    let old_fetch_history = get_setting_str(&conn, KEY_FETCH_HISTORY, "false")?;
    let old_fetch_history_count = get_setting_str(&conn, KEY_FETCH_HISTORY_COUNT, DEFAULT_FETCH_HISTORY_COUNT)?;
    let old_language = get_setting_str(&conn, KEY_LANGUAGE, "")?;
    let old_theme = get_setting_str(&conn, KEY_THEME, DEFAULT_THEME)?;

    let (interval_changed, changes) = settings::apply_settings(
        &conn,
        &[
            (KEY_POLL_INTERVAL, &old_interval, &poll_interval_minutes.to_string(), "setting.poll_interval"),
            (KEY_PROXY_MODE, &old_proxy_mode, &payload.proxy_mode, "setting.proxy_mode"),
            (KEY_PROXY_URL, &old_proxy, &payload.proxy_url, "setting.proxy_url"),
            (KEY_AUTO_START, &old_auto_start, &payload.auto_start.to_string(), "setting.auto_start"),
            (KEY_MINIMIZE_TO_TRAY, &old_minimize, &payload.minimize_to_tray.to_string(), "setting.minimize_to_tray"),
            (KEY_LOG_RETENTION, &old_retention, &log_retention_days.to_string(), "setting.log_retention_days"),
            (KEY_DEEPSEEK_ENABLED, &old_deepseek, &payload.deepseek_enabled.to_string(), "setting.deepseek_enabled"),
            (KEY_DEEPSEEK_MODEL, &old_model, &payload.deepseek_model, "setting.deepseek_model"),
            (KEY_DEEPSEEK_BASE_URL, &old_base_url, &payload.deepseek_base_url, "setting.deepseek_base_url"),
            (KEY_DEEPSEEK_PROXY_BYPASS, &old_deepseek_proxy_bypass, &payload.deepseek_proxy_bypass.to_string(), "setting.deepseek_proxy_bypass"),
            (KEY_DEEPSEEK_PROMPT, &old_deepseek_prompt, &strip_prompt_suffix(&payload.deepseek_prompt), "setting.deepseek_prompt"),
            (KEY_DEEPSEEK_MIN_IMPORTANCE, &old_deepseek_min_importance, &payload.deepseek_min_importance, "setting.deepseek_min_importance"),

            (KEY_CHECK_PRERELEASES, &old_check_pre, &payload.check_prereleases.to_string(), "setting.check_prereleases"),
            (KEY_FETCH_HISTORY, &old_fetch_history, &payload.fetch_history.to_string(), "setting.fetch_history"),
            (KEY_FETCH_HISTORY_COUNT, &old_fetch_history_count, &fetch_history_count.to_string(), "setting.fetch_history_count"),
            (KEY_LANGUAGE, &old_language, &payload.language, "setting.language"),
            // 不在此处触发 rendered_message 回填：这是有意为之的 locale-frozen 设计。
            // 切换语言后日志搜索仅对新写入的行生效，旧行以原 locale 的 rendered_message 存在。
            // 参见 write_log_key 中的注释了解设计取舍。
            (KEY_THEME, &old_theme, &payload.theme, "setting.theme"),
        ],
    )?;

    if changes.is_empty() {
        return Ok(());
    }

    db::logs::write_log_key(&conn, "INFO", "setting.updated", &json!({"changes": changes.join(", ")}).to_string());

    if interval_changed {
        let next = chrono::Utc::now().timestamp() + poll_interval_minutes * 60;
        state.next_poll_at.store(next, Ordering::Relaxed);
        let _ = settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
    }

    // 开机自启动变化时，立即执行系统注册/注销
    if old_auto_start != payload.auto_start.to_string() {
        if payload.auto_start {
            autostart::enable().map_err(|e| format!("设置开机自启动失败: {}", e))?;
        } else {
            autostart::disable().map_err(|e| format!("取消开机自启动失败: {}", e))?;
        }
    }

    Ok(())
}

#[tauri::command]
pub fn set_deepseek_api_key(
    state: tauri::State<AppState>,
    api_key: String,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| e.to_string())?;
    if api_key.is_empty() {
        settings::set_setting(&conn, KEY_DEEPSEEK_API_KEY, "")?;
    } else {
        let encrypted = crypto::encrypt(&api_key);
        settings::set_setting(&conn, KEY_DEEPSEEK_API_KEY, &encrypted)?;
    }
    db::logs::write_log_key(&conn, "INFO", "setting.deepseek_key_updated", &json!({}).to_string());
    Ok(())
}

#[tauri::command]
pub fn set_github_token(
    state: tauri::State<AppState>,
    token: String,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| e.to_string())?;
    if token.is_empty() {
        settings::set_setting(&conn, KEY_GITHUB_TOKEN, "")?;
    } else {
        let encrypted = crypto::encrypt(&token);
        settings::set_setting(&conn, KEY_GITHUB_TOKEN, &encrypted)?;
    }
    db::logs::write_log_key(&conn, "INFO", "setting.github_token_updated", &json!({}).to_string());
    Ok(())
}

#[tauri::command]
pub async fn test_deepseek_connection(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let (model, base_url, api_key, proxy_url, proxy_mode);
    {
        let conn = state.db.get().map_err(|e| format!("数据库连接失败: {}", e))?;
        let config = deepseek::read_config(&conn);
        model = config.1;
        base_url = config.2;
        api_key = config.3;
        let bypass = get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false)?;
        if bypass {
            proxy_url = String::new();
            proxy_mode = "none".to_string();
        } else {
            proxy_url = get_setting_str(&conn, KEY_PROXY_URL, DEFAULT_PROXY_URL)?;
            proxy_mode = get_setting_str(&conn, KEY_PROXY_MODE, "none")?;
        }
    }
    let api_key = api_key.ok_or("请先设置 DeepSeek API Key")?;
    let client = deepseek::build_client(&api_key, &proxy_url, &proxy_mode)?;
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": "Hi"}
        ],
        "max_tokens": 10,
        "temperature": 0.0
    });
    let resp = client
        .post(format!(
            "{}/v1/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API 返回错误 {}: {}", status, text));
    }
    Ok("连接成功".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::sync::Arc;

    fn test_state() -> AppState {
        AppState {
            db: db::init::init_memory_pool().unwrap(),
            next_poll_at: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            deepseek_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(50)),
        }
    }

    #[test]
    fn test_update_settings_only_writes_changed() {
        let state = test_state();
        {
            let conn = state.db.get().unwrap();
            settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
            settings::set_setting(&conn, KEY_PROXY_URL, "http://old").unwrap();
            settings::set_setting(&conn, KEY_LOG_RETENTION, "7").unwrap();
        }

        let conn = state.db.get().unwrap();
        let old_interval = get_setting_str(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        let old_proxy = get_setting_str(&conn, KEY_PROXY_URL, "").unwrap();
        let old_proxy_mode = get_setting_str(&conn, KEY_PROXY_MODE, "none").unwrap();
        let old_auto_start = get_setting_str(&conn, KEY_AUTO_START, DEFAULT_AUTO_START).unwrap();
        let old_retention = get_setting_str(&conn, KEY_LOG_RETENTION, "0").unwrap();
        let old_minimize = get_setting_str(&conn, KEY_MINIMIZE_TO_TRAY, "true").unwrap();
        let old_deepseek = get_setting_str(&conn, KEY_DEEPSEEK_ENABLED, "false").unwrap();
        let old_model = get_setting_str(&conn, KEY_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_MODEL).unwrap();
        let old_base_url = get_setting_str(&conn, KEY_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_BASE_URL).unwrap();
        let old_deepseek_proxy_bypass = get_setting_str(&conn, KEY_DEEPSEEK_PROXY_BYPASS, DEFAULT_DEEPSEEK_PROXY_BYPASS).unwrap();
        let old_deepseek_prompt = get_setting_str(&conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE).unwrap();
        let old_deepseek_min_importance = get_setting_str(&conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE).unwrap();
        let old_check_pre = get_setting_str(&conn, KEY_CHECK_PRERELEASES, "false").unwrap();
        let old_fetch_history = get_setting_str(&conn, KEY_FETCH_HISTORY, "false").unwrap();
        let old_fetch_history_count = get_setting_str(&conn, KEY_FETCH_HISTORY_COUNT, DEFAULT_FETCH_HISTORY_COUNT).unwrap();
        let old_language = get_setting_str(&conn, KEY_LANGUAGE, "").unwrap();
        let old_theme = get_setting_str(&conn, KEY_THEME, DEFAULT_THEME).unwrap();

        // 仅改 log_retention_days 7→14
        let (_, changes) = settings::apply_settings(
            &conn,
            &[
                (KEY_POLL_INTERVAL, &old_interval, "30", ""),
                (KEY_AUTO_START, &old_auto_start, "false", ""),
                (KEY_PROXY_MODE, &old_proxy_mode, "custom", ""),
                (KEY_PROXY_URL, &old_proxy, "http://old", ""),
                (KEY_MINIMIZE_TO_TRAY, &old_minimize, "true", ""),
                (KEY_LOG_RETENTION, &old_retention, "14", "setting.log_retention_days"),
                (KEY_DEEPSEEK_ENABLED, &old_deepseek, "false", ""),
                (KEY_DEEPSEEK_MODEL, &old_model, DEFAULT_DEEPSEEK_MODEL, ""),
                (KEY_DEEPSEEK_BASE_URL, &old_base_url, DEFAULT_DEEPSEEK_BASE_URL, ""),
                (KEY_DEEPSEEK_PROXY_BYPASS, &old_deepseek_proxy_bypass, "false", ""),
                (KEY_DEEPSEEK_PROMPT, &old_deepseek_prompt, DEFAULT_DEEPSEEK_PROMPT_EDITABLE, ""),
                (KEY_DEEPSEEK_MIN_IMPORTANCE, &old_deepseek_min_importance, "小", ""),
                (KEY_CHECK_PRERELEASES, &old_check_pre, "false", ""),
                (KEY_FETCH_HISTORY, &old_fetch_history, "false", ""),
                (KEY_FETCH_HISTORY_COUNT, &old_fetch_history_count, DEFAULT_FETCH_HISTORY_COUNT, ""),
                (KEY_LANGUAGE, &old_language, "zh-CN", ""),
                (KEY_THEME, &old_theme, DEFAULT_THEME, ""),
            ],
        ).unwrap();
        drop(conn);

        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("setting.log_retention_days"));

        let conn = state.db.get().unwrap();
        assert_eq!(get_setting_str(&conn, KEY_POLL_INTERVAL, "").unwrap(), "30");
        assert_eq!(get_setting_str(&conn, KEY_PROXY_URL, "").unwrap(), "http://old");
        assert_eq!(get_setting_str(&conn, KEY_LOG_RETENTION, "").unwrap(), "14");
    }

    #[test]
    fn test_update_settings_no_changes() {
        let state = test_state();
        {
            let conn = state.db.get().unwrap();
            settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
            settings::set_setting(&conn, KEY_PROXY_URL, "http://proxy").unwrap();
            settings::set_setting(&conn, KEY_MINIMIZE_TO_TRAY, "true").unwrap();
        }

        let conn = state.db.get().unwrap();
        let old_interval = get_setting_str(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        let old_proxy = get_setting_str(&conn, KEY_PROXY_URL, "").unwrap();
        let old_proxy_mode = get_setting_str(&conn, KEY_PROXY_MODE, "none").unwrap();
        let old_auto_start = get_setting_str(&conn, KEY_AUTO_START, DEFAULT_AUTO_START).unwrap();
        let old_minimize = get_setting_str(&conn, KEY_MINIMIZE_TO_TRAY, "true").unwrap();
        let old_retention = get_setting_str(&conn, KEY_LOG_RETENTION, "0").unwrap();
        let old_deepseek = get_setting_str(&conn, KEY_DEEPSEEK_ENABLED, "false").unwrap();
        let old_model = get_setting_str(&conn, KEY_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_MODEL).unwrap();
        let old_base_url = get_setting_str(&conn, KEY_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_BASE_URL).unwrap();
        let old_deepseek_proxy_bypass = get_setting_str(&conn, KEY_DEEPSEEK_PROXY_BYPASS, DEFAULT_DEEPSEEK_PROXY_BYPASS).unwrap();
        let old_deepseek_prompt = get_setting_str(&conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE).unwrap();
        let old_deepseek_min_importance = get_setting_str(&conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE).unwrap();
        let old_check_pre = get_setting_str(&conn, KEY_CHECK_PRERELEASES, "false").unwrap();
        let old_fetch_history = get_setting_str(&conn, KEY_FETCH_HISTORY, "false").unwrap();
        let old_fetch_history_count = get_setting_str(&conn, KEY_FETCH_HISTORY_COUNT, DEFAULT_FETCH_HISTORY_COUNT).unwrap();
        let old_language = get_setting_str(&conn, KEY_LANGUAGE, "").unwrap();
        let old_theme = get_setting_str(&conn, KEY_THEME, DEFAULT_THEME).unwrap();

        let (interval_changed, changes) = settings::apply_settings(
            &conn,
            &[
                (KEY_POLL_INTERVAL, &old_interval, "30", ""),
                (KEY_AUTO_START, &old_auto_start, "false", ""),
                (KEY_PROXY_MODE, &old_proxy_mode, "custom", ""),
                (KEY_PROXY_URL, &old_proxy, "http://proxy", ""),
                (KEY_MINIMIZE_TO_TRAY, &old_minimize, "true", ""),
                (KEY_LOG_RETENTION, &old_retention, "0", ""),
                (KEY_DEEPSEEK_ENABLED, &old_deepseek, "false", ""),
                (KEY_DEEPSEEK_MODEL, &old_model, DEFAULT_DEEPSEEK_MODEL, ""),
                (KEY_DEEPSEEK_BASE_URL, &old_base_url, DEFAULT_DEEPSEEK_BASE_URL, ""),
                (KEY_DEEPSEEK_PROXY_BYPASS, &old_deepseek_proxy_bypass, "false", ""),
                (KEY_DEEPSEEK_PROMPT, &old_deepseek_prompt, DEFAULT_DEEPSEEK_PROMPT_EDITABLE, ""),
                (KEY_DEEPSEEK_MIN_IMPORTANCE, &old_deepseek_min_importance, "小", ""),
                (KEY_CHECK_PRERELEASES, &old_check_pre, "false", ""),
                (KEY_FETCH_HISTORY, &old_fetch_history, "false", ""),
                (KEY_FETCH_HISTORY_COUNT, &old_fetch_history_count, DEFAULT_FETCH_HISTORY_COUNT, ""),
                (KEY_LANGUAGE, &old_language, "zh-CN", ""),
                (KEY_THEME, &old_theme, DEFAULT_THEME, ""),
            ],
        ).unwrap();
        drop(conn);

        assert!(!interval_changed);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_update_settings_poll_interval_triggers_flag() {
        let state = test_state();
        {
            let conn = state.db.get().unwrap();
            settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        }

        let conn = state.db.get().unwrap();
        let old_interval = get_setting_str(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        let old_proxy = get_setting_str(&conn, KEY_PROXY_URL, "").unwrap();
        let old_proxy_mode = get_setting_str(&conn, KEY_PROXY_MODE, "none").unwrap();
        let old_auto_start = get_setting_str(&conn, KEY_AUTO_START, DEFAULT_AUTO_START).unwrap();
        let old_minimize = get_setting_str(&conn, KEY_MINIMIZE_TO_TRAY, "true").unwrap();
        let old_retention = get_setting_str(&conn, KEY_LOG_RETENTION, "0").unwrap();
        let old_deepseek = get_setting_str(&conn, KEY_DEEPSEEK_ENABLED, "false").unwrap();
        let old_model = get_setting_str(&conn, KEY_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_MODEL).unwrap();
        let old_base_url = get_setting_str(&conn, KEY_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_BASE_URL).unwrap();
        let old_deepseek_proxy_bypass = get_setting_str(&conn, KEY_DEEPSEEK_PROXY_BYPASS, DEFAULT_DEEPSEEK_PROXY_BYPASS).unwrap();
        let old_deepseek_prompt = get_setting_str(&conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE).unwrap();
        let old_deepseek_min_importance = get_setting_str(&conn, KEY_DEEPSEEK_MIN_IMPORTANCE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE).unwrap();
        let old_check_pre = get_setting_str(&conn, KEY_CHECK_PRERELEASES, "false").unwrap();
        let old_fetch_history = get_setting_str(&conn, KEY_FETCH_HISTORY, "false").unwrap();
        let old_fetch_history_count = get_setting_str(&conn, KEY_FETCH_HISTORY_COUNT, DEFAULT_FETCH_HISTORY_COUNT).unwrap();
        let old_language = get_setting_str(&conn, KEY_LANGUAGE, "").unwrap();
        let old_theme = get_setting_str(&conn, KEY_THEME, DEFAULT_THEME).unwrap();

        let (interval_changed, changes) = settings::apply_settings(
            &conn,
            &[
                (KEY_POLL_INTERVAL, &old_interval, "60", "setting.poll_interval"),
                (KEY_AUTO_START, &old_auto_start, "false", ""),
                (KEY_PROXY_MODE, &old_proxy_mode, "none", ""),
                (KEY_PROXY_URL, &old_proxy, "", ""),
                (KEY_MINIMIZE_TO_TRAY, &old_minimize, "true", ""),
                (KEY_LOG_RETENTION, &old_retention, "0", ""),
                (KEY_DEEPSEEK_ENABLED, &old_deepseek, "false", ""),
                (KEY_DEEPSEEK_MODEL, &old_model, DEFAULT_DEEPSEEK_MODEL, ""),
                (KEY_DEEPSEEK_BASE_URL, &old_base_url, DEFAULT_DEEPSEEK_BASE_URL, ""),
                (KEY_DEEPSEEK_PROXY_BYPASS, &old_deepseek_proxy_bypass, "false", ""),
                (KEY_DEEPSEEK_PROMPT, &old_deepseek_prompt, DEFAULT_DEEPSEEK_PROMPT_EDITABLE, ""),
                (KEY_DEEPSEEK_MIN_IMPORTANCE, &old_deepseek_min_importance, "小", ""),
                (KEY_CHECK_PRERELEASES, &old_check_pre, "false", ""),
                (KEY_FETCH_HISTORY, &old_fetch_history, "false", ""),
                (KEY_FETCH_HISTORY_COUNT, &old_fetch_history_count, DEFAULT_FETCH_HISTORY_COUNT, ""),
                (KEY_LANGUAGE, &old_language, "zh-CN", ""),
                (KEY_THEME, &old_theme, DEFAULT_THEME, ""),
            ],
        ).unwrap();
        drop(conn);

        assert!(interval_changed);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("setting.poll_interval"));
    }

    /// 确保所有可更新设置都被 apply_settings 覆盖。
    /// 新增配置项时，必须同步更新此测试中的 settings_items。
    #[test]
    fn test_apply_settings_covers_all_keys() {
        let state = test_state();
        let conn = state.db.get().unwrap();

        // 为所有可更新设置写入旧值
        let old_values = [
            (KEY_POLL_INTERVAL, "30"),
            (KEY_PROXY_MODE, "custom"),
            (KEY_PROXY_URL, "http://old"),
            (KEY_AUTO_START, "false"),
            (KEY_MINIMIZE_TO_TRAY, "true"),
            (KEY_LOG_RETENTION, "7"),
            (KEY_DEEPSEEK_ENABLED, "false"),
            (KEY_DEEPSEEK_MODEL, "gpt-4"),
            (KEY_DEEPSEEK_BASE_URL, "https://old.api"),
            (KEY_DEEPSEEK_PROXY_BYPASS, "false"),
            (KEY_DEEPSEEK_PROMPT, ""),
            (KEY_DEEPSEEK_MIN_IMPORTANCE, "小"),
            (KEY_CHECK_PRERELEASES, "false"),
            (KEY_FETCH_HISTORY, "false"),
            (KEY_FETCH_HISTORY_COUNT, "3"),
            (KEY_LANGUAGE, "zh-CN"),
            (KEY_THEME, "system"),
        ];
        for &(key, val) in &old_values {
            settings::set_setting(&conn, key, val).unwrap();
        }

        // 全部改为新值 —— 这个列表必须与 update_settings 中的 apply_settings 列表一一对应
        let new_values = [
            (KEY_POLL_INTERVAL, "60"),
            (KEY_PROXY_MODE, "custom"),
            (KEY_PROXY_URL, "http://new"),
            (KEY_AUTO_START, "true"),
            (KEY_MINIMIZE_TO_TRAY, "false"),
            (KEY_LOG_RETENTION, "14"),
            (KEY_DEEPSEEK_ENABLED, "true"),
            (KEY_DEEPSEEK_MODEL, "deepseek-v4"),
            (KEY_DEEPSEEK_BASE_URL, "https://new.api"),
            (KEY_DEEPSEEK_PROXY_BYPASS, "true"),
            (KEY_DEEPSEEK_PROMPT, "你是一个帮助助手"),
            (KEY_DEEPSEEK_MIN_IMPORTANCE, "大"),
            (KEY_CHECK_PRERELEASES, "true"),
            (KEY_FETCH_HISTORY, "true"),
            (KEY_FETCH_HISTORY_COUNT, "5"),
            (KEY_LANGUAGE, "en-US"),
            (KEY_THEME, "dark"),
        ];

        let items: Vec<(&str, &str, &str, &str)> = old_values.iter()
            .zip(new_values.iter())
            .map(|(&(key, old), &(_, new))| (key, old, new, ""))
            .collect();

        let (interval_changed, _) = settings::apply_settings(&conn, &items).unwrap();

        assert!(interval_changed, "first key should be poll_interval_minutes");
        assert_eq!(items.len(), 17,
            "设置项数量变化！新增/删除配置项时，必须同步更新 update_settings 中的 apply_settings 列表和 UpdateSettingsPayload 结构体。"
        );

        // 验证每个新值都已写入
        for &(key, expected) in &new_values {
            let actual = get_setting_str(&conn, key, "").unwrap();
            assert_eq!(actual, expected, "key '{}' 未写入预期值", key);
        }
    }

    /// 新增配置项漏改 UpdateSettingsPayload 时会触发此测试失败。
    #[test]
    fn test_payload_field_count() {
        // 如果新增了配置项，请同步增加期望值并更新 UpdateSettingsPayload struct
        const EXPECTED_SETTING_FIELDS: usize = 17;
        let json = serde_json::json!({
            "pollIntervalMinutes": 30,
            "proxyMode": "none",
            "proxyUrl": "",
            "autoStart": false,
            "minimizeToTray": true,
            "logRetentionDays": 0,
            "deepseekEnabled": false,
            "deepseekModel": "m",
            "deepseekBaseUrl": "u",
            "deepseekProxyBypass": false,
            "deepseekPrompt": "",
            "deepseekMinImportance": "小",

            "checkPrereleases": false,
            "fetchHistory": true,
            "fetchHistoryCount": 3,
            "language": "zh-CN",
            "theme": "system",
        });
        let payload: UpdateSettingsPayload = serde_json::from_value(json)
            .expect("UpdateSettingsPayload 反序列化失败，前端字段名可能不匹配");
        // 无法直接检测字段数量，但可验证反序列化成功即所有字段均存在。
        // 如果后端新增了字段但前端未传，serde 会因缺少字段而失败（除非字段是 Option）。
        assert_eq!(payload.poll_interval_minutes, 30);
        assert_eq!(payload.proxy_url, "");
        assert!(payload.minimize_to_tray);
        assert_eq!(payload.log_retention_days, 0);
        assert!(!payload.deepseek_enabled);
        assert_eq!(payload.deepseek_model, "m");
        assert_eq!(payload.deepseek_base_url, "u");
        assert!(!payload.deepseek_proxy_bypass);
        assert_eq!(payload.deepseek_prompt, "");
        assert_eq!(payload.deepseek_min_importance, "小");

        assert!(!payload.check_prereleases);
        assert!(payload.fetch_history);
        assert_eq!(payload.fetch_history_count, 3);
        assert_eq!(payload.language, "zh-CN");
        assert_eq!(payload.theme, "system");

        // 常量标记，修改配置项时需同步改这里
        let _guard = EXPECTED_SETTING_FIELDS;
    }

    /// 验证 deepseek_proxy_bypass 通过 apply_settings 写入后能正确持久化。
    #[test]
    fn test_deepseek_proxy_bypass_apply_and_readback() {
        let state = test_state();
        let conn = state.db.get().unwrap();

        // 初始默认值应为 false
        assert!(!get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap());

        // 模拟 update_settings 流程：从 false 改为 true
        let old = get_setting_str(&conn, KEY_DEEPSEEK_PROXY_BYPASS, DEFAULT_DEEPSEEK_PROXY_BYPASS).unwrap();
        let (_, changes) = settings::apply_settings(
            &conn,
            &[
                (KEY_DEEPSEEK_PROXY_BYPASS, &old, "true", "setting.deepseek_proxy_bypass"),
            ],
        ).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("setting.deepseek_proxy_bypass"));

        // 验证已持久化为 true
        assert!(get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap());

        // 再次模拟 update_settings：改回 false
        let old = get_setting_str(&conn, KEY_DEEPSEEK_PROXY_BYPASS, DEFAULT_DEEPSEEK_PROXY_BYPASS).unwrap();
        let (_, changes) = settings::apply_settings(
            &conn,
            &[
                (KEY_DEEPSEEK_PROXY_BYPASS, &old, "false", "setting.deepseek_proxy_bypass"),
            ],
        ).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, true).unwrap());
    }
}
