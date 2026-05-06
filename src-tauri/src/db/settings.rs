use rusqlite::{params, Connection};

// ── 设置键常量 ──────────────────────────────────────
pub const KEY_POLL_INTERVAL: &str = "poll_interval_minutes";
pub const KEY_PROXY_URL: &str = "proxy_url";
pub const KEY_MINIMIZE_TO_TRAY: &str = "minimize_to_tray";
pub const KEY_LOG_RETENTION: &str = "log_retention_days";
pub const KEY_DEEPSEEK_ENABLED: &str = "deepseek_enabled";
pub const KEY_DEEPSEEK_MODEL: &str = "deepseek_model";
pub const KEY_DEEPSEEK_BASE_URL: &str = "deepseek_base_url";
pub const KEY_DEEPSEEK_API_KEY: &str = "deepseek_api_key";
pub const KEY_DEEPSEEK_PROXY: &str = "deepseek_proxy_enabled";
pub const KEY_CHECK_PRERELEASES: &str = "check_prereleases";
pub const KEY_LANGUAGE: &str = "language";
pub const KEY_GITHUB_TOKEN: &str = "github_token";
pub const KEY_LAST_POLL_AT: &str = "last_poll_at";

// ── 默认值常量 ──────────────────────────────────────
pub const DEFAULT_POLL_INTERVAL: &str = "30";
pub const DEFAULT_PROXY_URL: &str = "";
pub const DEFAULT_MINIMIZE_TO_TRAY: &str = "true";
pub const DEFAULT_LOG_RETENTION: &str = "0";
pub const DEFAULT_DEEPSEEK_ENABLED: &str = "false";
pub const DEFAULT_DEEPSEEK_MODEL: &str = "deepseek-v4-flash";
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_DEEPSEEK_PROXY: &str = "false";
pub const DEFAULT_CHECK_PRERELEASES: &str = "false";

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
    fn test_last_poll_at_roundtrip() {
        let conn = init_memory_db().unwrap();
        assert!(get_setting(&conn, KEY_LAST_POLL_AT).unwrap().is_none());
        set_setting(&conn, KEY_LAST_POLL_AT, "1234567890").unwrap();
        assert_eq!(
            get_setting(&conn, KEY_LAST_POLL_AT).unwrap().unwrap(),
            "1234567890"
        );
    }
}
