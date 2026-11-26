use rusqlite::Connection;

use crate::credential;
use crate::db;
use crate::db::settings::{
    DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_PROMPT_EDITABLE,
    DEFAULT_DEEPSEEK_TRANSLATE_RELEASE, KEY_DEEPSEEK_API_KEY, KEY_DEEPSEEK_BASE_URL,
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_PROMPT,
    KEY_DEEPSEEK_PROXY_BYPASS, KEY_DEEPSEEK_TRANSLATE_RELEASE, KEY_LANGUAGE,
};

/// DeepSeek 配置聚合（避免按位置取值的元组）。
#[derive(Debug, Clone)]
pub struct DeepSeekConfig {
    pub enabled: bool,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub prompt: String,
}

pub fn read_config(conn: &Connection) -> DeepSeekConfig {
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
    // API Key 走统一凭据管道（读取 → 解密 → v1→v2 迁移回写）
    let api_key = credential::read_credential(conn, KEY_DEEPSEEK_API_KEY);
    let prompt = db::settings::get_setting_str(conn, KEY_DEEPSEEK_PROMPT, DEFAULT_DEEPSEEK_PROMPT_EDITABLE)
        .unwrap_or_else(|_| DEFAULT_DEEPSEEK_PROMPT_EDITABLE.to_string());
    DeepSeekConfig {
        enabled,
        model,
        base_url,
        api_key,
        prompt,
    }
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

/// 读取 DeepSeek 网络配置（proxy 直连/自定义 + 连接测试专用 bypass 开关）。
/// 摘要与翻译两个入口共用。
/// 返回 (proxy_url, proxy_mode)，bypass 时强制直连。
pub fn load_ai_network_config(conn: &Connection) -> (String, String) {
    let bypass = db::settings::get_setting(conn, KEY_DEEPSEEK_PROXY_BYPASS)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    if bypass {
        return (String::new(), "none".to_string());
    }
    let proxy = db::settings::load_proxy_config(conn).unwrap_or_default();
    (proxy.url, proxy.mode)
}

