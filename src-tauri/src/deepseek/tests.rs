use super::calls::*;
use super::config::*;
use super::pipeline::*;
use super::transport::*;
use crate::db;
use crate::db::settings::{
    DEFAULT_DEEPSEEK_PROMPT_EDITABLE, KEY_DEEPSEEK_API_KEY, KEY_DEEPSEEK_BASE_URL,
    KEY_DEEPSEEK_ENABLED, KEY_DEEPSEEK_TRANSLATE_RELEASE, KEY_LANGUAGE,
};
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

/// `build_client` 失败必须写 DB 日志：log::error! 只进日志文件、不进应用内
/// 日志页，「待办非空、开关全开、retry 计数 = 0 却无任何可见错误」这类静默
/// 不发请求故障将无法定位。
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
        "build_client 失败应写入一条 ERROR DB 日志（应用内唯一可见的错误渠道）"
    );
    assert_eq!(
        translate_retry_count(&conn, id),
        0,
        "建 client 阶段失败不应递增 retry 计数（与发请求后的失败区分）"
    );
}
