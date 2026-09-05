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
    KEY_DEEPSEEK_TRANSLATE_RELEASE,
    KEY_AUTO_START,
    KEY_CHECK_PRERELEASES, KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT,
    KEY_LANGUAGE, KEY_THEME, KEY_FONT_SCALE, KEY_SHOW_SOURCE_TYPE_ICONS, KEY_GITHUB_TOKEN, KEY_YOUTUBE_API_KEY, KEY_BILIBILI_COOKIE, KEY_NEXT_POLL_AT, KEY_ENABLE_USAGE_STATS,
    DEFAULT_PROXY_URL,
    DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_BASE_URL,
    DEFAULT_DEEPSEEK_PROMPT_EDITABLE, DEFAULT_DEEPSEEK_MIN_IMPORTANCE,
    DEFAULT_THEME,
    FONT_SCALE_MIN, FONT_SCALE_MAX,
    SETTING_SPECS,
    get_setting_str, get_setting_bool, get_setting_i64, get_default_language,
    strip_prompt_suffix,
};
use serde_json::json;

#[tauri::command]

#[specta::specta]pub fn get_settings(state: tauri::State<AppState>) -> Result<AppSettings, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
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
        deepseek_translate_release: get_setting_bool(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, false)?,

        check_prereleases: get_setting_bool(&conn, KEY_CHECK_PRERELEASES, false)?,
        fetch_history: get_setting_bool(&conn, KEY_FETCH_HISTORY, false)?,
        fetch_history_count: get_setting_i64(&conn, KEY_FETCH_HISTORY_COUNT, 1)?.max(0),
        language: get_setting_str(&conn, KEY_LANGUAGE, &get_default_language())?,
        theme: get_setting_str(&conn, KEY_THEME, DEFAULT_THEME)?,
        // 默认 100 与 db::settings::DEFAULT_FONT_SCALE 对应；clamp 防御 DB 中越界的旧值
        font_scale: get_setting_i64(&conn, KEY_FONT_SCALE, 100)?.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX),
        show_source_type_icons: get_setting_bool(&conn, KEY_SHOW_SOURCE_TYPE_ICONS, true)?,
        enable_usage_stats: get_setting_bool(&conn, KEY_ENABLE_USAGE_STATS, true)?,
        github_token_set: get_setting_str(&conn, KEY_GITHUB_TOKEN, "")?
            .chars()
            .next()
            .is_some(),
        youtube_api_key_set: get_setting_str(&conn, KEY_YOUTUBE_API_KEY, "")?
            .chars()
            .next()
            .is_some(),
        bilibili_cookie_set: get_setting_str(&conn, KEY_BILIBILI_COOKIE, "")?
            .chars()
            .next()
            .is_some(),
    })
}

/// 设置读写共用 `AppSettings`（types.rs）：不再维护第二份与 AppSettings
/// 逐字段重复的 payload 结构，新增设置项少一处同步点。
#[tauri::command]

#[specta::specta]pub async fn update_settings(
    app: tauri::AppHandle,
    payload: AppSettings,
) -> Result<(), String> {
    // 边界 clamp（保持原有行为）与 prompt 后缀剥离在写入前统一处理
    let payload = AppSettings {
        poll_interval_minutes: payload.poll_interval_minutes.clamp(5, 1440),
        log_retention_days: payload.log_retention_days.clamp(0, 3650),
        fetch_history_count: payload.fetch_history_count.max(0),
        font_scale: payload.font_scale.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX),
        deepseek_prompt: strip_prompt_suffix(&payload.deepseek_prompt),
        ..payload
    };

    // 拿到 pool 与 next_poll_at 的克隆，避免跨 await 持有 tauri::State 借用。
    // 同步 SQLite I/O（读旧值 + 写设置项 + 写日志）与系统注册表写入（自启动）
    // 一并放进 spawn_blocking，避免在 Tauri 主线程冻结 UI（轮询高峰期 pool.get
    // 可能等待，且 autostart 注册表写入是阻塞 I/O）。
    let state = app.state::<AppState>();
    let pool = state.db.clone();
    let next_poll_at = state.next_poll_at.clone();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = pool.get().map_err(|e| e.to_string())?;

        // 注册表驱动（M2）：payload 序列化为 (字段名 → JSON 值) 映射，字段名与
        // DB key 同名（snake_case）；按 SETTING_SPECS 统一「读旧值 → 比较 → 写入」，
        // 不再手写 20 个 old 变量与 20 行元组表。新增设置项只需改 AppSettings +
        // SETTING_SPECS 一处，此处代码无需变动。
        let payload_map = serde_json::to_value(&payload)
            .map_err(|e| format!("err.settings_serialize|{}", e))?
            .as_object()
            .cloned()
            .ok_or_else(|| "err.settings_serialize|not-an-object".to_string())?;
        let json_to_str = json_setting_value;
        let new_values: Vec<String> = SETTING_SPECS
            .iter()
            .map(|s| {
                payload_map
                    .get(s.key)
                    .map(&json_to_str)
                    .unwrap_or_else(|| s.default.to_string())
            })
            .collect();
        let old_values: Vec<String> = SETTING_SPECS
            .iter()
            .map(|s| get_setting_str(&conn, s.key, s.default))
            .collect::<Result<_, _>>()?;
        let items: Vec<(&str, &str, &str, &str)> = SETTING_SPECS
            .iter()
            .zip(old_values.iter())
            .zip(new_values.iter())
            .map(|((s, old), new)| (s.key, old.as_str(), new.as_str(), s.label))
            .collect();
        // 开机自启动变化判断（写入后需触发系统注册表注册/注销）
        let auto_start_idx = SETTING_SPECS
            .iter()
            .position(|s| s.key == KEY_AUTO_START)
            .expect("KEY_AUTO_START 必须在 SETTING_SPECS 中");
        let auto_start_changed =
            old_values[auto_start_idx] != new_values[auto_start_idx];

        let (interval_changed, changes) = settings::apply_settings(&conn, &items)?;

        if changes.is_empty() {
            return Ok(());
        }

        db::logs::write_log_key(&conn, "INFO", "setting.updated", &json!({"changes": changes.join(", ")}).to_string());

        if interval_changed {
            let next = chrono::Utc::now().timestamp() + payload.poll_interval_minutes * 60;
            next_poll_at.store(next, Ordering::Relaxed);
            let _ = settings::set_setting(&conn, KEY_NEXT_POLL_AT, &next.to_string());
            // 唤醒轮询线程重算下次执行时间，新间隔立即生效（不再等逐秒轮询）
            crate::poll::notify_poll_wake();
        }

        // 开机自启动变化时，立即执行系统注册/注销（注册表写入是阻塞 I/O，已在 spawn_blocking 内）
        if auto_start_changed {
            if payload.auto_start {
                autostart::enable().map_err(|e| format!("err.autostart_enable_failed|{}", e))?;
            } else {
                autostart::disable().map_err(|e| format!("err.autostart_disable_failed|{}", e))?;
            }
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("err.task_failed|update_settings|{}", e))?
}

/// 凭据 kind → (DB key, 日志 label) 注册表。
/// 四个 `set_deepseek_api_key` / `set_github_token` / `set_youtube_api_key` /
/// `set_bilibili_cookie` 命令除 key 名外逐字相同（M2），合并为单个
/// `set_credential(kind, value)`：新增凭据只需在此登记一行。
const CREDENTIAL_KINDS: &[(&str, &str, &str)] = &[
    ("deepseek_api_key", KEY_DEEPSEEK_API_KEY, "setting.deepseek_key_updated"),
    ("github_token", KEY_GITHUB_TOKEN, "setting.github_token_updated"),
    ("youtube_api_key", KEY_YOUTUBE_API_KEY, "setting.youtube_key_updated"),
    ("bilibili_cookie", KEY_BILIBILI_COOKIE, "setting.bilibili_cookie_updated"),
];

/// 把 payload JSON 值序列化为 DB 存储字符串：bool → "true"/"false"、
/// 数字 → 十进制、字符串 → 原样（M2 注册表驱动的序列化规则）。
fn json_setting_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// 凭据写入核心逻辑（与 tauri::State 解耦，便于测试）：
/// 未知 kind 报错（不静默）；空值清除；非空加密存储 + 写变更日志。
fn set_credential_impl(
    conn: &rusqlite::Connection,
    kind: &str,
    value: &str,
) -> Result<(), String> {
    let (key, label) = CREDENTIAL_KINDS
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, key, label)| (*key, *label))
        .ok_or_else(|| format!("err.unknown_credential_kind|{}", kind))?;
    if value.is_empty() {
        settings::set_setting(conn, key, "")?;
    } else {
        let encrypted = crypto::encrypt(value);
        settings::set_setting(conn, key, &encrypted)?;
    }
    db::logs::write_log_key(conn, "INFO", label, &json!({}).to_string());
    Ok(())
}

/// 设置/更新单个加密凭据：空值清除，非空值加密存储。
/// `kind` 必须命中 `CREDENTIAL_KINDS` 注册表，未知 kind 返回错误（不静默）。
#[tauri::command]

#[specta::specta]pub fn set_credential(
    state: tauri::State<AppState>,
    kind: String,
    value: String,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| e.to_string())?;
    set_credential_impl(&conn, &kind, &value)
}

/// 测试连接的可选覆盖参数：前端把表单当前值（含未保存修改）传入，
/// 留空的项回退到已保存配置，实现"先试后存"。
#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TestDeepseekPayload {
    model: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    proxy_bypass: Option<bool>,
    proxy_url: Option<String>,
    proxy_mode: Option<String>,
}

#[tauri::command]

#[specta::specta]pub async fn test_deepseek_connection(
    state: tauri::State<'_, AppState>,
    payload: Option<TestDeepseekPayload>,
) -> Result<(), String> {
    let (model, base_url, api_key, proxy_url, proxy_mode);
    {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let config = deepseek::read_config(&conn);
        let p = payload.as_ref();
        // 空白字符串视为未提供（如 API Key 输入框留空 = 沿用已保存的 key）
        let non_empty = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());

        model = p.and_then(|p| non_empty(&p.model)).unwrap_or(config.model);
        base_url = p.and_then(|p| non_empty(&p.base_url)).unwrap_or(config.base_url);
        api_key = p.and_then(|p| non_empty(&p.api_key)).or(config.api_key);
        let bypass = match p.and_then(|p| p.proxy_bypass) {
            Some(b) => b,
            None => get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false)?,
        };
        if bypass {
            proxy_url = String::new();
            proxy_mode = "none".to_string();
        } else {
            proxy_url = match p.and_then(|p| non_empty(&p.proxy_url)) {
                Some(u) => u,
                None => get_setting_str(&conn, KEY_PROXY_URL, DEFAULT_PROXY_URL)?,
            };
            proxy_mode = match p.and_then(|p| non_empty(&p.proxy_mode)) {
                Some(m) => m,
                None => get_setting_str(&conn, KEY_PROXY_MODE, "none")?,
            };
        }
    }
    let api_key = api_key.ok_or("err.deepseek_api_key_missing")?;
    let client = deepseek::build_client(
        &api_key,
        &proxy_url,
        &proxy_mode,
        deepseek::DEEPSEEK_TIMEOUT_SECS_TEST,
    )?;
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": "Hi"}
        ],
        "max_tokens": 10,
        "temperature": 0.0
    });
    // 复用 deepseek::chat_completion 的 POST 模板（连接测试不再自写第 4 份）
    let outcome = deepseek::chat_completion(&client, &base_url, &body)
        .await
        .map_err(|(status, msg)| {
            if status > 0 {
                format!("err.api_error|{}|{}", status, msg)
            } else {
                format!("err.request_failed|{}", msg)
            }
        })?;
    // 连接测试也是真实消耗，记一条 usage（无源：source_id/release_id 均为 NULL）；
    // 落库失败静默——统计不影响测试结果。
    if let Ok(conn) = state.db.get() {
        let usage = crate::db::ai_usage::CallUsage::from_outcome(
            "test",
            outcome.usage,
            &outcome.content,
            deepseek::count_prompt_chars(&body),
            outcome.duration_ms,
        );
        if let Err(e) = crate::db::ai_usage::insert_call_usage(&conn, None, &model, &[usage]) {
            log::warn!("记录连接测试用量失败: {}", e);
        }
    }
    // 成功不返回任何文案：命令只承诺“测试通过”，提示语由前端按 i18n key 渲染
    // （settings.connection_success），避免后端硬编码语言与调用方无法预判返回语义。
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::settings::{
        DEFAULT_AUTO_START, DEFAULT_DEEPSEEK_PROXY_BYPASS, DEFAULT_FETCH_HISTORY_COUNT,
    };
    use std::sync::Arc;

    fn test_state() -> AppState {
        AppState {
            db: db::init::init_memory_pool().unwrap(),
            next_poll_at: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            deepseek_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(50)),
            agent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            agent_rpc: std::sync::Arc::new(crate::agent_rpc::RpcManager::new(db::init::init_memory_pool().unwrap())),
            agent_cancelled: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
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
            (KEY_DEEPSEEK_TRANSLATE_RELEASE, "false"),
            (KEY_CHECK_PRERELEASES, "false"),
            (KEY_FETCH_HISTORY, "false"),
            (KEY_FETCH_HISTORY_COUNT, "3"),
            (KEY_LANGUAGE, "zh-CN"),
            (KEY_THEME, "system"),
            (KEY_FONT_SCALE, "100"),
            (KEY_SHOW_SOURCE_TYPE_ICONS, "true"),
            (KEY_ENABLE_USAGE_STATS, "true"),
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
            (KEY_DEEPSEEK_TRANSLATE_RELEASE, "true"),
            (KEY_CHECK_PRERELEASES, "true"),
            (KEY_FETCH_HISTORY, "true"),
            (KEY_FETCH_HISTORY_COUNT, "5"),
            (KEY_LANGUAGE, "en-US"),
            (KEY_THEME, "dark"),
            (KEY_FONT_SCALE, "125"),
            (KEY_SHOW_SOURCE_TYPE_ICONS, "false"),
            (KEY_ENABLE_USAGE_STATS, "false"),
        ];

        let items: Vec<(&str, &str, &str, &str)> = old_values.iter()
            .zip(new_values.iter())
            .map(|(&(key, old), &(_, new))| (key, old, new, ""))
            .collect();

        let (interval_changed, _) = settings::apply_settings(&conn, &items).unwrap();

        assert!(interval_changed, "first key should be poll_interval_minutes");
        assert_eq!(items.len(), 21,
            "设置项数量变化！新增/删除配置项时，必须同步更新 update_settings 中的 apply_settings 列表和 UpdateSettingsPayload 结构体。"
        );

        // 验证每个新值都已写入
        for &(key, expected) in &new_values {
            let actual = get_setting_str(&conn, key, "").unwrap();
            assert_eq!(actual, expected, "key '{}' 未写入预期值", key);
        }
    }

    /// 新增配置项漏改 update_settings 参数（AppSettings）时会触发此测试失败。
    #[test]
    fn test_payload_field_count() {
        // 如果新增了配置项，请同步增加期望值并更新 AppSettings 结构体
        const EXPECTED_SETTING_FIELDS: usize = 20;
        let json = serde_json::json!({
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
            "deepseek_prompt": "",
            "deepseek_min_importance": "小",
            "deepseek_translate_release": false,

            "check_prereleases": false,
            "fetch_history": true,
            "fetch_history_count": 3,
            "language": "zh-CN",
            "theme": "system",
            "font_scale": 100,
            "show_source_type_icons": true,
            "enable_usage_stats": true,
            "github_token_set": false,
            "youtube_api_key_set": false,
            "bilibili_cookie_set": false,
        });
        let payload: AppSettings = serde_json::from_value(json)
            .expect("AppSettings 反序列化失败，前端字段名可能不匹配");
        // 无法直接检测字段数量，但可验证反序列化成功即所有字段均存在。
        // 如果后端新增了字段但前端未传，serde 会因缺少字段而失败。
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
        assert!(!payload.deepseek_translate_release);

        assert!(!payload.check_prereleases);
        assert!(payload.fetch_history);
        assert_eq!(payload.fetch_history_count, 3);
        assert_eq!(payload.language, "zh-CN");
        assert_eq!(payload.theme, "system");
        assert!(payload.show_source_type_icons);
        assert!(payload.enable_usage_stats);

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

    /// M2 防线：SETTING_SPECS 注册表与 AppSettings 字段一一对应，
    /// 新增设置项漏改任一侧都会在此失败。
    #[test]
    fn test_setting_specs_cover_all_app_settings_fields() {
        let json = serde_json::json!({
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
            "deepseek_prompt": "",
            "deepseek_min_importance": "小",
            "deepseek_translate_release": false,

            "check_prereleases": false,
            "fetch_history": true,
            "fetch_history_count": 3,
            "language": "zh-CN",
            "theme": "system",
            "font_scale": 100,
            "show_source_type_icons": true,
            "enable_usage_stats": true,
            "github_token_set": false,
            "youtube_api_key_set": false,
            "bilibili_cookie_set": false,
        });
        let payload: AppSettings = serde_json::from_value(json).unwrap();
        let map = serde_json::to_value(&payload).unwrap();
        let obj = map.as_object().unwrap();

        // 每个注册表 key 都必须能在 AppSettings 中找到（序列化后字段名 = DB key）
        for spec in SETTING_SPECS {
            assert!(
                obj.contains_key(spec.key),
                "SETTING_SPECS 中的 key '{}' 在 AppSettings 中不存在（字段名需与 DB key 一致）",
                spec.key
            );
        }
        // 数量守卫：注册表必须覆盖全部可更新项（当前 21 项）
        assert_eq!(
            SETTING_SPECS.len(),
            21,
            "可更新设置项数量变化！新增/删除配置项时需同步 AppSettings 与 SETTING_SPECS。"
        );
    }

    /// M2 防线：payload JSON 值 → DB 字符串的序列化规则。
    #[test]
    fn test_json_setting_value_serialization() {
        assert_eq!(json_setting_value(&serde_json::json!(true)), "true");
        assert_eq!(json_setting_value(&serde_json::json!(false)), "false");
        assert_eq!(json_setting_value(&serde_json::json!(30)), "30");
        assert_eq!(json_setting_value(&serde_json::json!(0)), "0");
        assert_eq!(json_setting_value(&serde_json::json!("zh-CN")), "zh-CN");
        assert_eq!(json_setting_value(&serde_json::json!("")), "");
        // 非常规类型（null/数组）不产生写入值
        assert_eq!(json_setting_value(&serde_json::json!(null)), "");
        assert_eq!(json_setting_value(&serde_json::json!([1])), "");
    }

    /// M2：set_credential 未知 kind 拒绝，空值清除，非空加密存储。
    #[test]
    fn test_set_credential_encrypts_and_clears() {
        crate::crypto::set_test_master_key();
        let state = test_state();
        {
            let conn = state.db.get().unwrap();
            // 未知 kind：报错且不写库
            let err = set_credential_impl(&conn, "evil_kind", "x").unwrap_err();
            assert!(err.starts_with("err.unknown_credential_kind|evil_kind"));
            assert!(settings::get_setting(&conn, KEY_GITHUB_TOKEN).unwrap().is_none());
        }
        {
            let conn = state.db.get().unwrap();
            // 非空值：加密存储（不是明文）
            set_credential_impl(&conn, "github_token", "ghp_secret").unwrap();
            let stored = settings::get_setting(&conn, KEY_GITHUB_TOKEN).unwrap().unwrap();
            assert!(!stored.contains("ghp_secret"), "凭据应以密文存储，实际: {}", stored);
            // 空值：清除
            set_credential_impl(&conn, "github_token", "").unwrap();
            assert_eq!(settings::get_setting(&conn, KEY_GITHUB_TOKEN).unwrap().unwrap(), "");
        }
    }
}
