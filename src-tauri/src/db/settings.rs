use rusqlite::{params, Connection};

// ── 设置键常量 ──────────────────────────────────────
pub const KEY_POLL_INTERVAL: &str = "poll_interval_minutes";
pub const KEY_PROXY_URL: &str = "proxy_url";
pub const KEY_PROXY_MODE: &str = "proxy_mode";
pub const KEY_MINIMIZE_TO_TRAY: &str = "minimize_to_tray";
pub const KEY_LOG_RETENTION: &str = "log_retention_days";
pub const KEY_DEEPSEEK_ENABLED: &str = "deepseek_enabled";
pub const KEY_DEEPSEEK_MODEL: &str = "deepseek_model";
pub const KEY_DEEPSEEK_BASE_URL: &str = "deepseek_base_url";
pub const KEY_DEEPSEEK_API_KEY: &str = "deepseek_api_key";
pub const KEY_DEEPSEEK_PROXY_BYPASS: &str = "deepseek_proxy_bypass";
pub const KEY_DEEPSEEK_PROMPT: &str = "deepseek_prompt";
pub const KEY_DEEPSEEK_MIN_IMPORTANCE: &str = "deepseek_min_importance";

pub const KEY_CHECK_PRERELEASES: &str = "check_prereleases";
pub const KEY_FETCH_HISTORY: &str = "fetch_history";
pub const KEY_FETCH_HISTORY_COUNT: &str = "fetch_history_count";
pub const KEY_LANGUAGE: &str = "language";
pub const KEY_THEME: &str = "theme";
pub const KEY_AUTO_START: &str = "auto_start";
pub const KEY_GITHUB_TOKEN: &str = "github_token";
pub const KEY_NEXT_POLL_AT: &str = "next_poll_at";

// ── 默认值常量 ──────────────────────────────────────
pub const DEFAULT_POLL_INTERVAL: &str = "30";
pub const DEFAULT_PROXY_URL: &str = "";
pub const DEFAULT_AUTO_START: &str = "false";
pub const DEFAULT_MINIMIZE_TO_TRAY: &str = "true";
pub const DEFAULT_LOG_RETENTION: &str = "0";
pub const DEFAULT_DEEPSEEK_ENABLED: &str = "false";
pub const DEFAULT_DEEPSEEK_MODEL: &str = "deepseek-v4-flash";
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_DEEPSEEK_PROXY_BYPASS: &str = "false";
pub const DEFAULT_DEEPSEEK_PROMPT_EDITABLE: &str = concat!(
    "你是版本发布摘要助手。请用中文总结下面 GitHub Release 更新内容，并评估重要度。\n",
    "\n",
    "摘要长度：2-4句话\n",
    "\n",
    "重要度标准：\n",
    "- 大：breaking changes、重大架构变更、严重安全漏洞修复\n",
    "- 中：新功能、重要 bug 修复、性能优化\n",
    "- 小：小修复、文档更新、依赖升级、日常维护\n",
    "\n",
    "Release 内容：\n",
    "{}"
);

/// 固定追加在用户可编辑提示词后的 JSON 格式约束。
/// 不可编辑——后端依赖此格式解析 AI 返回结果。
pub const DEEPSEEK_PROMPT_FIXED_SUFFIX: &str = concat!(
    "请严格按以下 JSON 格式返回（不要包含其他内容）：\n",
    "{\"summary\":\"简短中文摘要\",\"importance\":\"大|中|小\"}"
);

/// 用于从已存储的提示词中剥离固定后缀的标记字符串。
/// 兼容旧数据（旧版提示词也包含此行）。
pub const PROMPT_STRUCTURAL_BOUNDARY: &str = "请严格按以下 JSON 格式返回";

/// 从完整提示词中剥离固定后缀，仅返回可编辑部分。
pub fn strip_prompt_suffix(prompt: &str) -> String {
    if let Some(pos) = prompt.find(PROMPT_STRUCTURAL_BOUNDARY) {
        prompt[..pos].trim_end().to_string()
    } else {
        prompt.to_string()
    }
}

pub const DEFAULT_DEEPSEEK_MIN_IMPORTANCE: &str = "小";

pub const DEFAULT_CHECK_PRERELEASES: &str = "false";
pub const DEFAULT_FETCH_HISTORY_COUNT: &str = "1";
pub const DEFAULT_THEME: &str = "system";

// ── 语言检测 ────────────────────────────────────────

pub fn get_default_language() -> String {
    match sys_locale::get_locale() {
        Some(locale) if locale.starts_with("zh") => "zh-CN".to_string(),
        _ => "en-US".to_string(),
    }
}

// ── 基础存取 ────────────────────────────────────────

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
    .map(|v| v.flatten())
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 类型化读取封装 ──────────────────────────────────

pub fn get_setting_str(conn: &Connection, key: &str, default: &str) -> Result<String, String> {
    Ok(get_setting(conn, key)?.unwrap_or_else(|| default.to_string()))
}

pub fn get_setting_bool(conn: &Connection, key: &str, default: bool) -> Result<bool, String> {
    Ok(get_setting(conn, key)?
        .map(|v| v == "true")
        .unwrap_or(default))
}

pub fn get_setting_i64(conn: &Connection, key: &str, default: i64) -> Result<i64, String> {
    Ok(get_setting(conn, key)?
        .unwrap_or_else(|| default.to_string())
        .parse::<i64>()
        .unwrap_or(default))
}

// ── 批量应用（仅写入变化的项）───────────────────────

/// 每项为 (key, old_str, new_str, label)。
/// 返回 (第一项是否变化且 key == KEY_POLL_INTERVAL, 变更描述列表)。
pub fn apply_settings(
    conn: &Connection,
    items: &[(&str, &str, &str, &str)],
) -> Result<(bool, Vec<String>), String> {
    let mut first_changed = false;
    let mut changes: Vec<String> = Vec::new();
    let mut first = true;

    for &(key, old_val, new_val, label) in items {
        if old_val != new_val {
            set_setting(conn, key, new_val)?;
            if first {
                first_changed = key == KEY_POLL_INTERVAL;
                first = false;
            }
            if !label.is_empty() {
                changes.push(format!("{}→{}", label, new_val));
            }
        }
    }

    Ok((first_changed, changes))
}

// ── 内部 ────────────────────────────────────────────

trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_settings() {
        let conn = init_memory_db().unwrap();
        assert!(get_setting(&conn, "key").unwrap().is_none());
        set_setting(&conn, "key", "val").unwrap();
        assert_eq!(get_setting(&conn, "key").unwrap().unwrap(), "val");
    }

    #[test]
    fn test_deepseek_settings_defaults() {
        let conn = init_memory_db().unwrap();
        assert!(get_setting(&conn, KEY_DEEPSEEK_ENABLED).unwrap().is_none());
        assert!(get_setting(&conn, KEY_DEEPSEEK_API_KEY).unwrap().is_none());
        set_setting(&conn, KEY_DEEPSEEK_ENABLED, "true").unwrap();
        assert_eq!(
            get_setting(&conn, KEY_DEEPSEEK_ENABLED).unwrap().unwrap(),
            "true"
        );
    }

    #[test]
    fn test_get_setting_str() {
        let conn = init_memory_db().unwrap();
        assert_eq!(get_setting_str(&conn, "missing", "fallback").unwrap(), "fallback");
        set_setting(&conn, "foo", "bar").unwrap();
        assert_eq!(get_setting_str(&conn, "foo", "").unwrap(), "bar");
    }

    #[test]
    fn test_get_setting_bool() {
        let conn = init_memory_db().unwrap();
        assert!(get_setting_bool(&conn, "missing", true).unwrap());
        assert!(!get_setting_bool(&conn, "missing", false).unwrap());
        set_setting(&conn, "flag", "true").unwrap();
        assert!(get_setting_bool(&conn, "flag", false).unwrap());
    }

    #[test]
    fn test_get_setting_i64() {
        let conn = init_memory_db().unwrap();
        assert_eq!(get_setting_i64(&conn, "missing", 30).unwrap(), 30);
        set_setting(&conn, "num", "42").unwrap();
        assert_eq!(get_setting_i64(&conn, "num", 0).unwrap(), 42);
    }

    #[test]
    fn test_next_poll_at_roundtrip() {
        let conn = init_memory_db().unwrap();
        assert!(get_setting(&conn, KEY_NEXT_POLL_AT).unwrap().is_none());
        set_setting(&conn, KEY_NEXT_POLL_AT, "1234567890").unwrap();
        assert_eq!(
            get_setting(&conn, KEY_NEXT_POLL_AT).unwrap().unwrap(),
            "1234567890"
        );
    }

    #[test]
    fn test_set_setting_empty_value() {
        let conn = init_memory_db().unwrap();
        set_setting(&conn, "key", "").unwrap();
        assert_eq!(get_setting(&conn, "key").unwrap().unwrap(), "");
    }

    #[test]
    fn test_get_setting_i64_non_numeric_fallback() {
        let conn = init_memory_db().unwrap();
        set_setting(&conn, "num", "not-a-number").unwrap();
        // 非数字值应 fallback 到 default
        assert_eq!(get_setting_i64(&conn, "num", 42).unwrap(), 42);
    }

    #[test]
    fn test_apply_settings_no_changes() {
        let conn = init_memory_db().unwrap();
        set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();

        let old = get_setting_str(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        let (changed, changes) = apply_settings(
            &conn,
            &[(KEY_POLL_INTERVAL, &old, "30", "setting.poll_interval")],
        ).unwrap();

        assert!(!changed, "值未变时 first_changed 应为 false");
        assert!(changes.is_empty(), "值未变时不应有变更描述");
    }

    #[test]
    fn test_apply_settings_first_item_triggers_flag() {
        let conn = init_memory_db().unwrap();
        set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();

        let old = get_setting_str(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        let (changed, changes) = apply_settings(
            &conn,
            &[
                (KEY_POLL_INTERVAL, &old, "60", "setting.poll_interval"),
            ],
        ).unwrap();

        assert!(changed, "KEY_POLL_INTERVAL 变化时应返回 first_changed=true");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn test_deepseek_proxy_bypass_defaults() {
        let conn = init_memory_db().unwrap();
        // 默认值应为 false
        assert_eq!(
            get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap(),
            false
        );
        // 写入 true
        set_setting(&conn, KEY_DEEPSEEK_PROXY_BYPASS, "true").unwrap();
        assert_eq!(
            get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap(),
            true
        );
        // 写回 false
        set_setting(&conn, KEY_DEEPSEEK_PROXY_BYPASS, "false").unwrap();
        assert_eq!(
            get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap(),
            false
        );
    }
}
