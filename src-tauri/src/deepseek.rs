use rusqlite::Connection;
use std::error::Error as StdError;

use crate::credential;
use crate::db;
use crate::db::settings::{
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_MODEL, KEY_DEEPSEEK_BASE_URL, KEY_DEEPSEEK_API_KEY,
    KEY_DEEPSEEK_PROXY_BYPASS, KEY_DEEPSEEK_PROMPT, KEY_DEEPSEEK_TRANSLATE_RELEASE, KEY_PROXY_URL,
    KEY_LANGUAGE,
    DEFAULT_DEEPSEEK_MODEL, DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_PROMPT_EDITABLE,
    DEFAULT_DEEPSEEK_TRANSLATE_PROMPT, DEFAULT_DEEPSEEK_TRANSLATE_RELEASE,
    DEEPSEEK_PROMPT_FIXED_SUFFIX,
};

/// DeepSeek 配置聚合（替代原 5 元组按位置取值，消除索引魔法）。
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
/// 摘要与翻译两个入口共用，避免逐字重复。
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
/// 兼容三类填法（OpenAI 兼容生态常见），避免像旧版那样无条件追加 `/v1/chat/completions`：
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

/// DeepSeek 单次请求的总超时（秒）。reqwest `.timeout()` 覆盖建连→发请求→读完响应体全程，
/// 中转网关往往等到上游生成完才回响应头，故耗时主要由 token 数与网关排队决定。
/// - 翻译（max_tokens=20000 + 正文截 20000 字符）最重：原先写死 60s，曾在
///   commandcode 网关实测间歇超时（用户日志 "error sending request"），放宽到
///   300s。翻译已改为后台异步批（fire-and-forget，不阻塞轮询/手动检查），
///   长耗时对前端无感，故取值偏向「宁可等完也别中途放弃」——一次成功好过
///   超时后重排下一轮再等一遍。
/// - 摘要（max_tokens=800）多数秒回，但同样受网关排队牵连、偶发撞 60s，放宽到 120s。
/// - 连接测试仍用 60s：只发一小段探测请求，且要快速失败，不让用户在设置页久等。
pub const DEEPSEEK_TIMEOUT_SECS_SUMMARY: u64 = 120;
pub const DEEPSEEK_TIMEOUT_SECS_TRANSLATE: u64 = 300;
pub const DEEPSEEK_TIMEOUT_SECS_TEST: u64 = 60;

/// 翻译请求的 `max_tokens`：输出侧上限。
///
/// 与输入截断 `DEEPSEEK_TRANSLATE_TRUNCATE_CHARS` 必须保持同量级：中英互译
/// 时输出字符数与输入相当，输出上限若显著小于输入长度，译文会在句子中间被
/// 硬性截断（实测 v2.0.10 结尾停在半个 URL）。改动其中一个时请一并核对另一个。
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

/// 通用 chat/completions 调用：POST → 判 success → 取 content → 错误映射。
/// 三个 call_*（摘要/语言检测/翻译）与连接测试共用此模板，
/// 改超时/重试/错误格式只需动此处一处。
/// 返回 (status, msg)：status>0 时 msg 为 API 原始响应文本；status=0 时 msg 为网络/解析错误描述。
pub(crate) async fn chat_completion(
    client: &reqwest::Client,
    base_url: &str,
    body_json: &serde_json::Value,
) -> Result<String, (u16, String)> {
    let endpoint = resolve_chat_completion_url(base_url);
    // 显式要求非流式：部分中转（如 Cline）缺省 stream=true 会返回 SSE 流，
    // 与 relwatch 的 `resp.json()` 解析路径冲突。显式禁止即可规避。
    let body = body_json.to_owned().clone();
    // 网络层错误（未收到 HTTP 响应）。reqwest 的 Display 只打顶层文案
    // （"error sending request for url (...)"），真正的失败原因（连接超时/
    // 连接被重置/DNS 等）藏在 source 链里——此前不展开导致日志只能看到
    // 笼统的 url，无法区分是网关超时还是本机断网。逐层展开 source 拼入。
    let resp = client
        .post(&endpoint)
        .json(&{
            let mut b = body;
            if let Some(obj) = b.as_object_mut() {
                obj.insert("stream".to_string(), serde_json::Value::Bool(false));
            }
            b
        })
        .send()
        .await
        .map_err(|e| (0, format!("请求失败: {}", describe_reqwest_error(&e))))?;
    if resp.status().is_success() {
        let json: serde_json::Value = resp.json().await.map_err(|e| (0, format!("解析响应失败: {}", e)))?;
        let content = extract_content(&json);
        return Ok(content);
    }
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Err((status, text))
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
/// 三个 call_*（摘要/语言检测/翻译）共用，此前各有实现、格式曾漂移
/// （detect/translate 曾以重复 status 参数拼出与 summary 相同的输出）。
fn format_chat_error(status: u16, msg: &str) -> String {
    if status > 0 {
        format!("[{}] AI API 返回错误 {}: {}", status, status, msg)
    } else {
        msg.to_string()
    }
}

/// 重试判定（三个 call_* 共用）：区分「值得重试的瞬时故障」与「重试也无用的确定性错误」。
///
/// 可重试：
/// - `429` 限流（退避后通常能过）；
/// - `status == 0` 网络层/空内容错误。status 0 是本地约定的「未拿到 HTTP 响应」
///   标记（见 `chat_completion` 的 `map_err`），涵盖连接超时、连接被重置、
///   DNS 失败，以及模型返回空内容——这些都是瞬时故障，实测近 3 天 12 次译文
///   失败里有 3 次是空内容，全部一次即判死、未重试。
/// - `520` / `524` 网关上游故障。Cloudflare 52x 系的语义是「网关与上游之间的
///   连接层故障」，而非上游应用自身回应的 5xx。commandcode 网关对同一故障
///   消息体 `Upstream model provider is temporarily unavailable. Please try
///   again in a moment.` 实测会随机返回 520 或 524（长正文探针三次：
///   520/524/524，各等 7~127s）——只白名单 524 会把撞上 520 的那次长等待
///   白白作废。网关明确在请你重试；此时请求往往已被上游接收但结果作废，
///   不重试则白白浪费一次长等待（翻译批长达 300s）。
///
/// 不重试：其余 HTTP 状态码（400 参数错、401/403 鉴权错、404，以及 500/502/503/
/// 504 等）。（按用户要求，52x 已纳入可重试；其余 5xx 仍不重试——上游过载时
/// 退避重试会雪上加霜，且这类错误通常需要人工介入而非自动重试。）
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
) -> Result<(String, String), String> {
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

    crate::retry::retry_with_backoff(
        &crate::retry::RetryConfig::default(),
        is_retryable,
        || async {
            let content = chat_completion(client, base_url, &body_json).await?;
            let parsed: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| (0, format!("解析摘要 JSON 失败: {} — 原始内容: {}", e, content)))?;
            let summary = parsed["summary"].as_str().unwrap_or("").to_string();
            let importance = parsed["importance"].as_str().unwrap_or("中").to_string();
            if summary.is_empty() {
                return Err((0, "摘要为空".to_string()));
            }
            Ok((summary, importance))
        },
    )
    .await
    .map_err(|(status, msg)| format_chat_error(status, &msg))
}

/// 调用 AI 检测文本主体语言。仅取 body 前 500 字符以节省 token。
/// 返回 AI 判断的语言名称（如 "中文"/"English"/"日本語"）。
/// 用于翻译前判断是否需要翻译：若检测语言 == 目标语言则跳过。
async fn call_detect_language(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    text_sample: &str,
) -> Result<String, String> {
    let prompt = format!(
        "请判断以下文本的主体语言，仅用一个词回答语言名称（如 中文、English、日本語、Français 等），不要输出其他任何内容。\n\n文本：\n{}",
        text_sample
    );
    let body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.0,
        "max_tokens": 20
    });
    crate::retry::retry_with_backoff(
        &crate::retry::RetryConfig::default(),
        is_retryable,
        || async {
            let content = chat_completion(client, base_url, &body_json).await?;
            if content.is_empty() {
                return Err((0, "语言检测结果为空".to_string()));
            }
            Ok(content)
        },
    )
    .await
    .map_err(|(status, msg)| format_chat_error(status, &msg))
}

/// 调用 AI 翻译 release note 全文。纯文本输出（非 JSON），
/// 保留 Markdown 结构。`target_lang` 为目标语言（如 "中文"/"English"）。
async fn call_translate(
    client: &reqwest::Client,
    model: &str,
    base_url: &str,
    target_lang: &str,
    body_text: &str,
) -> Result<String, String> {
    let prompt = DEFAULT_DEEPSEEK_TRANSLATE_PROMPT
        .replace("{lang}", target_lang)
        .replace("{}", body_text);
    let body_json = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.3,
        "max_tokens": DEEPSEEK_TRANSLATE_MAX_TOKENS
    });

    crate::retry::retry_with_backoff(
        &crate::retry::RetryConfig::default(),
        is_retryable,
        || async {
            let content = chat_completion(client, base_url, &body_json).await?;
            if content.is_empty() {
                return Err((0, "翻译结果为空".to_string()));
            }
            Ok(content)
        },
    )
    .await
    .map_err(|(status, msg)| format_chat_error(status, &msg))
}

/// 写 AI 任务结果日志：统一"成功/失败 × 有/无 release 行"四种组合。
/// 摘要与翻译共用，收敛了原 4 份逐字复制的日志块。
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

/// AI 任务公共流水线：读配置 → 建 client → 并发调度 → 结果/失败分别落库。
///
/// 摘要与翻译两条流水线共用此骨架，收敛了原先整段平行的
/// for/spawn/信号量/spawn_blocking 结构（此前并发/日志语义的修改需同步改
/// 2 处，事实上"语言检测短路"就曾只存在于翻译一侧）。差异点以参数注入：
/// - `job`：控制台日志文案（"摘要"/"译文"）
/// - `truncate_chars`：正文截断上限（摘要 4000 / 翻译见
///   `DEEPSEEK_TRANSLATE_TRUNCATE_CHARS`，须与 `DEEPSEEK_TRANSLATE_MAX_TOKENS` 同量级）
/// - `timeout_secs`：本批请求的总超时（摘要 120 / 翻译 300，见 DEEPSEEK_TIMEOUT_SECS_*）
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
///   重复请求若返回空内容，会走 on_err 把本已成功的 retry 计数加 1——实测
///   v2.0.11 译文已落库后又被判失败（translate_retry_count=1）即此场景。
#[allow(clippy::too_many_arguments)]
async fn run_ai_job<T, F, Fut>(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    job: &'static str,
    truncate_chars: usize,
    timeout_secs: u64,
    extra_ready: impl Fn(&Connection) -> bool,
    already_done: impl Fn(&Connection, i64) -> bool + Send + Sync + 'static,
    call: F,
    on_ok: impl Fn(&Connection, i64, T) + Send + Sync + 'static,
    on_err: impl Fn(&Connection, i64, &str) + Send + Sync + 'static,
) where
    F: Fn(reqwest::Client, String, String, String, String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<T, String>> + Send + 'static,
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
    let client = match build_client(&api_key, &proxy_url, &proxy_mode, timeout_secs) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("创建 DeepSeek 客户端失败: {}", e);
            log::error!("{}", msg);
            // 同时写 DB：log 插件仅在 debug 构建启用，release 版用户看不到
            // log::error!，只打 console 会让「批静默不发请求」无法定位（实测
            // 待办非空、开关全开、retry 计数 = 0 却无任何可见错误即此分支）。
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
            match call(client, model, base_url, prompt, truncated).await {
                Ok(result) => {
                    // 同步 DB 写入收笼进 spawn_blocking，避免阻塞 tokio worker
                    let _ = tokio::task::spawn_blocking(move || {
                        let conn = match db.get() {
                            Ok(c) => c,
                            Err(e) => {
                                log::error!("数据库连接失败: {}", e);
                                return;
                            }
                        };
                        on_ok(&conn, release_id, result);
                    })
                    .await;
                }
                Err(e) => {
                    log::error!("生成{}失败 id={}: {}", job, release_id, e);
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(conn) = db.get() {
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
        |_| true,
        has_ai_summary,
        |client, model, base_url, prompt, text| async move {
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
pub async fn generate_translations_for_new(
    db_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    deepseek_semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    saved: &[(i64, Option<String>)],
    force: bool,
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
        // 翻译全文比摘要耗 token 多，截断上限与 max_tokens 同步放宽到 20000：
        // 此前 12000 字符输入 + 8000 token 输出，中英互译后输出装不下输入，
        // 长 release note 的译文在句子中间被硬截断（实测 v2.0.10/2.0.9 结尾
        // 都是半个 URL / 半个贡献者名）。二者对齐到同一量级，避免输出侧饥饿。
        DEEPSEEK_TRANSLATE_TRUNCATE_CHARS,
        // 翻译超时 300s：max_tokens=20000 的长生成中转可能等到上游出完才回
        // 响应头，原先写死的 60s 曾在 commandcode 网关实测间歇超时
        DEEPSEEK_TIMEOUT_SECS_TRANSLATE,
        // 翻译开关：force 绕过 translate_enabled（手动单条场景）
        move |conn| read_translate_config(conn).0 || force,
        // 复查跳过：仅对自动批（force=false）生效。手动单条翻译（force=true）
        // 是用户显式要求重翻（可能上一版译文不满意），必须放行，不能被
        // 「已有译文」挡住——否则设置页点了翻译却毫无反应、也不报错。
        move |conn, id| !force && has_translation(conn, id),
        move |client, model, base_url, _prompt, text| {
            let target_lang = target_lang.clone();
            async move {
                // 语言检测短路：取 body 前 500 字符让 AI 判断主体语言，
                // 若与目标语言一致则直接把原文返回（由 on_ok 写入 body_translated），
                // 跳过翻译调用。检测失败时不阻塞翻译（视为语言不一致，照常翻译）。
                let sample: String = text.chars().take(500).collect();
                if let Ok(detected) =
                    call_detect_language(&client, &model, &base_url, &sample).await
                {
                    if detected.trim() == target_lang {
                        return Ok(TranslateOutcome::Skipped(text));
                    }
                }
                call_translate(&client, &model, &base_url, &target_lang, &text)
                    .await
                    .map(TranslateOutcome::Translated)
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
        let (summary, importance) = result.unwrap();
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
        assert!(result.unwrap_err().contains("429"));
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
        assert!(!result.unwrap_err().contains("重试"));
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
        assert_eq!(result.unwrap(), "这是译文内容");
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
        assert!(result.unwrap_err().contains("为空"));
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
        assert_eq!(result.unwrap(), "English");
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
        assert_eq!(result.unwrap(), "中文");
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
        assert!(result.unwrap_err().contains("为空"));
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

    // ── 编排函数 generate_summaries_for_new / generate_translations_for_new 集成测试 ──
    //
    // 注入 init_memory_pool + wiremock，直接驱动真实的公开编排函数，覆盖
    // enabled / api_key / 成功写摘要 / 失败重试计数 / 空体跳过 /
    // 翻译 force 绕过开关 / 语言检测短路写原文 / 翻译失败重试 等编排分支。
    // 这条链路原先无测试覆盖（CI 不可达），至此补齐。

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
    /// 明确的「请求超时」前缀（此前只有笼统的 "error sending request for url"）。
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
    // 契约：瞬时故障（429 限流、status=0 网络层/空内容）重试；
    // 确定性错误（400/401/404/5xx）不重试——5xx 时上游可能已过载，退避重试雪上加霜。

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
        // 可重试：52x 网关上游故障。网关（api.commandcode.ai）对同一故障消息
        // "Upstream model provider is temporarily unavailable. Please try again
        // in a moment." 实测随机返回 520 或 524（长正文探针三次：520/524/524）。
        // 语义就是请调用方重试，且此时已白等一整次长请求，放弃太亏。
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
            out.as_deref(),
            Ok("这是译文内容"),
            "524 应触发重试并最终拿到译文"
        );
    }

    /// 空内容重试的端到端验证：前两次返回空 choices，第三次给正常译文 →
    /// 重试链路应救回这次请求（此前 status=0 不重试，一次即失败）。
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
            out.as_deref(),
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
    /// log::error! 对 release 版用户完全不可见——曾因此无法定位「待办非空、
    /// 开关全开、retry 计数 = 0 却无任何可见错误」的静默不发请求故障。
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
