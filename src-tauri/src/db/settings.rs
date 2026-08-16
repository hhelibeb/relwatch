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
pub const KEY_DEEPSEEK_TRANSLATE_RELEASE: &str = "deepseek_translate_release";

pub const KEY_CHECK_PRERELEASES: &str = "check_prereleases";
pub const KEY_FETCH_HISTORY: &str = "fetch_history";
pub const KEY_FETCH_HISTORY_COUNT: &str = "fetch_history_count";
pub const KEY_LANGUAGE: &str = "language";
pub const KEY_THEME: &str = "theme";
pub const KEY_SHOW_SOURCE_TYPE_ICONS: &str = "show_source_type_icons";
/// 诊断统计开关：是否记录功能按钮点击次数（默认开启）。
/// 关闭后前端 track() 直接 no-op，后端 record_usage 也丢弃写入（双保险）。
pub const KEY_ENABLE_USAGE_STATS: &str = "enable_usage_stats";
pub const KEY_AUTO_START: &str = "auto_start";
pub const KEY_GITHUB_TOKEN: &str = "github_token";
pub const KEY_YOUTUBE_API_KEY: &str = "youtube_api_key";
/// B 站登录 Cookie（SESSDATA，可选，加密存储，降低风控概率）。
pub const KEY_BILIBILI_COOKIE: &str = "bilibili_cookie";
pub const KEY_NEXT_POLL_AT: &str = "next_poll_at";

// ── Agent 全局配置键（不走 SETTING_SPECS 注册表：含 JSON 数组字段，
// 由 db::agent 的 load/save_agent_config 统一读写）──
pub const KEY_AGENT_ENABLED: &str = "agent_enabled";
pub const KEY_AGENT_TYPE: &str = "agent_type";
pub const KEY_AGENT_BINARY: &str = "agent_binary";
pub const KEY_AGENT_MODEL: &str = "agent_model";
pub const KEY_AGENT_PROMPT_SUFFIX: &str = "agent_prompt_suffix";
pub const KEY_AGENT_TIMEOUT_SECONDS: &str = "agent_timeout_seconds";
pub const KEY_AGENT_SKILLS: &str = "agent_skills";
/// Agent 工作区面板宽度（逻辑 px；未设置时前端回退默认 440）。
pub const KEY_AGENT_WS_WIDTH: &str = "agent_ws_width";

/// 声明为加密存储的设置键（master key 加密）。
/// 所有经 `crypto::encrypt` 存储的设置必须登记于此：
/// - `crypto::verify_master_key_consistency` 依赖它做启动时一致性检查，
///   master key 失配（如 Windows keyring 凭据丢失）时自动清空对应项，避免死数据。
///
/// 新增加密键时务必同步登记，否则失配时不会被自动清空、将永久解密失败。
pub const ENCRYPTED_SETTING_KEYS: &[&str] = &[
    KEY_DEEPSEEK_API_KEY,
    KEY_GITHUB_TOKEN,
    KEY_YOUTUBE_API_KEY,
    KEY_BILIBILI_COOKIE,
];

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
pub const DEFAULT_DEEPSEEK_TRANSLATE_RELEASE: &str = "false";
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

/// 翻译 release note 全文使用的固定提示词模板。
/// `{}` 为 release body 占位符，`{lang}` 为目标语言（运行时注入）。
/// 不对用户开放编辑，避免与摘要提示词的 `{}` 占位校验冲突。
pub const DEFAULT_DEEPSEEK_TRANSLATE_PROMPT: &str = concat!(
    "请将以下 GitHub Release 更新说明完整翻译成{lang}。",
    "要求：\n",
    "1. 必须翻译全部内容，覆盖输入的每一段、每一句，不得跳过或省略任何段落；\n",
    "2. 保留原文的 Markdown 格式、代码块、链接 URL、标题层级；\n",
    "3. 代码标识符、命令、配置键、@用户名、#编号、版本号等技术性 token 原样保留；\n",
    "4. 链接的 URL 原样保留，但链接的显示文本若为自然语言则翻译；\n",
    "5. 不要添加任何解释、注释、前后缀，直接输出完整译文。\n\n",
    "Release 内容：\n{}"
);

pub const DEFAULT_CHECK_PRERELEASES: &str = "false";
pub const DEFAULT_FETCH_HISTORY_COUNT: &str = "1";
pub const DEFAULT_THEME: &str = "system";
pub const DEFAULT_SHOW_SOURCE_TYPE_ICONS: &str = "true";
pub const DEFAULT_ENABLE_USAGE_STATS: &str = "true";

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

// ── 可更新设置项注册表 ─────────────────────────────

/// 单个设置项元数据：DB key、默认值、变更日志 label。
pub struct SettingSpec {
    pub key: &'static str,
    pub default: &'static str,
    pub label: &'static str,
}

/// 可更新设置项注册表：(key, 默认值, 日志 label)。
/// `update_settings` 按此注册表驱动「读旧值 → 比较 → 写入」，不再手写
/// 逐个 old 变量与元组表；新增可更新设置项只需：AppSettings 加字段 +
/// 此处加一行（key/default/label 集中一处，无需再同步 KEY/DEFAULT 常量
/// 之外的第三份清单）。
///
/// 注意：注册表不覆盖 `*_set` 派生只读字段（凭据是否已设置，不入库）。
pub const SETTING_SPECS: &[SettingSpec] = &[
    SettingSpec { key: KEY_POLL_INTERVAL, default: DEFAULT_POLL_INTERVAL, label: "setting.poll_interval" },
    SettingSpec { key: KEY_PROXY_MODE, default: "none", label: "setting.proxy_mode" },
    SettingSpec { key: KEY_PROXY_URL, default: DEFAULT_PROXY_URL, label: "setting.proxy_url" },
    SettingSpec { key: KEY_AUTO_START, default: DEFAULT_AUTO_START, label: "setting.auto_start" },
    SettingSpec { key: KEY_MINIMIZE_TO_TRAY, default: DEFAULT_MINIMIZE_TO_TRAY, label: "setting.minimize_to_tray" },
    SettingSpec { key: KEY_LOG_RETENTION, default: DEFAULT_LOG_RETENTION, label: "setting.log_retention_days" },
    SettingSpec { key: KEY_DEEPSEEK_ENABLED, default: DEFAULT_DEEPSEEK_ENABLED, label: "setting.deepseek_enabled" },
    SettingSpec { key: KEY_DEEPSEEK_MODEL, default: DEFAULT_DEEPSEEK_MODEL, label: "setting.deepseek_model" },
    SettingSpec { key: KEY_DEEPSEEK_BASE_URL, default: DEFAULT_DEEPSEEK_BASE_URL, label: "setting.deepseek_base_url" },
    SettingSpec { key: KEY_DEEPSEEK_PROXY_BYPASS, default: DEFAULT_DEEPSEEK_PROXY_BYPASS, label: "setting.deepseek_proxy_bypass" },
    SettingSpec { key: KEY_DEEPSEEK_PROMPT, default: DEFAULT_DEEPSEEK_PROMPT_EDITABLE, label: "setting.deepseek_prompt" },
    SettingSpec { key: KEY_DEEPSEEK_MIN_IMPORTANCE, default: DEFAULT_DEEPSEEK_MIN_IMPORTANCE, label: "setting.deepseek_min_importance" },
    SettingSpec { key: KEY_DEEPSEEK_TRANSLATE_RELEASE, default: DEFAULT_DEEPSEEK_TRANSLATE_RELEASE, label: "setting.deepseek_translate_release" },
    SettingSpec { key: KEY_CHECK_PRERELEASES, default: DEFAULT_CHECK_PRERELEASES, label: "setting.check_prereleases" },
    SettingSpec { key: KEY_FETCH_HISTORY, default: "false", label: "setting.fetch_history" },
    SettingSpec { key: KEY_FETCH_HISTORY_COUNT, default: DEFAULT_FETCH_HISTORY_COUNT, label: "setting.fetch_history_count" },
    SettingSpec { key: KEY_LANGUAGE, default: "", label: "setting.language" },
    SettingSpec { key: KEY_THEME, default: DEFAULT_THEME, label: "setting.theme" },
    SettingSpec { key: KEY_SHOW_SOURCE_TYPE_ICONS, default: DEFAULT_SHOW_SOURCE_TYPE_ICONS, label: "setting.show_source_type_icons" },
    SettingSpec { key: KEY_ENABLE_USAGE_STATS, default: DEFAULT_ENABLE_USAGE_STATS, label: "setting.enable_usage_stats" },
];

// ── 批量应用（仅写入变化的项）───────────────────────

/// 每项为 (key, old_str, new_str, label)。
/// 返回 (轮询周期是否发生变化, 变更描述列表)。
///
/// `interval_changed` 的判定看**任意**一项 key == KEY_POLL_INTERVAL 的设置发生变化，
/// 而非“第一个变更项”是否为轮询周期——旧实现依赖调用方把 poll_interval 放在数组第一位，
/// 一旦其它项同时变更，轮询周期变更会被吞掉， leading to 不重算 next_poll_at。
pub fn apply_settings(
    conn: &Connection,
    items: &[(&str, &str, &str, &str)],
) -> Result<(bool, Vec<String>), String> {
    let mut interval_changed = false;
    let mut changes: Vec<String> = Vec::new();

    for &(key, old_val, new_val, label) in items {
        if old_val != new_val {
            set_setting(conn, key, new_val)?;
            if key == KEY_POLL_INTERVAL {
                interval_changed = true;
            }
            if !label.is_empty() {
                changes.push(format!("{}→{}", label, new_val));
            }
        }
    }

    Ok((interval_changed, changes))
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

        assert!(changed, "KEY_POLL_INTERVAL 变化时应返回 interval_changed=true");
        assert_eq!(changes.len(), 1);
    }

    /// 问题4 回归测试：轮询周期变更但不是数组中第一个变更项时，interval_changed 仍应为 true。
    /// 旧实现只看“第一个变更项”，会把被前面其它项掩盖的 poll_interval 变更吞掉。
    #[test]
    fn test_apply_settings_interval_not_first_still_triggers_flag() {
        let conn = init_memory_db().unwrap();
        set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        set_setting(&conn, KEY_PROXY_URL, "old").unwrap();

        let old_interval = get_setting_str(&conn, KEY_POLL_INTERVAL, "30").unwrap();
        let old_proxy = get_setting_str(&conn, KEY_PROXY_URL, "").unwrap();
        // proxy_url 放在 poll_interval 之前且也变更——旧实现会因“第一个变更项是 proxy”
        // 而错报 interval_changed=false。
        let (changed, changes) = apply_settings(
            &conn,
            &[
                (KEY_PROXY_URL, &old_proxy, "new", "setting.proxy_url"),
                (KEY_POLL_INTERVAL, &old_interval, "60", "setting.poll_interval"),
            ],
        ).unwrap();

        assert!(changed, "poll_interval 变化时 interval_changed 必须为 true，无论它在数组中的位置");
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn test_deepseek_proxy_bypass_defaults() {
        let conn = init_memory_db().unwrap();
        // 默认值应为 false
        assert!(!get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap());
        // 写入 true
        set_setting(&conn, KEY_DEEPSEEK_PROXY_BYPASS, "true").unwrap();
        assert!(get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap());
        // 写回 false
        set_setting(&conn, KEY_DEEPSEEK_PROXY_BYPASS, "false").unwrap();
        assert!(!get_setting_bool(&conn, KEY_DEEPSEEK_PROXY_BYPASS, false).unwrap());
    }

    #[test]
    fn test_strip_prompt_suffix_with_boundary() {
        let result = strip_prompt_suffix("你是一个助手。\n\n请严格按以下 JSON 格式返回\n{\"summary\": ...}");
        assert_eq!(result, "你是一个助手。");
    }

    #[test]
    fn test_strip_prompt_suffix_without_boundary() {
        let result = strip_prompt_suffix("你是一个助手。请进行分析。");
        assert_eq!(result, "你是一个助手。请进行分析。");
    }

    #[test]
    fn test_strip_prompt_suffix_boundary_at_start() {
        let result = strip_prompt_suffix("请严格按以下 JSON 格式返回\n{\"summary\": ...}");
        assert_eq!(result, "");
    }

    #[test]
    fn test_strip_prompt_suffix_exact_boundary() {
        let result = strip_prompt_suffix("请严格按以下 JSON 格式返回");
        assert_eq!(result, "");
    }

    #[test]
    fn test_strip_prompt_suffix_empty_string() {
        let result = strip_prompt_suffix("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_strip_prompt_suffix_boundary_with_trailing_spaces() {
        let result = strip_prompt_suffix("你是一个助手。  \n请严格按以下 JSON 格式返回\n{\"summary\": ...}");
        // trim_end() 会去掉 boundary 之前的空格
        assert_eq!(result, "你是一个助手。");
    }
}
