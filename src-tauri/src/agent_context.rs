//! 上下文水位：对齐 pi footer 的 `5.2% / 1.0M (auto)`。
//!
//! relwatch 侧不做会话长度治理（依赖 pi 自身管理），但要**按 pi 的口径**把水位
//! 暴露给用户。此前用的是「整个会话字符数 ÷ 2」，与 pi 显示的数字偏差可达 1.7 倍
//! （实测：某会话 relwatch 8.2% vs pi 4.7%）——pi 的水位是「最后一次请求的真实
//! prompt + 其后零碎消息」，不是全历史字符数。
//!
//! 本模块复刻 pi 的两段逻辑（逐条对位，改动前先去看源码）：
//! - 词元：`dist/core/compaction/compaction.js` 的 `estimateProjectedContextTokens`
//!   / `estimateTokens` / `calculateContextTokens`，以及
//!   `dist/core/agent-session.js` 的 `getContextUsage`（压缩后置未知、返回 `?`）；
//! - 窗口与自动压缩：pi 模型目录 `models.json` 的 `contextWindow`、
//!   设置 `settings.json` 的 `compaction.enabled`（footer 的 `(auto)` 标记）。
//!
//! 全量数据都来自**磁盘文件**（会话 JSONL + pi 配置），不依赖 RPC 进程当前加载
//! 的是哪个会话，也不改动 pi 的任何状态。

use serde_json::Value;

use crate::agent_rpc::pi_agent_dir;

/// 图片块的字符当量（pi `ESTIMATED_IMAGE_CHARS`，compaction.js）。
const ESTIMATED_IMAGE_CHARS: i64 = 4800;

/// 会话上下文水位的词元侧结果（不含窗口，窗口由 `model_context_window` 查表填充）。
#[derive(Debug, Default, PartialEq)]
pub struct ContextTokens {
    /// 上下文词元。None = 未知：压缩后还没有新一轮模型响应，
    /// 此时 pi footer 显示 `?/1.0M`（旧 usage 反映的是压缩前的上下文大小）。
    pub tokens: Option<i64>,
    /// `tokens` 是否为 chars/4 估算（会话里没有可信 usage 时的回落口径）。
    pub estimated: bool,
    /// 会话最后使用的模型（provider + modelId，取自最后一条 `model_change`）。
    /// 仅用于查 `contextWindow`，不参与词元计算。
    pub provider: Option<String>,
    pub model_id: Option<String>,
}

/// 扫描会话文件内容，算出上下文词元（pi footer 同口径）。
///
/// 规则（与 pi `getContextUsage` 逐条对应）：
/// 1. **最后一条有效 assistant usage** 是基准：`stopReason` 非 aborted/error 且
///    `totalTokens`（缺失时按 input+output+cacheRead+cacheWrite）> 0；
/// 2. 基准之后的消息按 chars/4 估算后加上——它们已经在 prompt 里，但不在那次
///    usage 的计数范围内；
/// 3. 基准之前发生过 compaction（或 base 缺失但压缩过）→ **水位未知**：
///    pi 认为压缩前的 usage 已经反映不了当前上下文；
/// 4. 全会话没有有效 usage（新会话 / pi 未上报）→ 全量 chars/4 估算，
///    `estimated = true`（前端据此标 `≈`，不把估算值当精确值展示）。
pub fn context_tokens(content: &str) -> ContextTokens {
    let mut out = ContextTokens::default();
    // 全量 chars/4 估算（无有效 usage 时的回落）
    let mut all_tokens = 0i64;
    // 最后一条有效 usage 之后的消息估算
    let mut trailing = 0i64;
    let mut base: Option<i64> = None;
    let mut base_index: Option<usize> = None;
    let mut last_compaction: Option<usize> = None;
    let mut last_invalidating: Option<usize> = None;
    // 条目序号：含非 message 条目，保证 compression/usage 的先后比较与文件顺序一致
    let mut index = 0usize;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 坏行容忍（与 parse_session_jsonl 同口径）：写入中的半行跳过，
        // 不让一条残行让整段水位失效
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let kind = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "model_change" => {
                let provider = value.get("provider").and_then(|v| v.as_str());
                let model_id = value.get("modelId").and_then(|v| v.as_str());
                if let (Some(p), Some(m)) = (provider, model_id) {
                    out.provider = Some(p.to_string());
                    out.model_id = Some(m.to_string());
                }
            }
            // context_edit（扩展改写上下文）与 compaction 一样会让旧 usage 失效
            "compaction" | "context_edit" => {
                last_invalidating = Some(index);
                if kind == "compaction" {
                    last_compaction = Some(index);
                }
            }
            "message" => {
                if let Some(msg) = value.get("message") {
                    let estimate = estimate_message_tokens(msg);
                    all_tokens += estimate;
                    trailing += estimate;
                    if let Some(tokens) = assistant_context_tokens(msg) {
                        base = Some(tokens);
                        base_index = Some(index);
                        // 本条 usage 自身不计入 trailing（pi 只统计基准之后的消息）
                        trailing = 0;
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }

    // 压缩后尚未有新一轮响应：旧 usage 反映的是压缩前的上下文 → 未知
    let compacted_without_usage = match (last_compaction, base_index) {
        (Some(compaction), Some(usage)) => usage < compaction,
        (Some(_), None) => true,
        _ => false,
    };
    if compacted_without_usage {
        out.tokens = None;
        out.estimated = false;
        return out;
    }

    match base {
        // usage 晚于最近一次压缩/上下文改写 → usage 仍然可信
        Some(tokens) if last_invalidating.map_or(true, |i| base_index.unwrap_or(0) > i) => {
            out.tokens = Some(tokens + trailing);
            out.estimated = false;
        }
        // 其余情况（含无 usage）→ 全量 chars/4 估算
        _ => {
            out.tokens = Some(all_tokens);
            out.estimated = true;
        }
    }
    out
}

/// 单条消息的词元估算（pi `estimateTokens`，compaction.js）。
///
/// 逐 role 对位：system 取 content + sections + toolsAdded；assistant 取
/// text/thinking/toolCall（工具参数按 JSON 序列化计）；user/toolResult/custom 取
/// content 文本块（图片按 4800 字符）；bash 取 command + output；摘要类取 summary。
///
/// 字符数按 **Unicode 码点** 计（pi 用 JS 的 `.length`，即 UTF-16 码元）：仅对
/// 星平面字符（emoji 等）有 ±1 量级差异，且只影响估算路径，不追求逐字一致。
fn estimate_message_tokens(msg: &Value) -> i64 {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let chars = match role {
        "system" => {
            let mut chars = content_chars(msg.get("content"));
            if let Some(sections) = msg.get("sections").and_then(|s| s.as_object()) {
                for section in sections.values() {
                    chars += match section {
                        Value::String(s) => text_len(s),
                        other => compact_json_len(other),
                    };
                }
            }
            if let Some(tools) = msg.get("toolsAdded") {
                chars += compact_json_len(tools);
            }
            chars
        }
        "assistant" => {
            let mut chars = 0i64;
            if let Some(items) = msg.get("content").and_then(|c| c.as_array()) {
                for block in items {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => chars += str_chars(block.get("text")),
                        Some("thinking") => chars += str_chars(block.get("thinking")),
                        Some("toolCall") => {
                            chars += str_chars(block.get("name"));
                            if let Some(args) = block.get("arguments") {
                                chars += compact_json_len(args);
                            }
                        }
                        _ => {}
                    }
                }
            }
            chars
        }
        "bashExecution" | "bash" => str_chars(msg.get("command")) + str_chars(msg.get("output")),
        "branchSummary" | "compactionSummary" => str_chars(msg.get("summary")),
        // user / toolResult / custom：content 文本块 + 图片当量
        _ => content_chars(msg.get("content")),
    };
    // pi 对每条消息单独 ceil(chars/4) 再求和
    (chars + 3) / 4
}

/// pi `estimateTextAndImageContentChars`：字符串取长度；块数组取 text 块字符数
/// 与图片块当量。
fn content_chars(content: Option<&Value>) -> i64 {
    match content {
        Some(Value::String(s)) => text_len(s),
        Some(Value::Array(items)) => items
            .iter()
            .map(|block| match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => str_chars(block.get("text")),
                Some("image") => ESTIMATED_IMAGE_CHARS,
                _ => 0,
            })
            .sum(),
        _ => 0,
    }
}

fn str_chars(value: Option<&Value>) -> i64 {
    value
        .and_then(|v| v.as_str())
        .map(text_len)
        .unwrap_or(0)
}

fn text_len(s: &str) -> i64 {
    s.chars().count() as i64
}

/// JSON 序列化后的字符数（pi 用 `JSON.stringify(...).length`）。
fn compact_json_len(value: &Value) -> i64 {
    value.to_string().chars().count() as i64
}

/// 一条 assistant 消息的上下文词元：pi `calculateContextTokens(usage)`。
///
/// 跳过 aborted / error 的响应与 tokens 为 0 的 usage——它们不代表一次成功的
/// 上下文发送，拿它当水位会让数字虚低（pi `getAssistantUsage` 同一口径）。
fn assistant_context_tokens(msg: &Value) -> Option<i64> {
    if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }
    if matches!(
        msg.get("stopReason").and_then(|r| r.as_str()),
        Some("aborted") | Some("error")
    ) {
        return None;
    }
    let usage = msg.get("usage")?;
    let native = usage.get("totalTokens").and_then(|v| v.as_i64());
    let tokens = match native {
        Some(t) if t > 0 => t,
        _ => ["input", "output", "cacheRead", "cacheWrite"]
            .iter()
            .map(|k| usage.get(k).and_then(|v| v.as_i64()).unwrap_or(0))
            .sum(),
    };
    (tokens > 0).then_some(tokens)
}

// ---- pi 配置读取（模型窗口 / 自动压缩）----

/// 当前模型的上下文窗口（pi models.json 的 `contextWindow`，如 1000000）。
///
/// 查表键是会话最后一条 `model_change` 的 provider + modelId。provider 精确命中
/// 优先；provider 查不到时退回按 model id 匹配（同名模型挂在别家 provider 下）。
/// `modelOverrides` 的 `contextWindow` 覆盖模型定义（pi 的合并顺序）。
pub fn model_context_window(provider: &str, model_id: &str) -> Option<i64> {
    let text = std::fs::read_to_string(pi_agent_dir()?.join("models.json")).ok()?;
    parse_context_window(&text, provider, model_id)
}

/// 从 models.json 文本查 `contextWindow`（纯函数，便于测试）。
fn parse_context_window(text: &str, provider: &str, model_id: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(text).ok()?;
    let providers = value.get("providers")?.as_object()?;
    let mut fallback: Option<i64> = None;
    for (name, config) in providers {
        let window = provider_model_window(config, model_id);
        let Some(window) = window else { continue };
        if name == provider {
            return Some(window);
        }
        fallback.get_or_insert(window);
    }
    fallback
}

/// 单个 provider 配置里某模型的 `contextWindow`（定义字段 + `modelOverrides` 覆盖）。
fn provider_model_window(config: &Value, model_id: &str) -> Option<i64> {
    let defined = config
        .get("models")
        .and_then(|m| m.as_array())
        .and_then(|models| {
            models
                .iter()
                .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(model_id))
        })
        .and_then(|m| m.get("contextWindow"))
        .and_then(|v| v.as_i64());
    let overridden = config
        .get("modelOverrides")
        .and_then(|o| o.as_object())
        .and_then(|o| o.get(model_id))
        .and_then(|m| m.get("contextWindow"))
        .and_then(|v| v.as_i64());
    overridden.or(defined)
}

/// pi 是否开启自动压缩（settings.json `compaction.enabled`，缺省 true）。
///
/// 对应 pi footer 的 `(auto)` 标记：开着时 pi 会在逼近窗口上限前自动摘要历史，
/// 用户看到的 `5.2%` 才有「不会突然爆上下文」的保证。
pub fn auto_compaction_enabled() -> bool {
    let Some(dir) = pi_agent_dir() else {
        return true;
    };
    match std::fs::read_to_string(dir.join("settings.json")) {
        Ok(text) => parse_auto_compaction(&text),
        Err(_) => true,
    }
}

/// 从 settings.json 文本读 `compaction.enabled`（纯函数；缺失/坏文件 → pi 默认 true）。
fn parse_auto_compaction(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            v.pointer("/compaction/enabled")
                .and_then(|b| b.as_bool())
        })
        .unwrap_or(true)
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造一条 message 条目（拼 JSON 字符串，形如会话文件的一行）。
    fn message_entry(message: Value) -> String {
        json!({ "type": "message", "id": "m", "parentId": null, "message": message }).to_string()
    }

    fn assistant_with_usage(usage: Value) -> String {
        message_entry(json!({
            "role": "assistant",
            "content": [{ "type": "text", "text": "回复" }],
            "usage": usage,
        }))
    }

    fn usage(input: i64, output: i64, cache_read: i64) -> Value {
        json!({
            "input": input, "output": output, "cacheRead": cache_read, "cacheWrite": 0,
            "totalTokens": input + output + cache_read,
            "cost": { "total": 0.001 },
        })
    }

    #[test]
    fn estimate_tokens_matches_pi_roles() {
        // user 字符串："问题" 2 字 → ceil(2/4) = 1
        assert_eq!(
            estimate_message_tokens(&json!({ "role": "user", "content": "问题" })),
            1
        );
        // user 块数组 + 图片：4 字 + 4800 → ceil(4804/4)
        assert_eq!(
            estimate_message_tokens(&json!({
                "role": "user",
                "content": [{ "type": "text", "text": "截图如下" }, { "type": "image", "data": "x" }],
            })),
            (4 + ESTIMATED_IMAGE_CHARS + 3) / 4
        );
        // assistant：text + thinking + toolCall（名字 + 参数字符数）
        let assistant = json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "答案" },
                { "type": "thinking", "thinking": "想一想" },
                { "type": "toolCall", "name": "bash", "arguments": { "cmd": "ls" } },
            ],
        });
        // "答案"(2) + "想一想"(3) + "bash"(4) + {"cmd":"ls"}(12) = 21 → ceil(21/4) = 6
        assert_eq!(estimate_message_tokens(&assistant), 6);
        // toolResult：content 文本块（"file1\nfile2\n" = 12 字符 → 3）
        assert_eq!(
            estimate_message_tokens(&json!({
                "role": "toolResult",
                "content": [{ "type": "text", "text": "file1\nfile2\n" }],
            })),
            (12 + 3) / 4
        );
        // bashExecution：command + output
        assert_eq!(
            estimate_message_tokens(&json!({
                "role": "bashExecution", "command": "ls", "output": "a\nb\n",
            })),
            (6 + 3) / 4
        );
        // 摘要类：summary
        assert_eq!(
            estimate_message_tokens(&json!({ "role": "compactionSummary", "summary": "摘要内容" })),
            1
        );
        // system：content + sections + toolsAdded
        let system = json!({
            "role": "system",
            "content": "",
            "sections": { "preamble": "abcde" },
            "toolsAdded": [{ "name": "read" }],
        });
        let expected = (5 + "[{\"name\":\"read\"}]".len() as i64 + 3) / 4;
        assert_eq!(estimate_message_tokens(&system), expected);
    }

    #[test]
    fn context_tokens_uses_last_valid_usage_plus_trailing() {
        let content = [
            json!({ "type": "session", "version": 3 }).to_string(),
            json!({ "type": "model_change", "provider": "deepseek", "modelId": "deepseek-v4-flash" })
                .to_string(),
            message_entry(json!({ "role": "user", "content": "第一轮" })),
            assistant_with_usage(usage(100, 20, 0)),
            // 基准之后的消息：已在 prompt 里但不在那次 usage 里 → 按 chars/4 补
            message_entry(json!({ "role": "toolResult", "content": [{ "type": "text", "text": "12345678" }] })),
        ]
        .join("\n");

        let ctx = context_tokens(&content);
        assert_eq!(ctx.tokens, Some(122)); // 120 + ceil(8/4)
        assert!(!ctx.estimated);
        assert_eq!(ctx.provider.as_deref(), Some("deepseek"));
        assert_eq!(ctx.model_id.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn context_tokens_prefers_latest_usage_and_skips_aborted() {
        let content = [
            assistant_with_usage(usage(1000, 100, 0)),
            // 中止的响应不算数（pi getAssistantUsage 同口径）
            message_entry(json!({
                "role": "assistant", "content": [], "stopReason": "aborted",
                "usage": usage(1, 1, 0),
            })),
            // error 同理
            message_entry(json!({
                "role": "assistant", "content": [], "stopReason": "error",
                "usage": usage(1, 1, 0),
            })),
            message_entry(json!({ "role": "user", "content": "继续" })),
        ]
        .join("\n");
        let ctx = context_tokens(&content);
        // 基准仍是第一条（1100），其后一条 user 消息（ceil(2/4)=1）
        assert_eq!(ctx.tokens, Some(1101));
        assert!(!ctx.estimated);
    }

    #[test]
    fn context_tokens_falls_back_to_components_when_total_missing() {
        let content = message_entry(json!({
            "role": "assistant",
            "content": [],
            "usage": { "input": 10, "output": 20, "cacheRead": 30, "cacheWrite": 40 },
        }));
        assert_eq!(context_tokens(&content).tokens, Some(100));
    }

    #[test]
    fn context_tokens_is_unknown_after_compaction_without_new_response() {
        let content = [
            assistant_with_usage(usage(900_000, 1000, 0)),
            json!({ "type": "compaction", "id": "c1", "summary": "历史摘要" }).to_string(),
        ]
        .join("\n");
        let ctx = context_tokens(&content);
        // 压缩后没有新一轮响应：旧 usage 反映的是压缩前的上下文 → 未知（pi 显示 `?`）
        assert_eq!(ctx.tokens, None);
        assert!(!ctx.estimated);
    }

    #[test]
    fn context_tokens_trusts_usage_after_compaction() {
        let content = [
            assistant_with_usage(usage(900_000, 1000, 0)),
            json!({ "type": "compaction", "id": "c1", "summary": "历史摘要" }).to_string(),
            assistant_with_usage(usage(12_000, 300, 0)),
        ]
        .join("\n");
        let ctx = context_tokens(&content);
        assert_eq!(ctx.tokens, Some(12_300));
        assert!(!ctx.estimated);
    }

    #[test]
    fn context_tokens_estimates_whole_session_without_usage() {
        let content = [
            message_entry(json!({ "role": "user", "content": "12345678" })),
            message_entry(json!({ "role": "assistant", "content": [{ "type": "text", "text": "1234" }] })),
        ]
        .join("\n");
        let ctx = context_tokens(&content);
        assert_eq!(ctx.tokens, Some(3)); // ceil(8/4) + ceil(4/4)
        assert!(ctx.estimated);
        assert_eq!(ctx.provider, None);
    }

    #[test]
    fn context_tokens_tolerates_partial_and_broken_lines() {
        let content = [
            message_entry(json!({ "role": "user", "content": "12345678" })),
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"半".to_string(),
            String::new(),
        ]
        .join("\n");
        let ctx = context_tokens(&content);
        assert_eq!(ctx.tokens, Some(2));
        assert!(ctx.estimated);
    }

    #[test]
    fn parse_context_window_matches_provider_then_model_id() {
        let text = json!({
            "providers": {
                "commandcode": { "models": [{ "id": "deepseek/deepseek-v4-flash", "contextWindow": 1_000_000 }] },
                "zai": { "models": [{ "id": "glm-5.3-flash", "contextWindow": 1_048_576 }] },
                "empty": { "models": [{ "id": "no-window" }] },
            }
        })
        .to_string();
        // provider + id 精确命中
        assert_eq!(
            parse_context_window(&text, "commandcode", "deepseek/deepseek-v4-flash"),
            Some(1_000_000)
        );
        // provider 查不到 → 按 model id 退回（同名模型挂在别家 provider 下）
        assert_eq!(
            parse_context_window(&text, "unknown-provider", "glm-5.3-flash"),
            Some(1_048_576)
        );
        // 模型没有 contextWindow / 完全查不到 → None（前端不显示百分比）
        assert_eq!(parse_context_window(&text, "empty", "no-window"), None);
        assert_eq!(parse_context_window(&text, "commandcode", "nope"), None);
        assert_eq!(parse_context_window("not json", "a", "b"), None);
    }

    #[test]
    fn parse_context_window_prefers_model_overrides() {
        let text = json!({
            "providers": {
                "p": {
                    "models": [{ "id": "m", "contextWindow": 128_000 }],
                    "modelOverrides": { "m": { "contextWindow": 200_000 } },
                }
            }
        })
        .to_string();
        assert_eq!(parse_context_window(&text, "p", "m"), Some(200_000));
        // 只有 overrides、没有 models 定义时同样可用
        let only_overrides = json!({
            "providers": { "p": { "modelOverrides": { "m": { "contextWindow": 64_000 } } } }
        })
        .to_string();
        assert_eq!(parse_context_window(&only_overrides, "p", "m"), Some(64_000));
    }

    #[test]
    fn parse_auto_compaction_defaults_to_true() {
        // pi：settings.compaction?.enabled ?? true
        assert!(parse_auto_compaction("{}"));
        assert!(parse_auto_compaction("not json"));
        assert!(parse_auto_compaction(&json!({ "compaction": {} }).to_string()));
        assert!(!parse_auto_compaction(&json!({ "compaction": { "enabled": false } }).to_string()));
        assert!(parse_auto_compaction(&json!({ "compaction": { "enabled": true } }).to_string()));
    }
}


