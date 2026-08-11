use std::collections::HashMap;
use std::sync::OnceLock;
use serde_json::Value;

// ── 日志相关的 i18n key 翻译表 ──────────────────────
// 字典由 build.rs 从 src/i18n/zh-CN.ts / en-US.ts 生成（单一来源）。
// 修改文案请编辑 TS 文件；此处禁止手写字典，防止两侧漂移。

include!(concat!(env!("OUT_DIR"), "/i18n_generated.rs"));

fn zh_cn() -> &'static HashMap<&'static str, &'static str> {
    static ZH_CN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    ZH_CN.get_or_init(|| ZH_CN_ENTRIES.iter().copied().collect())
}

fn en_us() -> &'static HashMap<&'static str, &'static str> {
    static EN_US: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    EN_US.get_or_init(|| EN_US_ENTRIES.iter().copied().collect())
}


/// action 状态值 → i18n key 的映射
fn action_to_key(action: &str) -> &'static str {
    match action {
        "pending" => "status.pending",
        "ignored" => "status.ignored",
        "snoozed" => "status.snoozed",
        "clicked" => "status.viewed",
        _ => "status.viewed", // fallback, 不应到达
    }
}

/// 获取当前 locale 对应的翻译字典
fn get_dict(locale: &str) -> &'static HashMap<&'static str, &'static str> {
    match locale {
        "zh-CN" => zh_cn(),
        _ => en_us(),
    }
}

/// 翻译 action 占位符
fn resolve_action(text: &str, action_val: &str, dict: &HashMap<&'static str, &'static str>) -> String {
    let ak = action_to_key(action_val);
    let translated_action = dict.get(ak).unwrap_or(&action_val).to_string();
    text.replace("{action}", &translated_action)
}

/// 将字符串中的 setting.\w+ 引用替换为字典中对应的翻译文本。
///
/// 协议：token = `setting.` + 字母数字/下划线（单段键，不含点）。
/// 与前端 `resolveSettingKeys`（src/i18n/index.ts）的 `setting\.\w+` 规则必须保持一致，
/// 两端同步修改，防止渲染结果漂移；未命中的键原样保留。
fn resolve_setting_keys(text: &str, dict: &HashMap<&'static str, &'static str>) -> String {
    const PREFIX: &str = "setting.";
    let mut result = String::new();
    let mut remaining = text;
    while let Some(pos) = remaining.find(PREFIX) {
        result.push_str(&remaining[..pos]);
        let rest = &remaining[pos..];
        // token = "setting." + 字母数字/下划线：前缀中的点是 token 的一部分，
        // 与前端 `setting\.\w+` 一致（\w 不含点，故键为单段）。
        let tail_start = PREFIX.len();
        let tail_end = rest[tail_start..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map(|i| tail_start + i)
            .unwrap_or(rest.len());
        let token = &rest[..tail_end];
        match dict.get(token) {
            Some(translated) => result.push_str(translated),
            None => result.push_str(token),
        }
        remaining = &rest[tail_end..];
    }
    result.push_str(remaining);
    result
}

/// 翻译 err.* 格式的错误文本（与前端 translateError 保持一致）
fn translate_error_str(raw: &str, dict: &HashMap<&'static str, &'static str>) -> String {
    let msg = raw.trim_start_matches("Error: ");
    if !msg.starts_with("err.") {
        return raw.to_string();
    }
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.is_empty() {
        return raw.to_string();
    }
    let key = parts[0];
    let args = &parts[1..];
    match dict.get(key) {
        Some(template) => {
            let mut text = template.to_string();
            for (i, arg) in args.iter().enumerate() {
                text = text.replace(&format!("{{{}}}", i), arg);
            }
            text
        }
        None => raw.to_string(),
    }
}

/// 根据 key + 结构化 args 渲染翻译文本
///
/// 模拟前端 `tm()` 的行为:
/// 1. 取当前 locale 的翻译模板
/// 2. 如果 args 包含 action，先将其翻译为本地化文本
/// 3. 依次替换 {key} 占位符
/// 4. 替换结果中残余的 setting.\w+ 引用
pub fn render(key: &str, args: &Value, locale: &str) -> String {
    let dict = get_dict(locale);

    // 1. 获取翻译模板
    let template = match dict.get(key) {
        Some(t) => *t,
        None => return key.to_string(),
    };

    let Some(raw_map) = args.as_object() else {
        return template.to_string();
    };

    // 2. 如果有关键字 action，先翻译 action 值
    let mut text = template.to_string();
    if let Some(action_val) = raw_map.get("action").and_then(|v| v.as_str()) {
        text = resolve_action(&text, action_val, dict);
    }

    // 3.0 空 repo 兜底：GitHub 风格模板 `{owner}/{repo}` 在 repo 为空时省略斜杠
    // （YouTube 源 repo 为空，显示为 `频道名` 而非 `频道名/`）
    if raw_map.get("repo").is_some_and(|v| v.as_str().is_some_and(|s| s.is_empty())) {
        text = text.replace("/{repo}", "");
    }

    // 3. 替换其他占位符（所有值类型都转成字符串）
    for (k, v) in raw_map.iter() {
        if k == "action" {
            continue; // 已在上面处理
        }
        let val_str = match v {
            Value::String(s) => {
                // 翻译 err.* 格式的错误文本，使 rendered_message 包含用户可读文本
                if s.starts_with("err.") || s.starts_with("Error: err.") {
                    translate_error_str(s, dict)
                } else {
                    s.clone()
                }
            }
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        };
        text = text.replace(&format!("{{{}}}", k), &val_str);
    }

    // 4. 处理残余的 setting.\w+ 引用
    resolve_setting_keys(&text, dict)
}

/// 向后兼容包装器：接受 JSON 字符串，自动解析后调用 render()
pub fn render_json(key: &str, args_json: &str, locale: &str) -> String {
    let args: Value = serde_json::from_str(args_json).unwrap_or_default();
    render(key, &args, locale)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::*;

    #[test]
    fn test_render_check_auto() {
        let r = render("check.auto", &json!({"owner":"user","repo":"repo","count":"1"}), "zh-CN");
        assert_eq!(r, "检查 user/repo: 1 个新版本");
    }

    #[test]
    fn test_render_check_auto_en() {
        let r = render("check.auto", &json!({"owner":"user","repo":"repo","count":"1"}), "en-US");
        assert_eq!(r, "Check user/repo: 1 new release(s)");
    }

    #[test]
    fn test_render_check_manual() {
        let r = render("check.manual", &json!({"owner":"a","repo":"b","count":"3"}), "zh-CN");
        assert_eq!(r, "[手动] 检查 a/b: 3 个新版本");
    }

    #[test]
    fn test_render_check_manual_all_done() {
        let r = render("check.manual_all_done", &json!({"count":"5"}), "zh-CN");
        assert_eq!(r, "[手动] 全局检查完成, 5 个新版本");
    }

    #[test]
    fn test_render_release_status_changed() {
        let r = render("release.status_changed", &json!({"owner":"user","repo":"repo","tag":"v1.0","id":"123","action":"pending"}), "zh-CN");
        assert_eq!(r, "user/repo v1.0 未读(id=123)");
    }

    #[test]
    fn test_render_release_status_changed_en() {
        let r = render("release.status_changed", &json!({"owner":"user","repo":"repo","tag":"v1.0","id":"123","action":"ignored"}), "en-US");
        assert_eq!(r, "user/repo v1.0 - Ignored (id=123)");
    }

    #[test]
    fn test_render_release_status_changed_unknown() {
        let r = render("release.status_changed_unknown", &json!({"id":"42","action":"snoozed"}), "zh-CN");
        assert_eq!(r, "版本 id=42 状态: 稍后提醒");
    }

    #[test]
    fn test_render_setting_updated() {
        let r = render("setting.updated", &json!({"changes":"setting.poll_interval→60, setting.language→en-US"}), "zh-CN");
        assert_eq!(r, "更新设置: 轮询间隔→60, 界面语言→en-US");
    }

    #[test]
    fn test_render_setting_updated_en() {
        let r = render("setting.updated", &json!({"changes":"setting.poll_interval→60"}), "en-US");
        assert_eq!(r, "Setting updated: Poll Interval→60");
    }

    #[test]
    fn test_render_source_added() {
        let r = render("source.added", &json!({"source_type":"github","owner":"user","repo":"myapp"}), "zh-CN");
        assert_eq!(r, "添加监控源: github user/myapp");
    }

    #[test]
    fn test_render_source_add_failed() {
        let r = render("source.add_failed", &json!({"source_type":"github","owner":"user","repo":"myapp","error":"404 Not Found"}), "zh-CN");
        assert_eq!(r, "添加监控源失败: github user/myapp, 404 Not Found");
    }

    #[test]
    fn test_render_source_add_failed_en() {
        let r = render("source.add_failed", &json!({"source_type":"github","owner":"user","repo":"myapp","error":"404 Not Found"}), "en-US");
        assert_eq!(r, "Failed to add source: github user/myapp, 404 Not Found");
    }

    #[test]
    fn test_render_source_removed() {
        let r = render("source.removed", &json!({"owner":"user","repo":"repo","id":"5"}), "zh-CN");
        assert_eq!(r, "移除监控源 user/repo id=5");
    }

    #[test]
    fn test_render_youtube_source_removed_empty_repo() {
        // YouTube 源 repo 为空：展示频道名，且 `{owner}/{repo}` 模板应省略斜杠
        let r = render("source.removed", &json!({"owner":"Fireship","repo":"","id":"71"}), "zh-CN");
        assert_eq!(r, "移除监控源 Fireship id=71");
    }

    #[test]
    fn test_render_youtube_check_manual_empty_repo() {
        let r = render("check.manual", &json!({"owner":"Fireship","repo":"","count":232}), "zh-CN");
        assert_eq!(r, "[手动] 检查 Fireship: 232 个新版本");
    }

    #[test]
    fn test_render_youtube_status_changed_uses_release_name() {
        let r = render("release.status_changed", &json!({"owner":"Fireship","repo":"","tag":"The Future of Web Dev","id":95900,"action":"ignored"}), "zh-CN");
        assert_eq!(r, "Fireship The Future of Web Dev 已忽略(id=95900)");
    }

    #[test]
    fn test_render_youtube_status_changed_empty_repo_en() {
        let r = render("release.status_changed", &json!({"owner":"Fireship","repo":"","tag":"The Future of Web Dev","id":95900,"action":"ignored"}), "en-US");
        assert_eq!(r, "Fireship The Future of Web Dev - Ignored (id=95900)");
    }

    #[test]
    fn test_render_non_youtube_unchanged_with_empty_repo_field() {
        // GitHub 源即使 repo 字段为空（异常数据），斜杠同样被省略，不产生 `user/` 残废格式
        let r = render("source.removed", &json!({"owner":"user","repo":"","id":"5"}), "zh-CN");
        assert_eq!(r, "移除监控源 user id=5");
    }

    #[test]
    fn test_render_source_log_paused() {
        let r = render("source.log_paused", &json!({"owner":"user","repo":"repo","id":"3"}), "zh-CN");
        assert_eq!(r, "暂停监控源 user/repo id=3");
    }

    #[test]
    fn test_render_source_log_auto_disabled() {
        let r = render("source.log_auto_disabled", &json!({"owner":"user","repo":"repo","id":"3"}), "zh-CN");
        assert_eq!(r, "自动禁用监控源 user/repo id=3（连续失败）");
    }

    #[test]
    fn test_render_backup_exported() {
        let r = render("backup.exported", &json!({"path":"/tmp/backup.json"}), "zh-CN");
        assert_eq!(r, "导出备份到 /tmp/backup.json");
    }

    #[test]
    fn test_render_log_cleared() {
        let r = render("log.cleared", &json!({}), "zh-CN");
        assert_eq!(r, "已清空所有操作日志");
    }

    #[test]
    fn test_render_log_cleared_en() {
        let r = render("log.cleared", &json!({}), "en-US");
        assert_eq!(r, "All operation logs cleared");
    }

    #[test]
    fn test_render_deepseek_key_updated() {
        let r = render("setting.deepseek_key_updated", &json!({}), "zh-CN");
        assert_eq!(r, "已更新 DeepSeek API Key");
    }

    #[test]
    fn test_render_github_token_updated() {
        let r = render("setting.github_token_updated", &json!({}), "en-US");
        assert_eq!(r, "GitHub Token updated");
    }

    #[test]
    fn test_render_youtube_key_updated() {
        let r = render("setting.youtube_key_updated", &json!({}), "zh-CN");
        assert_eq!(r, "已更新 YouTube API Key");
    }

    #[test]
    fn test_render_release_go() {
        let r = render("release.go", &json!({"owner":"user","repo":"repo","tag":"v2.0","id":"10"}), "zh-CN");
        assert_eq!(r, "前往版本 user/repo v2.0 id=10");
    }

    #[test]
    fn test_render_release_ignored() {
        let r = render("release.ignored", &json!({"owner":"user","repo":"repo","tag":"v1.5","id":"7"}), "zh-CN");
        assert_eq!(r, "忽略版本 user/repo v1.5 id=7");
    }

    #[test]
    fn test_render_release_snoozed() {
        let r = render("release.snoozed", &json!({"owner":"user","repo":"repo","tag":"v3.0","id":"15"}), "zh-CN");
        assert_eq!(r, "推迟版本 user/repo v3.0 id=15");
    }

    #[test]
    fn test_render_unknown_key() {
        // 不存在的 key 应返回 key 本身
        let r = render("some.unknown.key", &json!({}), "zh-CN");
        assert_eq!(r, "some.unknown.key");
    }

    #[test]
    fn test_render_invalid_json_args() {
        // 无效 JSON 应返回原始模板（render_json 包装器处理）
        let r = render_json("check.auto", "not-json", "zh-CN");
        assert_eq!(r, "检查 {owner}/{repo}: {count} 个新版本");
    }

    #[test]
    fn test_render_setting_updated_with_unknown_setting_key() {
        // setting.xxx 如果在字典中没有对应翻译，保留原始
        let r = render("setting.updated", &json!({"changes":"setting.unknown_key→value"}), "zh-CN");
        assert_eq!(r, "更新设置: setting.unknown_key→value");
    }

    // ── 整数类型值的测试（与 Rust json!() 宏行为一致）──

    #[test]
    fn test_render_with_integer_count() {
        // Rust 中 json!({"count": 0}) 生成 "count":0（整数）
        let r = render("check.auto", &json!({"owner":"user","repo":"repo","count":0}), "zh-CN");
        assert_eq!(r, "检查 user/repo: 0 个新版本");
    }

    #[test]
    fn test_render_with_integer_count_nonzero() {
        let r = render("check.auto", &json!({"owner":"user","repo":"repo","count":3}), "zh-CN");
        assert_eq!(r, "检查 user/repo: 3 个新版本");
    }

    #[test]
    fn test_render_with_integer_id() {
        // release.go 中 id 是 i64 整数
        let r = render("release.go", &json!({"owner":"user","repo":"repo","tag":"v1.0","id":123}), "zh-CN");
        assert_eq!(r, "前往版本 user/repo v1.0 id=123");
    }

    #[test]
    fn test_render_with_mixed_types() {
        // release.status_changed 中 id 是整数、action 是字符串
        let r = render("release.status_changed", &json!({"owner":"user","repo":"repo","tag":"v2.0","id":456,"action":"pending"}), "zh-CN");
        assert_eq!(r, "user/repo v2.0 未读(id=456)");
    }

    #[test]
    fn test_render_check_auto_integer_from_db() {
        // 真实的数据库数据：count 是整数 0
        let r = render("check.auto", &json!({"count":0,"owner":"Scighost","repo":"Starward"}), "zh-CN");
        assert_eq!(r, "检查 Scighost/Starward: 0 个新版本");
    }

    #[test]
    fn test_render_check_manual_integer_from_db() {
        // 真实的数据库数据：count 是整数 1
        let r = render("check.manual", &json!({"count":1,"owner":"hhelibeb","repo":"relwatch"}), "zh-CN");
        assert_eq!(r, "[手动] 检查 hhelibeb/relwatch: 1 个新版本");
    }

    // ── err.* 错误码翻译测试（与前端 translateError 保持一致）──

    #[test]
    fn test_translate_error_str_repo_not_found() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.repo_not_found", dict), "不存在该仓库");
    }

    #[test]
    fn test_translate_error_str_repo_not_found_en() {
        let dict = en_us();
        assert_eq!(translate_error_str("err.repo_not_found", dict), "Repository not found");
    }

    #[test]
    fn test_translate_error_str_repo_verify_failed() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.repo_verify_failed|API token invalid", dict), "验证仓库失败: API token invalid");
    }

    #[test]
    fn test_translate_error_str_repo_verify_failed_en() {
        let dict = en_us();
        assert_eq!(translate_error_str("err.repo_verify_failed|API token invalid", dict), "Failed to verify repo: API token invalid");
    }

    #[test]
    fn test_translate_error_str_repo_api_error() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.repo_api_error|404", dict), "GitHub API 返回 404");
    }

    #[test]
    fn test_translate_error_str_request_failed() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.request_failed|timeout", dict), "网络请求失败: timeout");
    }

    #[test]
    fn test_translate_error_str_api_error() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.api_error|403|rate limit", dict), "API 请求失败: HTTP 403 rate limit");
    }

    #[test]
    fn test_translate_error_str_api_error_en() {
        let dict = en_us();
        assert_eq!(translate_error_str("err.api_error|403|rate limit", dict), "API request failed: HTTP 403 rate limit");
    }

    #[test]
    fn test_translate_error_str_parse_failed() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.parse_failed|unexpected token", dict), "解析响应失败: unexpected token");
    }

    #[test]
    fn test_translate_error_str_poll_in_progress() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.poll_in_progress", dict), "轮询正在进行中，请稍后再试");
    }

    #[test]
    fn test_translate_error_str_poll_in_progress_en() {
        let dict = en_us();
        assert_eq!(translate_error_str("err.poll_in_progress", dict), "Poll in progress, please try again later");
    }

    #[test]
    fn test_translate_error_str_unsupported_source() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.unsupported_source|gitlab", dict), "不支持的监控源类型: gitlab");
    }

    #[test]
    fn test_translate_error_str_source_not_found() {
        let dict = zh_cn();
        assert_eq!(translate_error_str("err.source_not_found", dict), "监控源不存在");
    }

    #[test]
    fn test_translate_error_str_source_not_found_en() {
        let dict = en_us();
        assert_eq!(translate_error_str("err.source_not_found", dict), "Source not found");
    }

    #[test]
    fn test_translate_error_str_with_error_prefix() {
        let dict = zh_cn();
        // 带 "Error: " 前缀的原始错误消息
        assert_eq!(translate_error_str("Error: err.repo_not_found", dict), "不存在该仓库");
    }

    #[test]
    fn test_translate_error_str_unknown_key() {
        let dict = zh_cn();
        // 不存在的 err.* 键应返回原始字符串
        assert_eq!(translate_error_str("err.definitely_missing_key", dict), "err.definitely_missing_key");
    }

    #[test]
    fn test_translate_error_str_non_err_text() {
        let dict = zh_cn();
        // 非 err.* 文本应返回原始字符串
        assert_eq!(translate_error_str("some random text", dict), "some random text");
    }
}
