use std::error::Error as StdError;

use crate::db::ai_usage::RawUsage;

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
pub(super) fn extract_content(json: &serde_json::Value) -> String {
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
pub(super) fn extract_usage(json: &serde_json::Value) -> Option<RawUsage> {
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
pub(super) const LOCAL_CODE_BASE: u16 = 600;

/// 「上游返回空内容」（语言检测用）。
///
/// 必须**不在 `is_retryable` 的瞬时故障白名单里**（0/429/520/524）：空结果重试是
/// 纯浪费——同一个请求重试结果必然一样（最常见成因是思考模式把 `max_tokens`
/// 预算全吃在思维链上），而检测失败本来就不阻塞翻译（代码按语言不一致照常翻译），
/// 此前为它睡 2+4+8 秒指数退避才开始翻译。
pub(super) const LOCAL_CODE_EMPTY_CONTENT: u16 = LOCAL_CODE_BASE;

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
pub(super) const OPTIONAL_BODY_FIELDS: [&str; 2] = ["thinking", "stream_options"];

/// 关闭思考模式：思维链经 `reasoning_content` 返回（与 `content` 同级），
/// 而我们只读 `content`——于是整段思考期间前端一个字都不显示，表现为「首字极慢」。
///
/// 且思考默认开启、effort 默认 high（见 DeepSeek 思考模式文档），翻译/语言检测这类
/// 确定性任务并不需要它：一段 849 字符的正文也能思考出约 2000 token（实测用量表
/// 对得上），这些 token 按输出计价，首字要等约 9 秒。
/// 思考模式还忽略 `temperature`（不报错但不生效），关掉后才能真正生效。
pub(super) fn disable_thinking(body_json: &mut serde_json::Value) {
    if let Some(obj) = body_json.as_object_mut() {
        obj.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "disabled" }),
        );
    }
}

/// 去掉请求体里的扩展字段（见 `OPTIONAL_BODY_FIELDS`），返回是否确实去掉了。
pub(super) fn strip_optional_fields(body_json: &mut serde_json::Value) -> bool {
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
pub(super) async fn send_chat_request(
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
pub(super) async fn read_error_body(resp: reqwest::Response) -> (u16, String) {
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
pub(super) fn describe_reqwest_error(e: &reqwest::Error) -> String {
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

