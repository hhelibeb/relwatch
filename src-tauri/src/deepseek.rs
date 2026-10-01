use rusqlite::Connection;
use std::error::Error as StdError;

use crate::credential;
use crate::db;
use crate::db::ai_usage::{CallUsage, RawUsage};
use crate::db::settings::{
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_API_KEY,
    KEY_DEEPSEEK_PROXY_BYPASS, KEY_DEEPSEEK_PROMPT, KEY_DEEPSEEK_TRANSLATE_RELEASE, KEY_PROXY_URL,
    KEY_LANGUAGE,
    DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_PROMPT_EDITABLE,
    DEFAULT_DEEPSEEK_TRANSLATE_PROMPT, DEFAULT_DEEPSEEK_TRANSLATE_RELEASE,
    DEEPSEEK_PROMPT_FIXED_SUFFIX,
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
    let proxy_url = db::settings::get_setting(conn, KEY_PROXY_URL)
        .ok()
        .flatten()
        .unwrap_or_default();
    let proxy_mode = db::settings::get_setting(conn, db::settings::KEY_PROXY_MODE)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            if proxy_url.is_empty() { "none".to_string() } else { "custom".to_string() }
        });
    (proxy_url, proxy_mode)
}

/// 把用户填写的 base_url 归一到 chat/completions 的完整 POST 端点。
///
/// 兼容三类填法（OpenAI 兼容生态常见），不能无条件追加 `/v1/chat/completions`：
/// - 根地址（DeepSeek 官方式）：`https://api.deepseek.com` → `.../v1/chat/completions`
/// - 带 /api/v1 前缀（Cline/中转官方式）：`https://api.cline.bot/api/v1` → `.../api/v1/chat/completions`
/// - 完整端点（含 /chat/completions）：`https://host/api/v1/chat/completions` → 原样返回
///
/// 规则：已含 `/chat/completions` 直接用；已含 `/v1` 则补 `/chat/completions`；
/// 否则追加 `/v1/chat/completions`。
pub fn resolve_chat_completion_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/').to_string();
    if base.to_ascii_lowercase().ends_with("/chat/completions") {
        return base;
    }
    if base.to_ascii_lowercase().ends_with("/v1") {
        return format!("{}/chat/completions", base);
    }
    format!("{}/v1/chat/completions", base)
}

/// 从 chat/completions 响应中提取 `content` 文本。
///
/// 兼容两类 JSON 外壳：标准 OpenAI `{"choices":[...]}` 与 Cline/中转常见的
/// `{"data":{"choices":[...]}, "success":true}`。取不下 content 时返回空串。
fn extract_content(json: &serde_json::Value) -> String {
    let choices = json
        .get("choices")
        .or_else(|| json.get("data").and_then(|d| d.get("choices")));
    if let Some(choice) = choices.and_then(|c| c.get(0)) {
        if let Some(content) = choice.pointer("/message/content").and_then(|v| v.as_str()) {
            return content.trim().to_string();
        }
    }
    String::new()
}

/// chat/completions 的成功结果：content + 原始 usage（可能缺失）+ 耗时。
#[derive(Debug)]
pub(crate) struct ChatCompletionOk {
    pub content: String,
    pub usage: Option<RawUsage>,
    pub duration_ms: i64,
}

/// 从响应 JSON 提取 `usage`。优先标准顶层字段；Cline 式中转外壳（
/// `{"data":{"choices":[...]}}`）可能把 usage 包在 `data` 里，两处都试。
/// 任一数值缺失按 0 计（中转可能只回部分字段）。整个 usage 节缺失时返回 None，
/// 由调用方按字符数估算兜底。
fn extract_usage(json: &serde_json::Value) -> Option<RawUsage> {
    let usage = json
        .get("usage")
        .or_else(|| json.get("data").and_then(|d| d.get("usage")))?;
    let get_i64 = |key: &str| usage.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    Some(RawUsage {
        prompt_tokens: get_i64("prompt_tokens"),
        completion_tokens: get_i64("completion_tokens"),
        cache_hit_tokens: get_i64("prompt_cache_hit_tokens"),
        cache_miss_tokens: get_i64("prompt_cache_miss_tokens"),
    })
}

/// 统计请求侧字符数（messages 各条 content 之和），供 usage 缺失时估算。
pub(crate) fn count_prompt_chars(body_json: &serde_json::Value) -> usize {
    body_json
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|msgs| {
            msgs.iter()
                .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                .map(|c| c.chars().count())
                .sum()
        })
        .unwrap_or(0)
}

/// DeepSeek 单次请求的总超时（秒）。reqwest `.timeout()` 覆盖建连→发请求→读完响应体全程，
/// 中转网关往往等到上游生成完才回响应头，故耗时主要由 token 数与网关排队决定。
/// - 翻译（max_tokens=20000 + 正文截 20000 字符）最重，取 300s：网关排队时 60s
///   会间歇超时（表现为笼统的 "error sending request"）。翻译走后台异步批
///   （fire-and-forget，不阻塞轮询/手动检查），长耗时对前端无感，故取值
///   偏向「宁可等完也别中途放弃」——一次成功好过超时后重排下一轮再等一遍。
/// - 摘要（max_tokens=800）多数秒回，但同样受网关排队牵连，取 120s。
/// - 连接测试仍用 60s：只发一小段探测请求，且要快速失败，不让用户在设置页久等。
pub const DEEPSEEK_TIMEOUT_SECS_SUMMARY: u64 = 120;
pub const DEEPSEEK_TIMEOUT_SECS_TRANSLATE: u64 = 300;
pub const DEEPSEEK_TIMEOUT_SECS_TEST: u64 = 60;

/// 流式翻译的空闲读超时（秒）：两次成功读之间的最大间隔。
///
/// 流式路径**不设总超时**（见 `build_stream_client`）：总超时会把「持续在产出、
/// 只是总量大」的长译文中途掐断，而流式下这正是常态。改用空闲读超时。
///
/// 取与非流式总预算同值，是因为它同样要覆盖**首字节之前**的等待：读超时的语义是
/// 「多久没读到数据」，而网关排队、以及无视 `stream: true` 攒完整包再回的缓冲型
/// 中转，都会让首字节晚到很久（见 `DEEPSEEK_TIMEOUT_SECS_TRANSLATE` 的取值理由）。
/// 收紧到 60s 会让这两类请求在服务端还在生成时就被判失败。
pub const DEEPSEEK_STREAM_READ_TIMEOUT_SECS: u64 = 300;

/// 本地错误码号段下限：`(u16, String)` 的 status 位除了 HTTP 状态，还要表达本地
/// 判定的失败（如「上游返回空内容」）。留 600 起的号段给本地码——HTTP 状态码不到这，
/// 两者不会撞号。`format_chat_error` 对号段内的码不套 HTTP 前缀。
const LOCAL_CODE_BASE: u16 = 600;

/// 「上游返回空内容」（语言检测用）。
///
/// 必须**不在 `is_retryable` 的瞬时故障白名单里**（0/429/520/524）：空结果重试是
/// 纯浪费——同一个请求重试结果必然一样（最常见成因是思考模式把 `max_tokens`
/// 预算全吃在思维链上），而检测失败本来就不阻塞翻译（代码按语言不一致照常翻译），
/// 此前为它睡 2+4+8 秒指数退避才开始翻译。
const LOCAL_CODE_EMPTY_CONTENT: u16 = LOCAL_CODE_BASE;

/// 翻译请求的 `max_tokens`：输出侧上限。
///
/// 与输入截断 `DEEPSEEK_TRANSLATE_TRUNCATE_CHARS` 必须保持同量级：中英互译
/// 时输出字符数与输入相当，输出上限若显著小于输入长度，译文会在句子中间被
/// 硬性截断。改动其中一个时请一并核对另一个。
pub const DEEPSEEK_TRANSLATE_MAX_TOKENS: u32 = 20000;
/// 翻译请求的正文截断上限（字符）：输入侧上限，见上常量说明。
pub const DEEPSEEK_TRANSLATE_TRUNCATE_CHARS: usize = 20000;

pub fn build_client(
    api_key: &str,
    proxy_url: &str,
    proxy_mode: &str,
    timeout_secs: u64,
) -> Result<reqwest::Client, String> {
    // DeepSeek 所有请求都打同一 API 域名，token 作为 default header 安全。
    crate::http::build_http_client(crate::http::HttpClientConfig {
        proxy_url,
        proxy_mode,
        bearer_token: Some(api_key),
        timeout_secs,
        content_type_json: true,
        set_default_auth: true,
        ..Default::default()
    })
}

/// 流式专用 client：不设总超时，只约束「多久没有新数据」（见
/// `DEEPSEEK_STREAM_READ_TIMEOUT_SECS`）。
///
/// 不能复用 `build_client`：它设的是总超时（摘要 120s / 翻译 300s），那是为「要么
/// 整包到达要么没有」的非流式请求量的。流式下一旦首片到达就开始产出，掐断的代价是
/// 用户已经看到了半截译文。
pub fn build_stream_client(
    api_key: &str,
    proxy_url: &str,
    proxy_mode: &str,
) -> Result<reqwest::Client, String> {
    crate::http::build_http_client(crate::http::HttpClientConfig {
        proxy_url,
        proxy_mode,
        bearer_token: Some(api_key),
        timeout_secs: 0,
        read_timeout_secs: Some(DEEPSEEK_STREAM_READ_TIMEOUT_SECS),
        content_type_json: true,
        set_default_auth: true,
        ..Default::default()
    })
}

/// relwatch 会往请求体里塞的 DeepSeek 扩展字段（非 OpenAI 标准）。
///
/// 网关不认这些字段时常见反应是 400。它们只影响速度（`thinking`）与统计口径
/// （`stream_options`），不值得让整条任务失败，故 400 时全部去掉重发一次。
const OPTIONAL_BODY_FIELDS: [&str; 2] = ["thinking", "stream_options"];

/// 关闭思考模式：思维链经 `reasoning_content` 返回（与 `content` 同级），
/// 而我们只读 `content`——于是整段思考期间前端一个字都不显示，表现为「首字极慢」。
///
/// 且思考默认开启、effort 默认 high（见 DeepSeek 思考模式文档），翻译/语言检测这类
/// 确定性任务并不需要它：一段 849 字符的正文也能思考出约 2000 token（实测用量表
/// 对得上），这些 token 按输出计价，首字要等约 9 秒。
/// 思考模式还忽略 `temperature`（不报错但不生效），关掉后才能真正生效。
fn disable_thinking(body_json: &mut serde_json::Value) {
    if let Some(obj) = body_json.as_object_mut() {
        obj.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "disabled" }),
        );
    }
}

/// 去掉请求体里的扩展字段（见 `OPTIONAL_BODY_FIELDS`），返回是否确实去掉了。
fn strip_optional_fields(body_json: &mut serde_json::Value) -> bool {
    let Some(obj) = body_json.as_object_mut() else {
        return false;
    };
    let mut removed = false;
    for key in OPTIONAL_BODY_FIELDS {
        removed |= obj.remove(key).is_some();
    }
    removed
}

/// POST 一次 chat/completions。
///
/// 网络层错误（未收到 HTTP 响应）：reqwest 的 Display 只打顶层文案（"error sending
/// request for url (...)"），真正的失败原因（连接超时/连接被重置/DNS 等）藏在 source
/// 链里，故逐层展开 source 拼入（见 `describe_reqwest_error`）。
async fn send_chat_request(
    client: &reqwest::Client,
    endpoint: &str,
    body_json: &serde_json::Value,
) -> Result<reqwest::Response, (u16, String)> {
    client
        .post(endpoint)
        .json(body_json)
        .send()
        .await
        .map_err(|e| (0, format!("请求失败: {}", describe_reqwest_error(&e))))
}

/// 通用 chat/completions 调用：POST → 判 success → 取 content + usage → 错误映射。
/// 三个 call_*（摘要/语言检测/翻译）与连接测试共用此模板，
/// 改超时/重试/错误格式只需动此处一处。
/// 返回 `ChatCompletionOk`（content + usage + 耗时）或 (status, msg)：
/// status>0 时 msg 为 API 原始响应文本；status=0 时 msg 为网络/解析错误描述。
pub(crate) async fn chat_completion(
    client: &reqwest::Client,
    base_url: &str,
    body_json: &serde_json::Value,
) -> Result<ChatCompletionOk, (u16, String)> {
    let endpoint = resolve_chat_completion_url(base_url);
    // 显式要求非流式：部分中转（如 Cline）缺省 stream=true 会返回 SSE 流，
    // 与 relwatch 的 `resp.json()` 解析路径冲突。显式禁止即可规避。
    let mut body = body_json.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.insert("stream".to_string(), serde_json::Value::Bool(false));
    }
    let started = std::time::Instant::now();
    let mut resp = send_chat_request(client, &endpoint, &body).await?;
    // 400/422 多见于「非标准字段不被接受」（不同网关用不同码）：去掉重发一次
    // （见 OPTIONAL_BODY_FIELDS）
    if matches!(resp.status().as_u16(), 400 | 422) && strip_optional_fields(&mut body) {
        resp = send_chat_request(client, &endpoint, &body).await?;
    }
    if resp.status().is_success() {
        // 流式累加 + 超限中断；错误文案保持本模块中文风格，不走
        // read_json_limited 的 err. 前缀格式
        let bytes = crate::http::read_body_limited(resp, crate::http::MAX_JSON_BYTES)
            .await
            .map_err(|e| match e {
                crate::http::BodyReadError::Overflow(n) => {
                    (0, format!("响应体过大 (超过 {} 字节)", n))
                }
                crate::http::BodyReadError::Transport(e) => (0, e),
            })?;
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| (0, format!("解析响应失败: {}", e)))?;
        let duration_ms = started.elapsed().as_millis() as i64;
        return Ok(ChatCompletionOk {
            content: extract_content(&json),
            usage: extract_usage(&json),
            duration_ms,
        });
    }
    Err(read_error_body(resp).await)
}

/// 读非 2xx 响应体生成 `(status, msg)`。
/// 错误体限流到 64KB：错误文本会进日志与 toast，超限时以占位文本替代完整 body。
async fn read_error_body(resp: reqwest::Response) -> (u16, String) {
    let status = resp.status().as_u16();
    const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
    let text = match crate::http::read_body_limited(resp, MAX_ERROR_BODY_BYTES).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(crate::http::BodyReadError::Overflow(_)) => "<body too large>".to_string(),
        Err(crate::http::BodyReadError::Transport(e)) => e,
    };
    (status, text)
}

/// 流式 chat/completions：`stream: true` + SSE 逐行解析，`delta.content` 每片即时
/// 经 `on_delta` 投递；返回值与非流式同形（累计全文 + usage + 耗时），调用方的
/// 落库/统计/错误处理逻辑无需感知差异。
///
/// 三条容错路径：
/// - **形态判定按首字节**：拿到第一批字节后，首个非空白字节是 `{` 就按整包 JSON 处理，
///   否则按 SSE。不能靠 Content-Type——中转可能回 text/plain，而 Nginx 缓冲后
///   又可能回 application/octet-stream。
/// - **网关无视 `stream: true`**：退回整包解析并把全文一次性投递（前端消费路径不变）。
///   请求流式却拿到静默非流式，不能变成失败。
/// - **`stream_options` / `thinking` 被拒**：部分网关对非标准字段回 400/422。
///   统计退化与思考照旧可以接受，整条翻译失败不可以，故去掉这些字段重发一次
///   （统一规则见 `OPTIONAL_BODY_FIELDS`）。
///
/// 截断判定：SSE 走完却从未见到「生成结束」（`[DONE]` 或带 `finish_reason` 的帧）
/// 即判失败。已投递的分片留在前端（调用方据此保留半截译文并提示），但**不落库**。
pub(crate) async fn chat_completion_stream(
    client: &reqwest::Client,
    base_url: &str,
    body_json: &serde_json::Value,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<ChatCompletionOk, (u16, String)> {
    use futures_util::StreamExt;

    let endpoint = resolve_chat_completion_url(base_url);
    let mut body = body_json.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.insert("stream".to_string(), serde_json::Value::Bool(true));
        // 要求末片携带 usage：否则流式路径拿不到 token 统计，只能退化为
        // 字符数估算（estimated=true），与设置页的统计口径不一致。
        obj.insert(
            "stream_options".to_string(),
            serde_json::json!({ "include_usage": true }),
        );
    }
    let started = std::time::Instant::now();
    let mut resp = send_chat_request(client, &endpoint, &body).await?;
    // 400/422 多见于「非标准字段不被接受」（不同网关用不同码）：去掉重发一次
    // （见 OPTIONAL_BODY_FIELDS）。为统计/速度字段丢整条译文不值得，退了仍有内容。
    if matches!(resp.status().as_u16(), 400 | 422) && strip_optional_fields(&mut body) {
        resp = send_chat_request(client, &endpoint, &body).await?;
    }
    if !resp.status().is_success() {
        return Err(read_error_body(resp).await);
    }

    let mut stream = resp.bytes_stream();
    // 未成行的原始字节。行尾换行符是完整性保证：跨 TCP 分片的多字节字符不会被
    // 截成半个 UTF-8 去解析，只有整行到齐才动它。
    let mut buf: Vec<u8> = Vec::new();
    let mut content = String::new();
    let mut usage: Option<RawUsage> = None;
    // None = 还没拿到第一批字节，无从判定形态
    let mut is_sse: Option<bool> = None;
    let mut finished = false;
    // 上游是否明确表示过「生成结束」：[DONE] 或任一帧带 finish_reason。
    // 两者都没有 = 流被截断（含对端直接关连接），半截不能当成品
    let mut terminated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| (0, format!("请求失败: {}", describe_reqwest_error(&e))))?;
        buf.extend_from_slice(&chunk);
        if buf.len() + content.len() > crate::http::MAX_JSON_BYTES {
            return Err((
                0,
                format!("响应体过大 (超过 {} 字节)", crate::http::MAX_JSON_BYTES),
            ));
        }
        if is_sse.is_none() {
            if let Some(first) = buf.iter().find(|b| !b.is_ascii_whitespace()) {
                is_sse = Some(*first != b'{');
            }
        }
        // 整包 JSON（含形态未定的空 chunk）：留到流结束再整体解析
        if is_sse != Some(true) {
            continue;
        }
        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..pos]).trim_end().to_string();
            buf.drain(..=pos);
            let Some(payload) = line.strip_prefix("data:") else {
                continue; // `event:` / 注释 / 空行都无需处理
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                finished = true;
                terminated = true;
                break;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
                // 单行坏数据不终止整条译文：SSE 里可能混入非 JSON 的调试行，
                // 已到达的分片照常交付
                continue;
            };
            if let Some(err) = v.get("error") {
                // 流中错误（限流/上游故障）：已投递的分片留在前端，由调用方按
                // 失败处理（不落库）
                return Err((0, format!("AI API 返回错误: {}", err)));
            }
            if let Some(u) = extract_usage(&v) {
                usage = Some(u);
            }
            // 部分中转只给 finish_reason、不给 [DONE]：两者取「或」判是否收全
            if v.pointer("/choices/0/finish_reason")
                .is_some_and(|r| !r.is_null())
            {
                terminated = true;
            }
            if let Some(delta) = v.pointer("/choices/0/delta/content").and_then(|d| d.as_str()) {
                if !delta.is_empty() {
                    content.push_str(delta);
                    on_delta(delta);
                }
            }
        }
        if finished {
            break;
        }
    }

    let duration_ms = started.elapsed().as_millis() as i64;
    if is_sse == Some(false) {
        // 静默非流式：全文一次性投递，前端消费路径与逐片一致
        let json: serde_json::Value =
            serde_json::from_slice(&buf).map_err(|e| (0, format!("解析响应失败: {}", e)))?;
        let content = extract_content(&json);
        if !content.is_empty() {
            on_delta(&content);
        }
        return Ok(ChatCompletionOk {
            content,
            usage: extract_usage(&json),
            duration_ms,
        });
    }

    // SSE 收完但上游从没说过「生成结束」：流被中途截断（半开连接、代理超时断开等）。
    // 半截译文一旦落库，用户拿到的就是一份看不出破绽的「完整译文」，比报错更坑。
    // 形态未定（空响应体）不走这条：那种情况交给调用方的「结果为空」判定。
    if is_sse == Some(true) && !terminated {
        return Err((0, "响应流中断（未收到生成结束标记），译文不完整".to_string()));
    }

    // 与非流式 `extract_content` 的 trim 口径对齐：两种路径落库内容不能有尾部差异
    Ok(ChatCompletionOk {
        content: content.trim().to_string(),
        usage,
        duration_ms,
    })
}

/// 展开 reqwest 错误链生成可读描述。
///
/// reqwest 错误 Display 只打顶层文案，底层原因（timeout / connect reset / dns /
/// tls）在 source 链里，不展开时日志无法区分「网关超时」与「本机断网」。若
/// source 链里已有可读子错误则附加之；超时类型给出明确前缀，便于日志检索。
fn describe_reqwest_error(e: &reqwest::Error) -> String {
    // reqwest 对请求超时有标准错误分类（is_timeout），优先给出明确语义
    if e.is_timeout() {
        return format!("请求超时: {}", e);
    }
    let mut detail = String::new();
    let mut cur = e.source();
    while let Some(src) = cur {
        let text = src.to_string();
        if !detail.contains(&text) {
            if !detail.is_empty() {
                detail.push_str(" ← ");
            }
            detail.push_str(&text);
        }
        cur = src.source();
    }
    if detail.is_empty() {
        e.to_string()
    } else {
        format!("{} ({})", e, detail)
    }
}

/// 复查：该 release 是否已有 AI 摘要（`run_ai_job` 的 `already_done`，摘要侧）。
fn has_ai_summary(conn: &Connection, release_id: i64) -> bool {
    db::releases::get_release(conn, release_id)
        .ok()
        .flatten()
        .map(|r| r.ai_summary.is_some())
        .unwrap_or(false)
}

/// 复查：该 release 是否已有译文（`run_ai_job` 的 `already_done`，翻译侧）。
fn has_translation(conn: &Connection, release_id: i64) -> bool {
    db::releases::get_release(conn, release_id)
        .ok()
        .flatten()
        .map(|r| r.body_translated.is_some())
        .unwrap_or(false)
}

/// 把 chat/completions 调用链的 `(status, msg)` 错误映射为展示串。
/// 三个 call_*（摘要/语言检测/翻译）共用同一展示格式（`status` 重复一次
/// 属既定格式，勿改）；本地码（见 `LOCAL_CODE_BASE`）不带 HTTP 前缀。
fn format_chat_error(status: u16, msg: &str) -> String {
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
fn is_retryable(e: &(u16, String)) -> bool {
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

async fn call_summary(
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
async fn call_detect_language(
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
async fn call_translate(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    target_lang: &str,
    body_text: &str,
) -> Result<(String, Vec<CallUsage>), (String, Vec<CallUsage>)> {
    call_translate_impl(client, model, base_url, target_lang, body_text, None).await
}

/// 流式翻译：分片边到边经 `sink` 投递，返回值与非流式完全相同。
async fn call_translate_streaming(
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
async fn call_translate_impl(
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

/// 写 AI 任务结果日志：统一"成功/失败 × 有/无 release 行"四种组合。
/// 摘要与翻译共用。
/// - `ok=true`：`{action}已生成: {owner}/{repo} {tag}[ {detail}]`
/// - `ok=false`：`{action}生成失败: {owner}/{repo} {tag}: {detail}`
///
/// 有 release 行时用 owner/repo/tag 定位，否则退化为 `id={release_id}`。
fn log_ai_job_result(
    conn: &Connection,
    level: &str,
    action: &str,
    release_id: i64,
    detail: &str,
    ok: bool,
) {
    let rel = db::releases::get_release(conn, release_id).ok().flatten();
    let who = match rel {
        Some(r) => format!("{}/{} {}", r.owner, r.repo, r.tag_name),
        None => format!("id={}", release_id),
    };
    let msg = if ok {
        if detail.is_empty() {
            format!("{}已生成: {}", action, who)
        } else {
            format!("{}已生成: {} {}", action, who, detail)
        }
    } else {
        format!("{}生成失败: {}: {}", action, who, detail)
    };
    db::logs::write_log(conn, level, &msg);
}

/// 翻译任务的结果：语言检测一致时短路（返回原文）或正常译文。
/// 短路与正常译文在 on_ok 中区分处理（短路写原文 + log::info，译文写译文 + DB 日志）。
enum TranslateOutcome {
    Skipped(String),
    Translated(String),
}

/// 批量注入的流式分片出口：分片带上 `release_id`。
///
/// 批内可能多条同时在出字，只有前端自己知道当前展示的是哪条，故由事件携带 id
/// 让前端过滤，而不是在服务端维护「谁是当前观察者」这种易错状态。
pub type ChunkSink = std::sync::Arc<dyn Fn(i64, &str) + Send + Sync>;

/// 单条任务的流式分片出口（已绑定 release_id）。
pub type DeltaSink = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// AI 任务公共流水线：读配置 → 建 client → 并发调度 → 结果/失败分别落库。
///
/// 摘要与翻译两条流水线共用此骨架，差异点以参数注入：
/// - `job`：控制台日志文案（"摘要"/"译文"）
/// - `truncate_chars`：正文截断上限（摘要 4000 / 翻译见
///   `DEEPSEEK_TRANSLATE_TRUNCATE_CHARS`，须与 `DEEPSEEK_TRANSLATE_MAX_TOKENS` 同量级）
/// - `timeout_secs`：本批请求的总超时（摘要 120 / 翻译 300，见 DEEPSEEK_TIMEOUT_SECS_*）。
///   仅在非流式路径生效：传入 `on_chunk` 时改用流式客户端（空闲读超时），见
///   `build_stream_client`。
/// - `on_chunk`：流式分片出口（None = 非流式）。只有用户点「翻译」的那次单条
///   翻译会传 Some：后台自动批没有观察者，逐片发事件纯属白耗 IPC 与主线程。
/// - `extra_ready`：额外前置开关（翻译的 translate_enabled/force；摘要恒 true）
/// - `call`：AI 调用（翻译侧在闭包内做语言检测短路）
/// - `on_ok` / `on_err`：成功/失败各自的落库与日志动作（在 spawn_blocking 内执行）
///
/// 参数较多：均为两条流水线的真实差异点，刻意集中注入而非隐式复制。
///
/// - `already_done`：调用前复查「这条是不是已经有结果了」，返回 true 则跳过。
///   待办名单是批启动时一次性读取的快照，批内任务并发跑（翻译批可长达 300s），
///   期间同一条可能已被另一路径写入（手动单条翻译、上一批收尾、用户删除重建
///   后重新检查）。不复查会对已完成的条目重复发请求，白白烧 token；更糟的是
///   重复请求若返回空内容，会走 on_err 把本已成功的 retry 计数加 1。
#[allow(clippy::too_many_arguments)]
async fn run_ai_job<T, F, Fut>(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    job: &'static str,
    truncate_chars: usize,
    timeout_secs: u64,
    on_chunk: Option<ChunkSink>,
    extra_ready: impl Fn(&Connection) -> bool,
    already_done: impl Fn(&Connection, i64) -> bool + Send + Sync + 'static,
    call: F,
    on_ok: impl Fn(&Connection, i64, T) + Send + Sync + 'static,
    on_err: impl Fn(&Connection, i64, &str) + Send + Sync + 'static,
) where
    F: Fn(reqwest::Client, String, String, String, String, Option<DeltaSink>) -> Fut
        + Send
        + Sync
        + 'static,
    // call 的返回值携带本次任务的用量明细（可能含语言检测 + 翻译多次调用），
    // 统一由本流水线落 `ai_usage` 表，call_* 与 on_ok 均无需感知统计。
    // 失败分支同样带回用量：重试耗尽前的已消耗调用也要记录。
    Fut: std::future::Future<
            Output = Result<(T, Vec<CallUsage>), (String, Vec<CallUsage>)>,
        > + Send
        + 'static,
    T: Send + 'static,
{
    let (cfg, proxy_url, proxy_mode) = {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("数据库连接失败: {}", e);
                return;
            }
        };
        if !extra_ready(&conn) {
            return;
        }
        let cfg = read_config(&conn);
        let (proxy_url, proxy_mode) = load_ai_network_config(&conn);
        (cfg, proxy_url, proxy_mode)
    };
    if !cfg.enabled {
        return;
    }
    let api_key = match cfg.api_key {
        Some(k) => k,
        None => return,
    };
    let client = match if on_chunk.is_some() {
        // 有分片消费者 = 本次走流式请求：客户端也得换成流式口径
        // （不设总超时，改空闲读超时，见 build_stream_client）
        build_stream_client(&api_key, &proxy_url, &proxy_mode)
    } else {
        build_client(&api_key, &proxy_url, &proxy_mode, timeout_secs)
    } {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("创建 DeepSeek 客户端失败: {}", e);
            log::error!("{}", msg);
            // 同时写 DB：log 插件仅在 debug 构建启用，release 版用户看不到
            // log::error!，只打 console 会让「批静默不发请求」（待办非空、开关
            // 全开、retry 计数 = 0 却无任何可见错误）无法定位。
            // 写入后由调用方（后台批收尾）emit LogAppended 刷新日志 tab。
            if let Ok(conn) = db_pool.get() {
                db::logs::write_log(&conn, "ERROR", &msg);
            }
            return;
        }
    };

    let call = std::sync::Arc::new(call);
    let on_ok = std::sync::Arc::new(on_ok);
    let on_err = std::sync::Arc::new(on_err);
    let already_done = std::sync::Arc::new(already_done);
    let semaphore = deepseek_semaphore.clone();
    let model = cfg.model;
    let base_url = cfg.base_url;
    let prompt = cfg.prompt;
    let mut handles = Vec::new();
    for (release_id, body) in saved {
        let body_text = match body {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        let truncated: String = body_text.chars().take(truncate_chars).collect();
        let client = client.clone();
        let model = model.clone();
        let base_url = base_url.clone();
        let prompt = prompt.clone();
        let db = db_pool.clone();
        let release_id = *release_id;
        let sem_clone = semaphore.clone();
        let call = call.clone();
        let on_ok = on_ok.clone();
        let on_err = on_err.clone();
        let already_done = already_done.clone();
        // 分片出口绑定本条 release：批内可能多条并行出字，前端按 id 只接自己那条
        let on_chunk: Option<DeltaSink> = on_chunk.clone().map(|sink| {
            let bound: DeltaSink = std::sync::Arc::new(move |delta: &str| sink(release_id, delta));
            bound
        });

        handles.push(tokio::spawn(async move {
            let _permit = match sem_clone.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("信号量获取失败: {}", e);
                    return;
                }
            };
            // 抢到信号量后才复查：此时才是真正要发请求的时刻。远离批启动
            // 快照越久（长批尾部的任务），越可能有别的路径已写入结果。
            let db_check = db.clone();
            let done = already_done.clone();
            let skip = tokio::task::spawn_blocking(move || {
                let conn = match db_check.get() {
                    Ok(c) => c,
                    // 复查失败不阻塞：按未处理继续，宁可重复一次也不漏翻
                    Err(e) => {
                        log::error!("数据库连接失败: {}", e);
                        return false;
                    }
                };
                done(&conn, release_id)
            })
            .await
            .unwrap_or(false);
            if skip {
                log::info!(
                    "跳过{} id={}：调用前复查已有结果（可能由其他路径写入）",
                    job,
                    release_id
                );
                return;
            }
            match call(client, model.clone(), base_url, prompt, truncated, on_chunk).await {
                Ok((result, usages)) => {
                    // 同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
                    let _ = tokio::task::spawn_blocking(move || {
                        let conn = match db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("数据库连接失败: {}", e);
                                return;
                            }
                        };
                        // 用量记录先行（独立于业务结果）：落库失败只记日志，
                        // 绝不阻塞/影响摘要与翻译的主流程写入。
                        if !usages.is_empty() {
                            if let Err(e) = db::ai_usage::insert_call_usage(
                                &conn,
                                Some(release_id),
                                &model,
                                &usages,
                            ) {
                                log::warn!("记录 AI token 用量失败 id={}: {}", release_id, e);
                            }
                        }
                        on_ok(&conn, release_id, result);
                    })
                    .await;
                }
                Err((e, usages)) => {
                    log::error!("生成{}失败 id={}: {}", job, release_id, e);
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(conn) = db.get() {
                            // 失败任务里已成功响应过的调用（如重试前的语言检测）也是真实消耗
                            if !usages.is_empty() {
                                if let Err(e) = db::ai_usage::insert_call_usage(
                                    &conn,
                                    Some(release_id),
                                    &model,
                                    &usages,
                                ) {
                                    log::warn!("记录 AI token 用量失败 id={}: {}", release_id, e);
                                }
                            }
                            on_err(&conn, release_id, &e);
                        }
                    })
                    .await;
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }
}

pub async fn generate_summaries_for_new(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
) {
    run_ai_job(
        db_pool,
        deepseek_semaphore,
        saved,
        "摘要",
        4000,
        DEEPSEEK_TIMEOUT_SECS_SUMMARY,
        // 摘要不走流式（无人逐片展示，卡片只显示最终摘要）
        None,
        |_| true,
        has_ai_summary,
        |client, model, base_url, prompt, text, _sink| async move {
            call_summary(&client, &model, &base_url, &prompt, &text).await
        },
        |conn, release_id, (summary, importance)| {
            if let Err(e) = db::releases::set_ai_summary(conn, release_id, &summary, &importance) {
                log::error!("保存摘要失败 id={}: {}", release_id, e);
            } else {
                log_ai_job_result(
                    conn,
                    "INFO",
                    "AI 摘要",
                    release_id,
                    &format!("重要度={}", importance),
                    true,
                );
            }
        },
        |conn, release_id, e| {
            let _ = db::releases::increment_retry_count(conn, release_id);
            log_ai_job_result(conn, "ERROR", "AI 摘要", release_id, e, false);
        },
    )
    .await;
}

/// 为新增的 releases 生成全文翻译。与摘要任务共用 `deepseek_semaphore`，
/// 保证 AI 请求总并发不变。
/// - `force=false`：仅在 `deepseek_translate_release=true` 且已配置 API key 时生效（轮询自动场景）
/// - `force=true`：绕过 `translate_enabled` 开关，只要 AI 已启用且配置 key 即翻译（手动单条场景）
///
/// 无流式出口：这里都是后台/无观察者的批，逐片发事件无人消费（需要流式分片的手动
/// 单条翻译走 `translate_single_release`）。
pub async fn generate_translations_for_new(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    force: bool,
) {
    run_translate_job(db_pool, deepseek_semaphore, saved, force, None).await;
}

/// 用户在弹窗里点「翻译」的单条翻译：force 语义（绕过开关与「已有译文」复查）+
/// 可选流式分片出口。
///
/// 独立入口而非给 `generate_translations_for_new` 多加一个 bool/`Option`
/// 参数：调用点写得出「这是手动单条 + 要流式」的意图，不用去猜裸参含义。
pub async fn translate_single_release(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    release_id: i64,
    body: String,
    on_chunk: Option<ChunkSink>,
) {
    run_translate_job(
        db_pool,
        deepseek_semaphore,
        &[(release_id, Some(body))],
        true,
        on_chunk,
    )
    .await;
}

async fn run_translate_job(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    force: bool,
    on_chunk: Option<ChunkSink>,
) {
    // 目标语言供 call 闭包（语言检测短路）与 on_ok（短路日志）使用，提前读取。
    let target_lang = {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!("数据库连接失败: {}", e);
                return;
            }
        };
        read_translate_config(&conn).1
    };
    let target_lang_log = target_lang.clone();
    run_ai_job(
        db_pool,
        deepseek_semaphore,
        saved,
        "译文",
        // 翻译全文比摘要耗 token 多：截断上限（20000 字符）与 max_tokens（20000）
        // 必须对齐到同一量级——中英互译后输出装不下输入时，长 release note 的
        // 译文会在句子中间被硬截断。
        DEEPSEEK_TRANSLATE_TRUNCATE_CHARS,
        // 翻译超时 300s：max_tokens=20000 的长生成，中转要等上游出完才回
        // 响应头，60s 会在网关排队时超时（流式路径不用它，见 build_stream_client）
        DEEPSEEK_TIMEOUT_SECS_TRANSLATE,
        // 流式出口：仅手动单条翻译注入
        on_chunk,
        // 翻译开关：force 绕过 translate_enabled（手动单条场景）
        move |conn| read_translate_config(conn).0 || force,
        // 复查跳过：仅对自动批（force=false）生效。手动单条翻译（force=true）
        // 是用户显式要求重翻（可能上一版译文不满意），必须放行，不能被
        // 「已有译文」挡住——否则设置页点了翻译却毫无反应、也不报错。
        move |conn, id| !force && has_translation(conn, id),
        move |client, model, base_url, _prompt, text, sink| {
            let target_lang = target_lang.clone();
            async move {
                // 语言检测短路：取 body 前 500 字符让 AI 判断主体语言，
                // 若与目标语言一致则直接把原文返回（由 on_ok 写入 body_translated），
                // 跳过翻译调用。检测失败时不阻塞翻译（视为语言不一致，照常翻译），
                // 但检测侧已产生的用量仍要收集。
                let sample: String = text.chars().take(500).collect();
                let mut usages: Vec<CallUsage> = Vec::new();
                match call_detect_language(&client, &model, &base_url, &sample).await {
                    Ok((detected, mut detect_usages)) => {
                        usages.append(&mut detect_usages);
                        if detected.trim() == target_lang {
                            // 短路没有流式可言（原文即译文）：保持既有行为，
                            // 一次性写库后由 ReleaseStateChanged 驱动前端切视图
                            return Ok((TranslateOutcome::Skipped(text), usages));
                        }
                    }
                    Err((_e, mut detect_usages)) => usages.append(&mut detect_usages),
                }
                // 不能用 `?`：Err 直接传播会把局部 usages（语言检测的用量）丢掉，
                // 翻译失败时检测侧已消耗的调用就漏记了。
                let translate_outcome = match sink.as_deref() {
                    Some(sink) => {
                        call_translate_streaming(&client, &model, &base_url, &target_lang, &text, sink)
                            .await
                    }
                    None => call_translate(&client, &model, &base_url, &target_lang, &text).await,
                };
                let (translated, mut translate_usages) = match translate_outcome {
                    Ok(v) => v,
                    Err((e, mut translate_usages)) => {
                        usages.append(&mut translate_usages);
                        return Err((e, usages));
                    }
                };
                usages.append(&mut translate_usages);
                Ok((TranslateOutcome::Translated(translated), usages))
            }
        },
        move |conn, release_id, outcome| match outcome {
            TranslateOutcome::Skipped(original) => {
                if let Err(e) = db::releases::set_body_translated(conn, release_id, &original) {
                    log::error!("保存译文失败 id={}: {}", release_id, e);
                } else {
                    log::info!(
                        "跳过翻译(语言一致): id={} lang={}",
                        release_id,
                        target_lang_log
                    );
                }
            }
            TranslateOutcome::Translated(translated) => {
                if let Err(e) = db::releases::set_body_translated(conn, release_id, &translated) {
                    log::error!("保存译文失败 id={}: {}", release_id, e);
                } else {
                    log_ai_job_result(conn, "INFO", "AI 译文", release_id, "", true);
                }
            }
        },
        |conn, release_id, e| {
            let _ = db::releases::increment_translate_retry_count(conn, release_id);
            log_ai_job_result(conn, "ERROR", "AI 译文", release_id, e, false);
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};


    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": r#"{"summary":"测试摘要内容","importance":"中"}"#
                }
            }]
        })
    }

    #[tokio::test]
    async fn test_call_summary_200_success() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
        assert!(result.is_ok());
        let ((summary, importance), _) = result.unwrap();
        assert_eq!(summary, "测试摘要内容");
        assert_eq!(importance, "中");
    }

    #[tokio::test]
    async fn test_call_summary_429_then_200() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_call_summary_429_exhausted() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("429"));
    }

    #[tokio::test]
    async fn test_call_summary_400_no_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_summary(&client, "test-model", &mock.uri(), DEFAULT_DEEPSEEK_PROMPT_EDITABLE, "Some release body").await;
        assert!(result.is_err());
        // 非429不重试，错误不应包含"重试"
        assert!(!result.unwrap_err().0.contains("重试"));
    }

    // ── 翻译任务测试 ──────────────────────────────────────

    fn sample_translate_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": "这是译文内容"
                }
            }]
        })
    }

    #[tokio::test]
    async fn test_call_translate_200_success() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_translate(&client, "test-model", &mock.uri(), "中文", "Some release body").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0, "这是译文内容");
    }

    #[tokio::test]
    async fn test_call_translate_429_then_200() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_translate(&client, "test-model", &mock.uri(), "中文", "body").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_call_translate_empty_content_errors() {
        let mock = MockServer::start().await;
        let empty_resp = serde_json::json!({
            "choices": [{ "message": { "content": "   " } }]
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_resp))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_translate(&client, "test-model", &mock.uri(), "中文", "body").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("为空"));
    }

    #[test]
    fn test_read_translate_config_defaults_disabled() {
        use crate::db::init::init_memory_db;
        let conn = init_memory_db().unwrap();
        let (enabled, lang) = read_translate_config(&conn);
        assert!(!enabled, "默认未启用翻译");
        // 无 language 设置时 fallback 为中文
        assert_eq!(lang, "中文");
    }

    #[test]
    fn test_read_translate_config_enabled_and_lang() {
        use crate::db::init::init_memory_db;
        use crate::db::settings;
        let conn = init_memory_db().unwrap();
        settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
        settings::set_setting(&conn, KEY_LANGUAGE, "en-US").unwrap();
        let (enabled, lang) = read_translate_config(&conn);
        assert!(enabled);
        assert_eq!(lang, "English");
    }

    // ── 语言检测测试 ────────────────────────────────

    fn sample_detect_response(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": lang
                }
            }]
        })
    }

    #[tokio::test]
    async fn test_call_detect_language_english() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("English")))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_detect_language(&client, "test-model", &mock.uri(), "Fixed a bug").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0, "English");
    }

    #[tokio::test]
    async fn test_call_detect_language_chinese() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("中文")))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_detect_language(&client, "test-model", &mock.uri(), "修复了一个问题").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0, "中文");
    }

    #[tokio::test]
    async fn test_call_detect_language_empty_content_errors() {
        let mock = MockServer::start().await;
        let empty_resp = serde_json::json!({
            "choices": [{ "message": { "content": "" } }]
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_resp))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = call_detect_language(&client, "test-model", &mock.uri(), "some text").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("为空"));

        // 空结果不重试：同一个请求重试结果必然一样，而检测失败本来就不阻塞翻译。
        // 此前这里会重试 3 次（睡 2+4+8 秒）才开始翻译——「首字特别慢」的一半就是它。
        let reqs = mock.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1, "空结果不应重试");
        // 思考模式默认开启且 effort=high，20 token 预算会被思维链吃光并返回空 content，
        // 即「空结果」的最常见成因：必须顶关掉
        let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(sent["thinking"]["type"], serde_json::json!("disabled"));
        assert_eq!(sent["max_tokens"], serde_json::json!(20));
    }

    // ── Semaphore 并发限制测试 ────────────────────────────────────

    #[tokio::test]
    async fn test_semaphore_limits_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // 模拟生产代码的 Semaphore 模式：acquire_owned 在 spawn 内部
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let s = sem.clone();
            let p = peak.clone();
            let a = active.clone();

            handles.push(tokio::spawn(async move {
                // acquire 在 spawn 内部，与生产代码模式一致
                let _permit = s.acquire_owned().await.unwrap();
                let v = a.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(v, Ordering::SeqCst);
                // 保持一段时间让其他任务有机会同时运行
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                a.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let max = peak.load(Ordering::SeqCst);
        assert!(max <= 2, "并发峰值 {} 不应超过信号量限制 2", max);
        assert_eq!(max, 2, "应能同时运行 2 个任务");
    }

    #[tokio::test]
    async fn test_semaphore_single_permit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let sem = Arc::new(tokio::sync::Semaphore::new(1));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..3 {
            let s = sem.clone();
            let p = peak.clone();
            let a = active.clone();

            handles.push(tokio::spawn(async move {
                let _permit = s.acquire_owned().await.unwrap();
                let v = a.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(v, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                a.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let max = peak.load(Ordering::SeqCst);
        assert!(max <= 1, "并发峰值 {} 不应超过信号量限制 1", max);
        assert_eq!(max, 1, "应严格串行执行");
    }

    // ── 流式翻译（手动单条：SSE 分片 / 形态兜底 / 重试边界）──

    /// SSE 报文：`data:` 行逐片累积，末片带 usage，`[DONE]` 结束。
    fn sse_body(deltas: &[&str]) -> String {
        let mut s = String::new();
        for d in deltas {
            s.push_str(&format!(
                "data: {}\n\n",
                serde_json::json!({"choices": [{"delta": {"content": d}}]})
            ));
        }
        s.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 22,
                    "prompt_cache_hit_tokens": 3,
                    "prompt_cache_miss_tokens": 8
                }
            })
        ));
        s.push_str("data: [DONE]\n\n");
        s
    }

    /// 收集分片的回调（测试用 sink）。
    fn chunk_collector() -> (std::sync::Arc<std::sync::Mutex<Vec<String>>>, impl Fn(&str) + Send + Sync) {
        let store = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink_store = store.clone();
        (store, move |d: &str| sink_store.lock().unwrap().push(d.to_string()))
    }

    #[tokio::test]
    async fn test_chat_completion_stream_accumulates_deltas_and_usage() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body(&["这是", "译文"]), "text/event-stream"),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let body = serde_json::json!({"model": "m", "messages": []});
        let out = chat_completion_stream(&client, &mock.uri(), &body, &sink)
            .await
            .unwrap();

        assert_eq!(out.content, "这是译文", "分片应累加为全文");
        assert_eq!(*chunks.lock().unwrap(), vec!["这是".to_string(), "译文".to_string()]);
        let usage = out.usage.expect("末片携带的 usage 应被采集");
        assert_eq!(usage.prompt_tokens, 11);
        assert_eq!(usage.completion_tokens, 22);
        assert_eq!(usage.cache_hit_tokens, 3);

        // 请求侧：必须声明流式，并要求末片带 usage（否则统计只能退化为字符数估算）
        let req = &mock.received_requests().await.unwrap()[0];
        let sent: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(sent["stream"], serde_json::json!(true));
        assert_eq!(
            sent["stream_options"]["include_usage"],
            serde_json::json!(true)
        );
    }

    /// 流被截断（既无 `[DONE]` 也无 `finish_reason`）：判失败。
    /// 已到达的分片照常投递（前端留住半截并提示），但绝不落库——半截译文写进去后
    /// 用户看到的就是一份看不出破绽的「完整译文」。
    #[tokio::test]
    async fn test_chat_completion_stream_truncated_is_error() {
        let mock = MockServer::start().await;
        let body = format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "半截"}}]})
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let body = serde_json::json!({"model": "m", "messages": []});
        let out = chat_completion_stream(&client, &mock.uri(), &body, &sink).await;

        let (status, msg) = out.expect_err("无生成结束标记应判失败");
        assert_eq!(status, 0, "截断按非 HTTP 层失败上报（status 0）");
        assert!(msg.contains("中断"), "错误文案应说明是流中断: {}", msg);
        assert_eq!(
            *chunks.lock().unwrap(),
            vec!["半截".to_string()],
            "已到达的分片保留给前端作部分译文"
        );
    }

    /// 只有 `finish_reason`、没有 `[DONE]`：视为生成收全（部分中转如此）。
    /// 终结判定取「两者或」，否则会把正常完成的流判成截断。
    #[tokio::test]
    async fn test_chat_completion_stream_finish_reason_without_done_is_ok() {
        let mock = MockServer::start().await;
        let mut body = format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "译文"}}]})
        );
        body.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
        ));
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let body = serde_json::json!({"model": "m", "messages": []});
        let out = chat_completion_stream(&client, &mock.uri(), &body, &sink)
            .await
            .expect("带 finish_reason 的流不应判截断");

        assert_eq!(out.content, "译文");
        assert_eq!(*chunks.lock().unwrap(), vec!["译文".to_string()]);
    }

    /// 网关无视 `stream: true` 回整包 JSON：按首字节判定走整包解析，内容一次性投递。
    /// 「请求流式却拿到静默非流式」不能变成失败。
    #[tokio::test]
    async fn test_chat_completion_stream_falls_back_to_plain_json() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let body = serde_json::json!({"model": "m", "messages": []});
        let out = chat_completion_stream(&client, &mock.uri(), &body, &sink)
            .await
            .unwrap();

        assert_eq!(out.content, "这是译文内容");
        assert_eq!(*chunks.lock().unwrap(), vec!["这是译文内容".to_string()]);
    }

    /// `stream_options` 被网关拒（400）：去掉该字段重发。
    /// 为用量统计丢整条译文不值得，退了仍有内容，只是退化为估算。
    #[tokio::test]
    async fn test_chat_completion_stream_reposts_without_stream_options_on_400() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string("unknown field: stream_options"))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body(&["译文"]), "text/event-stream"),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let body = serde_json::json!({"model": "m", "messages": []});
        let out = chat_completion_stream(&client, &mock.uri(), &body, &sink)
            .await
            .unwrap();

        assert_eq!(out.content, "译文");
        assert_eq!(*chunks.lock().unwrap(), vec!["译文".to_string()], "重发后的分片照常投递");
        let reqs = mock.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2, "400 应触发一次不带 stream_options 的重发");
        let second: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
        assert!(second.get("stream_options").is_none());
        assert_eq!(second["stream"], serde_json::json!(true), "重发仍是流式");
    }

    /// 首个分片前失败（429）：照常重试。重试的第二轮从零开始，不污染已投递内容。
    #[tokio::test]
    async fn test_call_translate_streaming_retries_before_first_delta() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body(&["这是", "译文"]), "text/event-stream"),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let out = call_translate_streaming(&client, "m", &mock.uri(), "中文", "body", &sink).await;

        assert_eq!(out.map(|(s, _)| s).as_deref(), Ok("这是译文"));
        assert_eq!(*chunks.lock().unwrap(), vec!["这是".to_string(), "译文".to_string()]);
    }

    /// 已投递分片后的失败：**不得重试**——重试会从零重新生成，与用户眼前已有的
    /// 分片叠成两份文本，且上游已为作废的那轮计费。
    #[tokio::test]
    async fn test_call_translate_streaming_does_not_retry_after_first_delta() {
        let mock = MockServer::start().await;
        let mut body = format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "半截"}}]})
        );
        // 流中错误帧（限流/上游故障）
        body.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"error": {"message": "rate limited"}})
        ));
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (chunks, sink) = chunk_collector();
        let out = call_translate_streaming(&client, "m", &mock.uri(), "中文", "body", &sink).await;

        assert!(out.is_err(), "流中错误应上报失败（不落库）");
        assert_eq!(*chunks.lock().unwrap(), vec!["半截".to_string()], "已到达的分片保留");
        assert_eq!(
            mock.received_requests().await.unwrap().len(),
            1,
            "已投递分片后不得重试"
        );
    }

    /// 手动单条流式翻译全链路：detect 判定非目标语言 → SSE 逐片投递，落库完整译文。
    #[tokio::test]
    async fn test_translate_single_release_streams_chunks_and_persists() {
        use wiremock::matchers::body_string_contains;
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("请判断以下文本的主体语言"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("English")))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("完整翻译成"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body(&["这是", "译文"]), "text/event-stream"),
            )
            .mount(&mock)
            .await;

        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let chunks: std::sync::Arc<std::sync::Mutex<Vec<(i64, String)>>> = Default::default();
        let sink_chunks = chunks.clone();
        let sink: ChunkSink = std::sync::Arc::new(move |rid: i64, delta: &str| {
            sink_chunks.lock().unwrap().push((rid, delta.to_string()));
        });

        translate_single_release(&pool, &sem, id, "release body".to_string(), Some(sink)).await;

        // 分片必须带 release_id：并行翻译时前端靠它认出自己那条
        assert_eq!(
            *chunks.lock().unwrap(),
            vec![(id, "这是".to_string()), (id, "译文".to_string())]
        );
        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(rel.body_translated.as_deref(), Some("这是译文"));
    }

    /// 手动单条翻译遇到截断流：分片照常到手，但**不落库**。
    /// 与前端「留住半截 + 红字中断提示 + 可重试」的行为配套：落库了就没有重试入口。
    #[tokio::test]
    async fn test_translate_single_release_truncated_stream_does_not_persist() {
        use wiremock::matchers::body_string_contains;
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("请判断以下文本的主体语言"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("English")))
            .mount(&mock)
            .await;
        let truncated = format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "半截译文"}}]})
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("完整翻译成"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(truncated, "text/event-stream"))
            .mount(&mock)
            .await;

        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let chunks: std::sync::Arc<std::sync::Mutex<Vec<(i64, String)>>> = Default::default();
        let sink_chunks = chunks.clone();
        let sink: ChunkSink = std::sync::Arc::new(move |rid: i64, delta: &str| {
            sink_chunks.lock().unwrap().push((rid, delta.to_string()));
        });

        translate_single_release(&pool, &sem, id, "release body".to_string(), Some(sink)).await;

        assert_eq!(
            *chunks.lock().unwrap(),
            vec![(id, "半截译文".to_string())],
            "截断前已到达的分片仍要交给前端"
        );
        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(
            rel.body_translated.is_none(),
            "截断的半截译文不得当成成品落库: {:?}",
            rel.body_translated
        );
    }

    /// 语言检测命中目标语言：短路写原文、不投递任何分片（保持既有行为）。
    #[tokio::test]
    async fn test_translate_single_release_language_match_emits_no_chunk() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("中文")))
            .mount(&mock)
            .await;

        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "原文内容")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let sink_chunks = chunks.clone();
        let sink: ChunkSink = std::sync::Arc::new(move |_rid: i64, delta: &str| {
            sink_chunks.lock().unwrap().push(delta.to_string());
        });

        translate_single_release(&pool, &sem, id, "原文内容".to_string(), Some(sink)).await;

        assert!(chunks.lock().unwrap().is_empty(), "短路无译文可流式投递");
        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(rel.body_translated.as_deref(), Some("原文内容"));
    }

    /// 需注意：重发时不应再带任何非标准字段。
    #[tokio::test]
    async fn test_chat_completion_reposts_without_optional_fields_on_400() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string("unknown field: thinking"))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 与 call_translate 同形：带 thinking 的非标准字段
        let mut body = serde_json::json!({"model": "m", "messages": []});
        disable_thinking(&mut body);
        let out = chat_completion(&client, &mock.uri(), &body).await.unwrap();

        assert_eq!(out.content, "这是译文内容");
        let reqs = mock.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2, "400 应触发一次去掉扩展字段的重发");
        let second: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
        assert!(second.get("thinking").is_none());
        assert_eq!(second["stream"], serde_json::json!(false));
    }

    /// 翻译/流式翻译路径都必须带 `thinking: disabled`：默认开启的思考模式会把
    /// 首字拖到整段思维链之后（译文只会在 `content` 里出现，思考走 `reasoning_content`）。
    #[tokio::test]
    async fn test_translate_requests_disable_thinking() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse_body(&["译文"]), "text/event-stream"),
            )
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (_, sink) = chunk_collector();
        call_translate_streaming(&client, "m", &mock.uri(), "中文", "body", &sink)
            .await
            .unwrap();

        let sent: serde_json::Value =
            serde_json::from_slice(&mock.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(sent["thinking"]["type"], serde_json::json!("disabled"));
        assert_eq!(sent["stream"], serde_json::json!(true));
    }

    // ── 编排函数 generate_summaries_for_new / generate_translations_for_new 集成测试 ──
    //
    // 注入 init_memory_pool + wiremock，直接驱动真实的公开编排函数，覆盖
    // enabled / api_key / 成功写摘要 / 失败重试计数 / 空体跳过 /
    // 翻译 force 绕过开关 / 语言检测短路写原文 / 翻译失败重试 等编排分支。

    fn enable_deepseek(conn: &rusqlite::Connection, base_url: &str) {
        crate::crypto::set_test_master_key();
        db::settings::set_setting(conn, KEY_DEEPSEEK_ENABLED, "true").unwrap();
        db::settings::set_setting(conn, KEY_DEEPSEEK_BASE_URL, base_url).unwrap();
        db::settings::set_setting(conn, KEY_DEEPSEEK_API_KEY, &crate::crypto::encrypt("test-key")).unwrap();
    }

    fn insert_release_with_body(conn: &rusqlite::Connection, body: &str) -> i64 {
        let sid = db::sources::add_source(conn, "github", "o", "r", "").unwrap();
        db::releases::insert_release(
            conn, sid, "v1", "v1", "https://github.com/o/r/releases/tag/v1",
            "2024-01-01T00:00:00Z", false, Some(body),
        ).unwrap()
    }

    fn retry_count(conn: &rusqlite::Connection, id: i64) -> i64 {
        conn.query_row(
            "SELECT COALESCE(retry_count, 0) FROM releases WHERE id = ?1",
            rusqlite::params![id], |r| r.get(0),
        ).unwrap()
    }

    fn translate_retry_count(conn: &rusqlite::Connection, id: i64) -> i64 {
        conn.query_row(
            "SELECT COALESCE(translate_retry_count, 0) FROM releases WHERE id = ?1",
            rusqlite::params![id], |r| r.get(0),
        ).unwrap()
    }

    /// 未启用 DeepSeek：编排函数应早返回，不发起任何请求、不写摘要。
    #[tokio::test]
    async fn test_generate_summaries_disabled_no_call() {
        let _mock = MockServer::start().await; // 不挂 mock：若误调用会落到错误分支
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("body".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none(), "未启用 AI 不应写摘要");
        assert_eq!(retry_count(&conn, id), 0, "未启用不应触发请求");
    }

    /// 已启用但未配置 api_key：应早返回，不发起请求。
    #[tokio::test]
    async fn test_generate_summaries_no_api_key_no_call() {
        let _mock = MockServer::start().await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            db::settings::set_setting(&conn, KEY_DEEPSEEK_ENABLED, "true").unwrap();
            // 故意不设置 api_key
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("body".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none());
        assert_eq!(retry_count(&conn, id), 0, "无 api_key 不应触发请求");
    }

    /// 成功路径：mock 200 返回摘要 JSON，应写回 ai_summary / ai_importance 并重置 retry_count。
    #[tokio::test]
    async fn test_generate_summaries_success_writes_summary() {
        let mock = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "release body content")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("release body content".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(rel.ai_summary.as_deref(), Some("测试摘要内容"));
        assert_eq!(rel.ai_importance.as_deref(), Some("中"));
        assert_eq!(retry_count(&conn, id), 0, "成功后 retry_count 应被重置");
    }

    /// 失败路径：mock 500（非 429 不重试），应递增 retry_count 且不写摘要。
    #[tokio::test]
    async fn test_generate_summaries_error_increments_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, Some("release body".to_string()))]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none(), "失败不应写摘要");
        assert!(retry_count(&conn, id) >= 1, "失败应递增 retry_count");
    }

    /// body 为 None：应被 continue 跳过，不发起请求（retry_count 保持 0 证明未触发错误分支）。
    #[tokio::test]
    async fn test_generate_summaries_skips_none_body() {
        let mock = MockServer::start().await; // 无 mock
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_summaries_for_new(&pool, &sem, &[(id, None)]).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.ai_summary.is_none());
        assert_eq!(retry_count(&conn, id), 0, "None body 不应触发请求");
    }

    /// 翻译：translate 未启用且 force=false → 早返回，不翻译、不触发请求。
    #[tokio::test]
    async fn test_generate_translations_disabled_no_force_noop() {
        let mock = MockServer::start().await; // 无 mock
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            // KEY_DEEPSEEK_TRANSLATE_RELEASE 默认 false
            insert_release_with_body(&conn, "body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("body".to_string()))], false).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.body_translated.is_none(), "translate 未启用且非 force 不应翻译");
        assert_eq!(translate_retry_count(&conn, id), 0);
    }

    /// 翻译：force=true 绕过 translate 开关。detect 返回非目标语言 → 继续翻译并写译文。
    #[tokio::test]
    async fn test_generate_translations_force_bypasses_switch() {
        let mock = MockServer::start().await;
        // 单 mock：detect 得 "English"(≠ 默认中文 → 继续翻译)，translate 得 "English" 作为译文
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("English")))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            // translate_release 未启用，靠 force=true 绕过
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("release body".to_string()))], true).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.body_translated.is_some(), "force=true 应绕过开关执行翻译");
        assert_eq!(translate_retry_count(&conn, id), 0);
    }

    /// 翻译：语言检测 == 目标语言 → 短路跳过翻译，直接写原文为 body_translated。
    #[tokio::test]
    async fn test_generate_translations_language_match_writes_original() {
        let mock = MockServer::start().await;
        // detect 返回 "中文" == 默认 target_lang("中文") → 跳过翻译，写原文
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_detect_response("中文")))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            db::settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
            insert_release_with_body(&conn, "原文内容")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("原文内容".to_string()))], false).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(rel.body_translated.as_deref(), Some("原文内容"), "语言一致应直接写原文跳过翻译");
    }

    /// 翻译失败：detect 与 translate 均 500（detect 失败不阻塞翻译），应递增 translate_retry_count。
    #[tokio::test]
    async fn test_generate_translations_error_increments_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock).await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            db::settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("release body".to_string()))], false).await;

        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert!(rel.body_translated.is_none(), "翻译失败不应写译文");
        assert!(translate_retry_count(&conn, id) >= 1, "翻译失败应递增 translate_retry_count");
    }
    #[test]
    fn test_resolve_chat_completion_url() {
        // 根地址 → 补 /v1/chat/completions
        assert_eq!(resolve_chat_completion_url("https://api.deepseek.com"), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(resolve_chat_completion_url("https://api.deepseek.com/"), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(resolve_chat_completion_url(" https://api.deepseek.com/ "), "https://api.deepseek.com/v1/chat/completions");
        // 带 /api/v1 前缀 → 补 /chat/completions
        assert_eq!(resolve_chat_completion_url("https://api.cline.bot/api/v1"), "https://api.cline.bot/api/v1/chat/completions");
        // 已含完整端点 → 原样返回
        assert_eq!(resolve_chat_completion_url("https://host/api/v1/chat/completions"), "https://host/api/v1/chat/completions");
    }

    /// describe_reqwest_error：真实请求超时路径。client 总超时 300ms + wiremock
    /// 延迟 2s 响应 → .send() 报超时错误；断言 chat_completion 的错误文案包含
    /// 明确的「请求超时」前缀（而非笼统的 "error sending request for url"）。
    #[tokio::test]
    async fn test_chat_completion_timeout_reports_timeout() {
        use wiremock::matchers::{method, path};
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(sample_response())
                    .set_delay(std::time::Duration::from_secs(5)),
            )
            .mount(&mock)
            .await;

        // 与生产同构：短超时的 client
        let client = crate::http::build_http_client(crate::http::HttpClientConfig {
            proxy_url: "",
            proxy_mode: "none",
            bearer_token: Some("test-key"),
            timeout_secs: 1,
            content_type_json: true,
            set_default_auth: true,
            ..Default::default()
        })
        .unwrap();
        let body = serde_json::json!({
            "model": "test",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 10,
        });
        let err = chat_completion(&client, &mock.uri(), &body)
            .await
            .unwrap_err();
        assert_eq!(err.0, 0, "网络层错误 status=0");
        assert!(
            err.1.contains("请求超时") || err.1.contains("超时"),
            "超时应报明确超时文案，实际: {}",
            err.1
        );
    }

    // ── 重试判定 is_retryable ──
    //
    // 契约：瞬时故障（429 限流、status=0 网络层/空内容、520/524 网关上游故障）重试；
    // 确定性错误（400/401/404 及 500/502/503/504）不重试——上游过载时退避重试雪上加霜。

    #[test]
    fn test_is_retryable_covers_transient_and_rejects_permanent() {
        // 可重试：429 限流
        assert!(is_retryable(&(429, "rate limited".into())), "429 应重试");
        // 可重试：status=0（未拿到 HTTP 响应 / 模型返回空内容）
        assert!(
            is_retryable(&(0, "翻译结果为空".into())),
            "空内容应重试（实测 12 次译文失败里 3 次是它，此前一次即判死）"
        );
        assert!(
            is_retryable(&(0, "请求失败: error sending request".into())),
            "网络层错误应重试"
        );
        // 可重试：52x 网关上游故障。网关对同一故障消息
        // "Upstream model provider is temporarily unavailable. Please try again
        // in a moment." 会随机返回 520 或 524，语义就是请调用方重试，且此时
        // 已白等一整次长请求，放弃太亏。
        for status in [520u16, 524] {
            assert!(
                is_retryable(&(
                    status,
                    "Upstream model provider is temporarily unavailable".into()
                )),
                "{} 网关上游故障应重试",
                status
            );
        }
        // 不重试：确定性 HTTP 错误
        for status in [400u16, 401, 403, 404] {
            assert!(
                !is_retryable(&(status, "client error".into())),
                "{} 是确定性错误，重试无用",
                status
            );
        }
        // 其余 5xx 仍不重试（上游过载时退避重试会雪上加霜，52x 是唯一例外）
        for status in [500u16, 502, 503, 504] {
            assert!(
                !is_retryable(&(status, "server error".into())),
                "{} 不重试（仅 52x 例外）",
                status
            );
        }
    }

    /// 524 重试的端到端验证：网关先返回 524（上游暂时不可用），重试后给出译文。
    /// 这是真实故障场景——摘要能过、翻译撞 524，一次即失败会让用户完全看不到原因。
    #[tokio::test]
    async fn test_translate_retries_on_524_upstream_timeout() {
        let mock = MockServer::start().await;
        // 第 1 次：524 上游超时
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(524).set_body_json(serde_json::json!({
                "error": {
                    "message": "Upstream model provider is temporarily unavailable. Please try again in a moment.",
                    "type": "server_error"
                }
            })))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        // 第 2 次起：正常译文
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        let client = crate::http::build_http_client(crate::http::HttpClientConfig {
            proxy_url: "",
            proxy_mode: "none",
            bearer_token: Some("test-key"),
            timeout_secs: 5,
            content_type_json: true,
            set_default_auth: true,
            ..Default::default()
        })
        .unwrap();
        let out = call_translate(&client, "test-model", &mock.uri(), "中文", "some body").await;
        assert_eq!(
            out.map(|(s, _)| s).as_deref(),
            Ok("这是译文内容"),
            "524 应触发重试并最终拿到译文"
        );
    }

    /// 空内容重试的端到端验证：前两次返回空 choices，第三次给正常译文 →
    /// 重试链路应救回这次请求。
    ///
    /// 用 detect 短路把链路收敛到单次 translate 调用，避免 detect 的重试
    /// 与 translate 的重试叠加导致 mock 次数难以推断。
    #[tokio::test]
    async fn test_translate_retries_on_empty_content() {
        let mock = MockServer::start().await;
        // 第 1、2 次：200 但 content 为空（模拟中转网关返回空内容）
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{ "message": { "content": "" } }]
            })))
            .up_to_n_times(2)
            .mount(&mock)
            .await;
        // 第 3 次起：正常译文
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;

        // 直接测 call_translate（跳过 detect），只跑 translate 一条重试链
        let client = crate::http::build_http_client(crate::http::HttpClientConfig {
            proxy_url: "",
            proxy_mode: "none",
            bearer_token: Some("test-key"),
            timeout_secs: 5,
            content_type_json: true,
            set_default_auth: true,
            ..Default::default()
        })
        .unwrap();
        let out = call_translate(&client, "test-model", &mock.uri(), "中文", "some body").await;
        assert_eq!(
            out.map(|(s, _)| s).as_deref(),
            Ok("这是译文内容"),
            "空内容应触发重试并最终拿到译文"
        );
    }

    // ── 调用前复查跳过（already_done）──
    //
    // 契约：自动批内若该条在批跑期间已被别的路径写入结果，则跳过、不发请求，
    // 避免重复烧 token，更避免「已成功的条目被重复请求判失败、retry 计数 +1」。

    /// 自动批（force=false）：已有译文 → 跳过，不发起任何请求。
    /// 用「mock 零调用次数」证明请求未发出。
    #[tokio::test]
    async fn test_auto_batch_skips_when_translation_already_exists() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            db::settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
            let id = insert_release_with_body(&conn, "release body");
            // 预先写入译文：模拟「批启动后、本条执行前已被别的路径写入」
            db::releases::set_body_translated(&conn, id, "已有的译文").unwrap();
            id
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(
            &pool,
            &sem,
            &[(id, Some("release body".to_string()))],
            false,
        )
        .await;

        // 关键断言：一次请求都没发（此前会重复翻译并可能把 retry 计数加 1）
        assert_eq!(
            mock.received_requests().await.unwrap().len(),
            0,
            "已有译文时自动批不应发起任何 AI 请求"
        );
        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(
            rel.body_translated.as_deref(),
            Some("已有的译文"),
            "已有译文不应被覆盖"
        );
        assert_eq!(
            translate_retry_count(&conn, id),
            0,
            "跳过的条目不应递增 retry 计数"
        );
    }

    /// 手动单条翻译（force=true）：即使已有译文也必须执行——用户是显式要求重翻，
    /// 若被「已有译文」挡住，设置页点了翻译会毫无反应且不报错。
    #[tokio::test]
    async fn test_force_translation_reruns_despite_existing_translation() {
        let mock = MockServer::start().await;
        // detect 得 "English"(≠ 默认中文) → 不短路，走真实翻译
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sample_detect_response("English")),
            )
            .mount(&mock)
            .await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            let id = insert_release_with_body(&conn, "release body");
            db::releases::set_body_translated(&conn, id, "旧译文").unwrap();
            id
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(&pool, &sem, &[(id, Some("release body".to_string()))], true)
            .await;

        assert!(
            !mock.received_requests().await.unwrap().is_empty(),
            "force=true 应无视已有译文，照常发起请求"
        );
        let conn = pool.get().unwrap();
        let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
        assert_eq!(
            rel.body_translated.as_deref(),
            Some("English"),
            "force=true 应用新译文覆盖旧译文"
        );
    }

    /// `build_client` 失败必须写 DB 日志：log 插件仅 debug 构建启用，只打
    /// log::error! 对 release 版用户完全不可见，「待办非空、开关全开、
    /// retry 计数 = 0 却无任何可见错误」这类静默不发请求故障将无法定位。
    /// 用带换行的非法 API key 触发 build_client 确定性失败，断言：
    /// 零请求 + DB 出现 ERROR 日志 + retry 计数不动（失败发生在发请求之前）。
    #[tokio::test]
    async fn test_build_client_failure_writes_db_log() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_translate_response()))
            .mount(&mock)
            .await;
        let pool = crate::db::init::init_memory_pool().unwrap();
        let id = {
            let conn = pool.get().unwrap();
            enable_deepseek(&conn, &mock.uri());
            // 覆写为带换行的非法 key（与 test_build_deepseek_client_invalid_key 同触发条件）
            db::settings::set_setting(
                &conn,
                KEY_DEEPSEEK_API_KEY,
                &crate::crypto::encrypt("bad\nkey"),
            )
            .unwrap();
            db::settings::set_setting(&conn, KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
            insert_release_with_body(&conn, "release body")
        };
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        generate_translations_for_new(
            &pool,
            &sem,
            &[(id, Some("release body".to_string()))],
            false,
        )
        .await;

        // 失败发生在建 client，一个请求都不该发出
        assert_eq!(
            mock.received_requests().await.unwrap().len(),
            0,
            "build_client 失败时不应发出任何请求"
        );
        let conn = pool.get().unwrap();
        // 关键断言：错误进了 DB（日志 tab 可见），不再只打不可见的 console log
        let has_err: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM logs WHERE level='ERROR' AND message LIKE '%创建 DeepSeek 客户端失败%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            has_err, 1,
            "build_client 失败应写入一条 ERROR DB 日志（release 版用户唯一可见的错误渠道）"
        );
        assert_eq!(
            translate_retry_count(&conn, id),
            0,
            "建 client 阶段失败不应递增 retry 计数（与发请求后的失败区分）"
        );
    }
}
