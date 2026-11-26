use crate::db::ai_usage::CallUsage;
use crate::db::settings::{
    DEEPSEEK_PROMPT_FIXED_SUFFIX, DEFAULT_DEEPSEEK_PROMPT_EDITABLE,
    DEFAULT_DEEPSEEK_TRANSLATE_PROMPT,
};
use super::transport::{
    chat_completion, chat_completion_stream, count_prompt_chars, disable_thinking,
    DEEPSEEK_TRANSLATE_MAX_TOKENS, LOCAL_CODE_BASE, LOCAL_CODE_EMPTY_CONTENT,
};

/// 把 chat/completions 调用链的 `(status, msg)` 错误映射为展示串。
/// 三个 call_*（摘要/语言检测/翻译）共用同一展示格式（`status` 重复一次
/// 属既定格式，勿改）；本地码（见 `LOCAL_CODE_BASE`）不带 HTTP 前缀。
pub(super) fn format_chat_error(status: u16, msg: &str) -> String {
    if status >= LOCAL_CODE_BASE {
        // 本地码不对应任何 HTTP 状态，套 HTTP 前缀只会误导
        msg.to_string()
    } else if status > 0 {
        format!("[{}] AI API 返回错误 {}: {}", status, status, msg)
    } else {
        msg.to_string()
    }
}

/// 重试判定（三个 call_* 共用）：区分「值得重试的瞬时故障」与「重试也无用的确定性错误」。
///
/// 可重试：
/// - `429` 限流（退避后通常能过）；
/// - `status == 0`：本地约定的「未拿到 HTTP 响应」标记（见 `chat_completion` 的
///   `map_err`），涵盖连接超时、连接被重置、DNS 失败，以及模型返回空内容——
///   都是瞬时故障，不能一次即判死。
/// - `520` / `524` 网关上游故障。Cloudflare 52x 系的语义是「网关与上游之间的
///   连接层故障」，而非上游应用自身回应的 5xx；网关对同一故障会随机返回 520 或
///   524，只白名单 524 会把撞上 520 的那次长等待白白作废。网关明确在请调用方
///   重试，且此时请求往往已被上游接收但结果作废，不重试则白费一次长等待
///   （翻译批长达 300s）。
///
/// 不重试：其余 HTTP 状态码（400 参数错、401/403 鉴权错、404，以及 500/502/503/
/// 504 等）——上游过载时退避重试会雪上加霜，且这类错误通常需要人工介入而非自动重试。
pub(super) fn is_retryable(e: &(u16, String)) -> bool {
    if e.0 == 429 {
        log::warn!("DeepSeek 限流(429), 将重试");
        return true;
    }
    if e.0 == 520 || e.0 == 524 {
        log::warn!("DeepSeek 网关上游故障({}), 将重试", e.0);
        return true;
    }
    if e.0 == 0 {
        log::warn!("DeepSeek 请求未拿到响应或返回空内容({}), 将重试", e.1);
        return true;
    }
    false
}

pub(super) async fn call_summary(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    prompt_template: &str,
    body_text: &str,
) -> Result<((String, String), Vec<CallUsage>), (String, Vec<CallUsage>)> {
    // 组装完整提示词：可编辑部分 + 固定 JSON 格式约束
    let editable = if prompt_template.is_empty() {
        DEFAULT_DEEPSEEK_PROMPT_EDITABLE.to_string()
    } else {
        prompt_template.to_string()
    };
    let full_prompt = format!("{}\n\n{}", editable, DEEPSEEK_PROMPT_FIXED_SUFFIX);
    let prompt = full_prompt.replace("{}", body_text);
    let body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.3,
        "max_tokens": 800
        // 注：不传 response_format，以保证对不支持的 OpenAI 兼容供应商可用；
        // JSON 格式已由 DEEPSEEK_PROMPT_FIXED_SUFFIX 在提示词中强制约束。
    });
    // 每次成功 HTTP 响应都记一条用量（响应后 JSON 解析失败触发重试时，
    // 已消耗的那次也要计入）；连接级失败（无响应）无 usage 可记，不计。
    // FnMut 闭包返回的 future 不能借用闭包捕获（&mut 逃逸），用 Arc<Mutex> 收集。
    // 错误分支同样带回 usages：重试耗尽后的已消耗调用不能凭空丢失。
    let usages: std::sync::Arc<std::sync::Mutex<Vec<CallUsage>>> = Default::default();

    let outcome = crate::retry::retry_with_backoff(
        &crate::retry::RetryConfig::default(),
        is_retryable,
        || async {
            let outcome = chat_completion(client, base_url, &body_json).await?;
            usages.lock().unwrap().push(CallUsage::from_outcome(
                "summary",
                outcome.usage,
                &outcome.content,
                count_prompt_chars(&body_json),
                outcome.duration_ms,
            ));
            let parsed: serde_json::Value = serde_json::from_str(&outcome.content)
                .map_err(|e| (0, format!("解析摘要 JSON 失败: {} — 原始内容: {}", e, outcome.content)))?;
            let summary = parsed["summary"].as_str().unwrap_or("").to_string();
            let importance = parsed["importance"].as_str().unwrap_or("中").to_string();
            if summary.is_empty() {
                return Err((0, "摘要为空".to_string()));
            }
            Ok((summary, importance))
        },
    )
    .await;
    // 独立语句克隆走收集结果，MutexGuard 借用在此结束
    let collected: Vec<CallUsage> = usages.lock().unwrap().clone();
    match outcome {
        Ok(result) => Ok((result, collected)),
        Err((status, msg)) => Err((format_chat_error(status, &msg), collected)),
    }
}

/// 调用 AI 检测文本主体语言。仅取 body 前 500 字符以节省 token。
/// 返回 AI 判断的语言名称（如 "中文"/"English"/"日本語"）。
/// 用于翻译前判断是否需要翻译：若检测语言 == 目标语言则跳过。
pub(super) async fn call_detect_language(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    text_sample: &str,
) -> Result<(String, Vec<CallUsage>), (String, Vec<CallUsage>)> {
    let prompt = format!(
        "请判断以下文本的主体语言，仅用一个词回答语言名称（如 中文、English、日本語、Français 等），不要输出其他任何内容。\n\n文本：\n{}",
        text_sample
    );
    let mut body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.0,
        "max_tokens": 20
    });
    // 必须关掉思考：默认 effort=high 的思维链会把 20 token 预算吃光，
    // `content` 返回空（之前的空结果重试就是被它触发的）
    disable_thinking(&mut body_json);
    let usages: std::sync::Arc<std::sync::Mutex<Vec<CallUsage>>> = Default::default();
    let outcome = crate::retry::retry_with_backoff(
        &crate::retry::RetryConfig::default(),
        is_retryable,
        || async {
            let outcome = chat_completion(client, base_url, &body_json).await?;
            usages.lock().unwrap().push(CallUsage::from_outcome(
                "detect_language",
                outcome.usage,
                &outcome.content,
                count_prompt_chars(&body_json),
                outcome.duration_ms,
            ));
            if outcome.content.is_empty() {
                // 用 LOCAL_CODE_EMPTY_CONTENT 上报：空结果不重试（见常量说明），
                // 而 429/网络类瞬时故障照旧重试
                return Err((LOCAL_CODE_EMPTY_CONTENT, "语言检测结果为空".to_string()));
            }
            Ok(outcome.content)
        },
    )
    .await;
    let collected: Vec<CallUsage> = usages.lock().unwrap().clone();
    match outcome {
        Ok(result) => Ok((result, collected)),
        Err((status, msg)) => Err((format_chat_error(status, &msg), collected)),
    }
}

/// 调用 AI 翻译 release note 全文。纯文本输出（非 JSON），
/// 保留 Markdown 结构。`target_lang` 为目标语言（如 "中文"/"English"）。
pub(super) async fn call_translate(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    target_lang: &str,
    body_text: &str,
) -> Result<(String, Vec<CallUsage>), (String, Vec<CallUsage>)> {
    call_translate_impl(client, model, base_url, target_lang, body_text, None).await
}

/// 流式翻译：分片边到边经 `sink` 投递，返回值与非流式完全相同。
pub(super) async fn call_translate_streaming(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    target_lang: &str,
    body_text: &str,
    sink: &(dyn Fn(&str) + Send + Sync),
) -> Result<(String, Vec<CallUsage>), (String, Vec<CallUsage>)> {
    call_translate_impl(client, model, base_url, target_lang, body_text, Some(sink)).await
}

/// 翻译调用主体：`sink` 为 None 走非流式（自动批与卡片入口），Some 走流式
/// （用户在弹窗里点「翻译」）。
///
/// 两条路径的请求体、重试、用量收集、空结果判定完全共用——差异只在单次请求
/// 用 `chat_completion` 还是 `chat_completion_stream`。
pub(super) async fn call_translate_impl(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    target_lang: &str,
    body_text: &str,
    sink: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<(String, Vec<CallUsage>), (String, Vec<CallUsage>)> {
    let prompt = DEFAULT_DEEPSEEK_TRANSLATE_PROMPT
        .replace("{lang}", target_lang)
        .replace("{}", body_text);
    let mut body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.3,
        "max_tokens": DEEPSEEK_TRANSLATE_MAX_TOKENS
    });
    // 翻译是确定性任务，思考没有收益（见 disable_thinking）
    disable_thinking(&mut body_json);
    let usages: std::sync::Arc<std::sync::Mutex<Vec<CallUsage>>> = Default::default();
    // 是否已向前端投递过分片。重试只对「还没吐出任何内容」的失败有意义：
    // 已开流的尝试重试会从零重新生成，与已投递的分片在用户眼前叠成两份文本，
    // 且上游已为作废的那轮计费。
    let emitted = std::sync::atomic::AtomicBool::new(false);
    let outcome = crate::retry::retry_with_backoff(
        &crate::retry::RetryConfig::default(),
        |e| !emitted.load(std::sync::atomic::Ordering::SeqCst) && is_retryable(e),
        || async {
            let outcome = match sink {
                Some(sink) => {
                    let emit = |delta: &str| {
                        emitted.store(true, std::sync::atomic::Ordering::SeqCst);
                        sink(delta);
                    };
                    chat_completion_stream(client, base_url, &body_json, &emit).await?
                }
                None => chat_completion(client, base_url, &body_json).await?,
            };
            usages.lock().unwrap().push(CallUsage::from_outcome(
                "translate",
                outcome.usage,
                &outcome.content,
                count_prompt_chars(&body_json),
                outcome.duration_ms,
            ));
            if outcome.content.is_empty() {
                return Err((0, "翻译结果为空".to_string()));
            }
            Ok(outcome.content)
        },
    )
    .await;
    let collected: Vec<CallUsage> = usages.lock().unwrap().clone();
    match outcome {
        Ok(result) => Ok((result, collected)),
        Err((status, msg)) => Err((format_chat_error(status, &msg), collected)),
    }
}

