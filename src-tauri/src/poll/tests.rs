use super::*;
use super::ai_jobs::*;
use super::check::*;
use super::settings::*;
use crate::db::settings::{KEY_FETCH_HISTORY, KEY_FETCH_HISTORY_COUNT};
use crate::adapters::github;
use crate::source;
use std::sync::Arc;

/// 已到点（包括系统休眠唤醒后挂钟已远超 target 的情形）应返回 `None`，
/// 让轮询立即执行而不是继续等。
#[test]
fn poll_wait_slice_none_when_due_or_overdue() {
    assert_eq!(poll_wait_slice(100, 100), None, "恰好到点应视为该检查");
    assert_eq!(
        poll_wait_slice(100, 100 + 7 * 24 * 3600),
        None,
        "挂钟已过期（休眠 7 天后唤醒）应视为该检查"
    );
}

/// 剩余时间超过一个分片时按分片截断：这是「休眠唤醒后最多再空等一个分片」
/// 的实现依据（若改回一次睡到底，本用例会挂）。
#[test]
fn poll_wait_slice_is_clamped_to_slice() {
    assert_eq!(
        poll_wait_slice(100 + 65 * 60, 100),
        Some(POLL_WAIT_SLICE),
        "65 分钟应被截断为一个分片"
    );
    assert_eq!(
        poll_wait_slice(100 + 60, 100),
        Some(POLL_WAIT_SLICE),
        "恰为一个分片时不应多切"
    );
    assert_eq!(
        poll_wait_slice(100 + 10, 100),
        Some(std::time::Duration::from_secs(10)),
        "不足一个分片时按剩余等待，不无谓拖长"
    );
}

#[test]
fn test_poll_lock_acquire_and_release() {
    let rt = PollRuntime::const_new();
    let g1 = rt.acquire_lock();
    assert!(g1.is_ok());
    assert!(rt.acquire_lock().is_err());
    drop(g1);
    assert!(rt.acquire_lock().is_ok());
}

#[test]
fn test_poll_guard_drop_releases_lock() {
    let rt = PollRuntime::const_new();
    {
        let _guard = rt.acquire_lock().unwrap();
        assert!(rt.lock.load(Ordering::Relaxed));
    }
    assert!(!rt.lock.load(Ordering::Relaxed));
}

#[test]
fn test_poll_runtime_instances_are_isolated() {
    let rt_a = PollRuntime::const_new();
    let rt_b = PollRuntime::const_new();
    let _g = rt_a.acquire_lock().unwrap();
    assert!(rt_b.acquire_lock().is_ok());
    rt_a.stop();
    assert!(!rt_a.is_running());
    assert!(rt_b.is_running());
}

#[test]
fn test_poll_running_flag() {
    RUNTIME.running.store(true, Ordering::SeqCst);
    assert!(is_poll_running());

    stop_poll();
    assert!(!is_poll_running());

    RUNTIME.running.store(true, Ordering::SeqCst);
}

use crate::db::init::init_memory_db;

#[test]
fn test_read_deepseek_config_defaults() {
    let conn = init_memory_db().unwrap();
    let cfg = deepseek::read_config(&conn);
    assert!(!cfg.enabled);
    assert_eq!(cfg.model, "deepseek-v4-flash");
    assert_eq!(cfg.base_url, "https://api.deepseek.com");
    assert!(cfg.api_key.is_none());
}

#[test]
fn test_read_deepseek_config_configured() {
    crate::crypto::set_test_master_key();
    let conn = init_memory_db().unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_MODEL, "deepseek-v4-pro").unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, "https://custom.api").unwrap();
    let encrypted = crate::crypto::encrypt("sk-test");
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_API_KEY, &encrypted).unwrap();

    let cfg = deepseek::read_config(&conn);
    assert!(cfg.enabled);
    assert_eq!(cfg.model, "deepseek-v4-pro");
    assert_eq!(cfg.base_url, "https://custom.api");
    assert_eq!(cfg.api_key.unwrap(), "sk-test");
}

use crate::http;

#[test]
fn test_build_http_client_empty() {
    let result = http::build_http_client(http::HttpClientConfig::default());
    assert!(result.is_ok());
}

#[test]
fn test_build_http_client_invalid() {
    let result = http::build_http_client(http::HttpClientConfig {
        proxy_url: "://invalid",
        proxy_mode: "custom",
        ..Default::default()
    });
    assert!(result.is_err());
    // 非法代理 URL 现在收敛到 net::ProxyPolicy 判定，返回 i18n 错误码
    assert_eq!(result.unwrap_err(), "err.invalid_url");
}

#[test]
fn test_build_http_client_valid() {
    let result = http::build_http_client(http::HttpClientConfig {
        proxy_url: "http://127.0.0.1:1080",
        proxy_mode: "custom",
        ..Default::default()
    });
    assert!(result.is_ok());
}

#[test]
fn test_build_http_client_none_ignores_proxy_url() {
    // mode=none 时传无效 URL 也不报错
    let result = http::build_http_client(http::HttpClientConfig {
        proxy_url: "://invalid",
        proxy_mode: "none",
        ..Default::default()
    });
    assert!(result.is_ok());
}

#[test]
fn test_build_http_client_system() {
    let result = http::build_http_client(http::HttpClientConfig {
        proxy_mode: "system",
        ..Default::default()
    });
    assert!(result.is_ok());
}

#[test]
fn test_build_deepseek_client_system_proxy() {
    let result = deepseek::build_client("sk-test", "", "system", 60);
    assert!(result.is_ok());
}

#[test]
fn test_build_deepseek_client_success() {
    let result = deepseek::build_client("sk-test", "", "none", 60);
    assert!(result.is_ok());
}

#[test]
fn test_build_deepseek_client_with_proxy() {
    let result = deepseek::build_client("sk-test", "http://127.0.0.1:1080", "custom", 60);
    assert!(result.is_ok());
}

#[test]
fn test_build_deepseek_client_invalid_key() {
    let result = deepseek::build_client("key\nwith\nnewlines", "", "none", 60);
    assert!(result.is_err());
}

// Simulates the startup logic in lib.rs
fn calc_next_poll(conn: &rusqlite::Connection) -> i64 {
    let now = chrono::Utc::now().timestamp();
    db::settings::get_setting(conn, KEY_NEXT_POLL_AT)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > now)
        .unwrap_or(now)
}

#[test]
fn test_next_poll_at_startup_restores_future() {
    let conn = init_memory_db().unwrap();
    let now = chrono::Utc::now().timestamp();
    db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
    // Saved next_poll_at is 20 min in the future → should be used as-is
    db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now + 20 * 60).to_string()).unwrap();
    let result = calc_next_poll(&conn);
    assert_eq!(result - now, 20 * 60);
}

#[test]
fn test_next_poll_at_startup_falls_back_when_expired() {
    let conn = init_memory_db().unwrap();
    let now = chrono::Utc::now().timestamp();
    db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "30").unwrap();
    // Saved next_poll_at is 10 min in the past → expired, fallback to now → immediate check
    db::settings::set_setting(&conn, KEY_NEXT_POLL_AT, &(now - 10 * 60).to_string()).unwrap();
    let result = calc_next_poll(&conn);
    assert_eq!(result, now);
}

#[test]
fn test_acquire_lock_succeeds_and_fails() {
    // 验证 pub(crate) acquire_lock API
    RUNTIME.lock.store(false, Ordering::Release);

    // 第一次获取应成功
    let guard1 = acquire_lock();
    assert!(guard1.is_ok(), "first acquire should succeed");
    assert!(RUNTIME.lock.load(Ordering::Relaxed));

    // 第二次获取应失败（锁已被持有）
    let guard2 = acquire_lock();
    assert!(guard2.is_err(), "second acquire should fail while locked");

    // 释放第一个 guard
    drop(guard1);
    assert!(!RUNTIME.lock.load(Ordering::Relaxed));

    // 再次获取应成功
    let guard3 = acquire_lock();
    assert!(guard3.is_ok(), "acquire should succeed after release");

    // 清理
    drop(guard3);
    RUNTIME.lock.store(false, Ordering::Release);
}

#[test]
fn test_next_poll_at_startup_falls_back_when_missing() {
    let conn = init_memory_db().unwrap();
    let now = chrono::Utc::now().timestamp();
    db::settings::set_setting(&conn, KEY_POLL_INTERVAL, "45").unwrap();
    // No saved value → fallback to now → immediate check
    let result = calc_next_poll(&conn);
    assert_eq!(result, now);
}

// --- compute_fetch_plan tests ---

#[test]
fn test_compute_plan_no_history() {
    // fetch_history=false → 增量分支：每轮不截断写入（(None, 10, false)）。
    // 曾经这里返回 Some(count)，把「历史版本数量」兼任每轮上限 → 同一周期内
    // 第二条起的新版本全部不入库。
    let (max, per_page, paginate) = compute_fetch_plan(false, true, 50);
    assert_eq!(max, None);
    assert_eq!(per_page, 10);
    assert!(!paginate);
}

#[test]
fn test_compute_plan_not_first_query() {
    // !is_first_query → 增量分支，同样不受 fetch_history_count 约束
    let (max, per_page, paginate) = compute_fetch_plan(true, false, 50);
    assert_eq!(max, None);
    assert_eq!(per_page, 10);
    assert!(!paginate);
}

#[test]
fn test_compute_plan_count_0() {
    // count=0 → first branch: None = no limit, paginate = true
    let (max, per_page, paginate) = compute_fetch_plan(true, true, 0);
    assert_eq!(max, None);
    assert_eq!(per_page, 100);
    assert!(paginate);
}

#[test]
fn test_compute_plan_count_1() {
    // count=1 → second branch: small single-page fetch
    let (max, per_page, paginate) = compute_fetch_plan(true, true, 1);
    assert_eq!(max, Some(1));
    // per_page = clamp(1+5, 10, 100) = 10
    assert_eq!(per_page, 10);
    assert!(!paginate);
}

#[test]
fn test_compute_plan_count_50() {
    // count=50 → second branch
    let (max, per_page, paginate) = compute_fetch_plan(true, true, 50);
    assert_eq!(max, Some(50));
    // per_page = clamp(50+5, 10, 100) = 55
    assert_eq!(per_page, 55);
    assert!(!paginate);
}

#[test]
fn test_compute_plan_count_95() {
    // count=95 → second branch, per_page capped at 100
    let (max, per_page, paginate) = compute_fetch_plan(true, true, 95);
    assert_eq!(max, Some(95));
    // per_page = clamp(95+5, 10, 100) = 100
    assert_eq!(per_page, 100);
    assert!(!paginate);
}

#[test]
fn test_compute_plan_count_100() {
    // count=100 → second branch (since 100 > 100 is false)
    let (max, per_page, paginate) = compute_fetch_plan(true, true, 100);
    assert_eq!(max, Some(100));
    // per_page = clamp(100+5, 10, 100) = 100
    assert_eq!(per_page, 100);
    assert!(!paginate);
}

#[test]
fn test_compute_plan_count_101() {
    // count=101 → first branch, paginated with limit (safety net)
    let (max, per_page, paginate) = compute_fetch_plan(true, true, 101);
    assert_eq!(max, Some(101));
    assert_eq!(per_page, 100);
    assert!(paginate);
}

#[test]
fn test_uncounted_failure_covers_rate_limit_and_save_timeout() {
    use crate::error::{SourceError, SourceErrorKind};
    for err in [
        SourceError::rate_limit(429),
        SourceError::rate_limit_reset(403, 120),
        SourceError::credential_invalid(401),
        SourceError::save_timeout(300),
        SourceError::new(
            403,
            SourceErrorKind::YoutubeApiQuota {
                detail: Some("quota".into()),
            },
        ),
        SourceError::new(429, SourceErrorKind::BiliRateLimit),
    ] {
        assert!(err.is_uncounted_failure(), "{} 应豁免断路器", err);
    }
    for err in [
        SourceError::api_error(403, "Forbidden"),
        SourceError::api_error(404, "Not Found"),
        SourceError::source_timeout(300),
    ] {
        assert!(!err.is_uncounted_failure(), "{} 应计入断路器", err);
    }
}

struct StubAdapter {
    windows: std::sync::Mutex<Vec<Vec<(&'static str, &'static str)>>>,
}

#[async_trait::async_trait]
impl source::SourceAdapter for StubAdapter {
    fn source_type(&self) -> &'static str {
        "github"
    }

    async fn fetch(
        &self,
        _client: &reqwest::Client,
        _source: &db::sources::Source,
        _per_page: usize,
        _token: Option<&str>,
    ) -> Result<Vec<source::FetchedRelease>, crate::error::SourceError> {
        Ok(self
            .windows
            .lock()
            .unwrap()
            .remove(0)
            .into_iter()
            .map(|(tag, published)| source::FetchedRelease {
                tag: tag.to_string(),
                name: tag.to_string(),
                html_url: format!("https://x/{}", tag),
                published: published.to_string(),
                prerelease: false,
                body: None,
                extra: source::SourceExtraMetadata::None,
            })
            .collect())
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &db::sources::Source,
        _max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<source::FetchedRelease>, crate::error::SourceError> {
        self.fetch(client, source, 0, token).await
    }

    async fn save(
        &self,
        db_pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
        source: &db::sources::Source,
        data: &[source::FetchedRelease],
        max_count: usize,
        _client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let entries: Vec<db::save::SaveEntry> = data
            .iter()
            .map(source::FetchedRelease::to_save_entry)
            .collect();
        db::save::save_entries_generic(
            &conn,
            source.id,
            &entries,
            max_count,
            |_, _, _| {},
            |_, _, _| {},
        )
    }

    async fn verify_and_describe(
        &self,
        _client: &reqwest::Client,
        _owner: &str,
        _repo: &str,
        _token: Option<&str>,
    ) -> Result<String, crate::error::SourceError> {
        Ok("stub".to_string())
    }
}

/// 回归（核心缺陷）：同一轮询周期内发布多条时，上一轮已抓走最新那条，本轮首条即
/// 去重命中——旧实现在此返回空集，较旧的新条目**每轮都排在已知条目之后**，永不入库。
/// 走真实编排链路（fetch → save → post_save）验证缺口会被自动补上。
#[tokio::test]
async fn check_one_source_backfills_releases_published_in_same_interval() {
    let pool = db::init::init_memory_pool().unwrap();
    let source_id = {
        let conn = pool.get().unwrap();
        db::sources::add_source(&conn, "github", "o", "r", "").unwrap()
    };
    let adapter = StubAdapter {
        windows: std::sync::Mutex::new(vec![
            // 第 1 轮：该周期只发布了 v3
            vec![("v3", "2024-01-03T00:00:00Z")],
            // 第 2 轮：v2/v1 其实与 v3 同属一个周期，只是排在已入库的 v3 之后
            vec![
                ("v3", "2024-01-03T00:00:00Z"),
                ("v2", "2024-01-02T00:00:00Z"),
                ("v1", "2024-01-01T00:00:00Z"),
            ],
        ]),
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let expected_new = [1usize, 2usize];
    for (round, expect_new) in expected_new.iter().enumerate() {
        let source = {
            let conn = pool.get().unwrap();
            db::sources::list_sources(&conn)
                .unwrap()
                .into_iter()
                .find(|s| s.id == source_id)
                .unwrap()
        };
        let ctx = CheckCtx {
            db_pool: &pool,
            adapter: &adapter,
            client: &client,
            token: None,
            fetch_history: false,
            fetch_history_count: 1,
            log_key: if round == 0 { "check.manual" } else { "check.auto" },
        };
        let outcome = check_one_source(&ctx, &source)
            .await
            .expect("本轮检查应成功");
        assert_eq!(
            outcome.new_ids.len(),
            *expect_new,
            "第 {} 轮新增条数不符",
            round + 1
        );
    }

    let conn = pool.get().unwrap();
    let mut tags: Vec<String> = db::releases::get_releases_with_state(&conn)
        .unwrap()
        .into_iter()
        .map(|r| r.tag_name)
        .collect();
    tags.sort();
    assert_eq!(tags, vec!["v1", "v2", "v3"], "同一周期内的多条版本都必须入库");
}

// --- credential 读取（经 credential::read_credential）---

#[test]
fn test_get_github_token_no_key_returns_none() {
    let conn = init_memory_db().unwrap();
    assert!(
        crate::credential::read_credential(&conn, crate::db::settings::KEY_GITHUB_TOKEN).is_none(),
        "未设置 token 时应返回 None"
    );
}

#[test]
fn test_get_github_token_empty_string_returns_none() {
    let conn = init_memory_db().unwrap();
    crate::crypto::set_test_master_key();
    db::settings::set_setting(&conn, crate::db::settings::KEY_GITHUB_TOKEN, "").unwrap();
    assert!(
        crate::credential::read_credential(&conn, crate::db::settings::KEY_GITHUB_TOKEN).is_none(),
        "空字符串 token 时应返回 None"
    );
}

#[test]
fn test_get_github_token_valid_returns_decrypted() {
    let conn = init_memory_db().unwrap();
    crate::crypto::set_test_master_key();
    let encrypted = crate::crypto::encrypt("ghp_test_token");
    db::settings::set_setting(&conn, crate::db::settings::KEY_GITHUB_TOKEN, &encrypted).unwrap();
    let result = crate::credential::read_credential(&conn, crate::db::settings::KEY_GITHUB_TOKEN);
    assert_eq!(result.as_deref(), Some("ghp_test_token"), "应解密返回原始 token");
}

// --- load_poll_settings tests ---

#[test]
fn test_load_poll_settings_defaults() {
    let conn = init_memory_db().unwrap();
    let settings = load_poll_settings(&conn).unwrap();
    assert_eq!(settings.proxy.url, "");
    assert_eq!(settings.proxy.mode, "none");
    assert!(settings.credentials.get(source::AuthKind::GitHubToken).is_none());
    assert!(!settings.fetch_history);
    assert_eq!(settings.fetch_history_count, 1);
}

#[test]
fn test_load_poll_settings_configured() {
    let conn = init_memory_db().unwrap();
    crate::crypto::set_test_master_key();

    db::settings::set_setting(&conn, db::settings::KEY_PROXY_URL, "http://127.0.0.1:1080").unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_PROXY_MODE, "custom").unwrap();
    let encrypted = crate::crypto::encrypt("ghp_configured");
    db::settings::set_setting(&conn, crate::db::settings::KEY_GITHUB_TOKEN, &encrypted).unwrap();
    db::settings::set_setting(&conn, KEY_FETCH_HISTORY, "true").unwrap();
    db::settings::set_setting(&conn, KEY_FETCH_HISTORY_COUNT, "5").unwrap();

    let settings = load_poll_settings(&conn).unwrap();
    assert_eq!(settings.proxy.url, "http://127.0.0.1:1080");
    assert_eq!(settings.proxy.mode, "custom");
    assert_eq!(
        settings.credentials.get(source::AuthKind::GitHubToken),
        Some("ghp_configured")
    );
    assert!(settings.fetch_history);
    assert_eq!(settings.fetch_history_count, 5);
}

#[test]
fn test_load_poll_settings_proxy_mode_fallback() {
    // proxy_url=非空但 proxy_mode 未设置 → fallback 为 "custom"
    let conn = init_memory_db().unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_PROXY_URL, "http://proxy:8080").unwrap();
    let settings = load_poll_settings(&conn).unwrap();
    assert_eq!(settings.proxy.mode, "custom");
}

// --- mark_older_as_read tests ---

#[test]
fn test_mark_older_as_read_empty_saved_is_noop() {
    let conn = init_memory_db().unwrap();
    mark_older_as_read(&conn, 1, &[]);
    // 不应 panic，不应改变任何状态
    let logs = db::logs::get_logs(&conn, 10).unwrap();
    assert!(logs.is_empty());
}

#[test]
fn test_mark_older_as_read_skips_latest_when_no_newer() {
    let conn = init_memory_db().unwrap();
    let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
    let r1 = db::releases::insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
    let r2 = db::releases::insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();

    // saved 按 published_at 降序排列（最新在前）
    let saved = vec![(r2, Some("v2.0".into())), (r1, Some("v1.0".into()))];
    mark_older_as_read(&conn, sid, &saved);

    // 最新（v2.0）应保持 pending，v1.0 应变为 clicked
    let releases = db::releases::get_releases_with_state(&conn).unwrap();
    let r1_state = releases.iter().find(|r| r.id == r1).unwrap();
    let r2_state = releases.iter().find(|r| r.id == r2).unwrap();
    assert_eq!(r2_state.notification_status, "pending", "最新 release 应保持 pending");
    assert_eq!(r1_state.notification_status, "clicked", "旧 release 应标记为已读");
}

#[test]
fn test_mark_older_as_read_all_when_newer_exists() {
    let conn = init_memory_db().unwrap();
    let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

    // 先在 DB 中创建一个更新的 release
    let newer_id = db::releases::insert_release(&conn, sid, "v3.0", "R3", "https://x", "2024-01-03T00:00:00Z", false, None).unwrap();

    let r1 = db::releases::insert_release(&conn, sid, "v1.0", "R1", "https://x", "2024-01-01T00:00:00Z", false, None).unwrap();
    let r2 = db::releases::insert_release(&conn, sid, "v2.0", "R2", "https://x", "2024-01-02T00:00:00Z", false, None).unwrap();

    // 只传 r1, r2 进来（r3 已存在于 DB）
    let saved = vec![(r2, Some("v2.0".into())), (r1, Some("v1.0".into()))];
    mark_older_as_read(&conn, sid, &saved);

    // r3 已有更新版本在 DB 中，所以 r1, r2 都应标记为 clicked
    let releases = db::releases::get_releases_with_state(&conn).unwrap();
    let newer = releases.iter().find(|r| r.id == newer_id).unwrap();
    let r1_state = releases.iter().find(|r| r.id == r1).unwrap();
    let r2_state = releases.iter().find(|r| r.id == r2).unwrap();
    assert_eq!(newer.notification_status, "pending", "完全独立的更新 release 不受影响");
    assert_eq!(r1_state.notification_status, "clicked", "有更新版本时所有 saved release 都应标记");
    assert_eq!(r2_state.notification_status, "clicked", "有更新版本时所有 saved release 都应标记");
}

// ── source 分发测试 ──

fn gh_release(tag: &str, date: &str, body: Option<&str>) -> source::FetchedRelease {
    source::FetchedRelease {
        tag: tag.to_string(),
        name: tag.to_string(),
        html_url: format!("https://github.com/o/r/releases/tag/{}", tag),
        published: date.to_string(),
        prerelease: false,
        body: body.map(|s| s.to_string()),
        extra: source::SourceExtraMetadata::None,
    }
}

/// github 入库逻辑由 `github::save_releases` 覆盖（trait 实现内部调它）。
/// 此测试验证 max_count 截断行为。
#[test]
fn test_github_save_respects_max_count() {
    let conn = init_memory_db().unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
    let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();

    let data = vec![
        gh_release("v3.0", "2024-03-01T00:00:00Z", Some("v3")),
        gh_release("v2.0", "2024-02-01T00:00:00Z", Some("v2")),
        gh_release("v1.0", "2024-01-01T00:00:00Z", Some("v1")),
    ];

    // max_count=2 应只保存最新两条
    let saved = github::save_releases(&conn, sid, &data, 2);

    assert_eq!(saved.len(), 2, "max_count=2 应只保存 2 条");
    assert_eq!(saved[0].1.as_deref(), Some("v3"));
    assert_eq!(saved[1].1.as_deref(), Some("v2"));
}

/// 不支持的 source_type：`get_adapter` 返回 `err.unsupported_source`。
#[test]
fn test_get_adapter_unsupported_type_errors() {
    match source::get_adapter("gitlab") {
        Ok(_) => panic!("不支持的源类型应返回错误"),
        Err(e) => {
            assert_eq!(e.status(), 0);
            assert!(
                e.to_ipc_string().contains("err.unsupported_source"),
                "错误信息: {}",
                e
            );
        }
    }
}

/// github / huggingface 应能取得适配器。
#[test]
fn test_get_adapter_supported_types() {
    assert!(source::get_adapter("github").is_ok());
    assert!(source::get_adapter("huggingface").is_ok());
}

// ── 编排链路集成测试：fetch→save→mark_read→record ──────
//
// 用真实的 github fetch + db 层串起两条调用路径共用的核心链路
// （post_save_mark_record），锁住其行为。

fn make_source(conn: &rusqlite::Connection, owner: &str, repo: &str) -> db::sources::Source {
    let id = db::sources::add_source(conn, "github", owner, repo, "").unwrap();
    db::sources::get_source(conn, id).unwrap().unwrap()
}

// 说明：`sources.poll_interval_minutes` 是死字段（无读取方），调度只读全局周期，
// 故此处**刻意没有**按源间隔调度的用例（「未设置」约定见 `db::init` Migration 20）。

// ── 断路器：不计入失败分类 + 整轮回退 ─────────────────

#[test]
fn is_uncounted_failure_matches_account_level_errors() {
    use crate::error::{SourceError, SourceErrorKind};
    assert!(SourceError::new(403, SourceErrorKind::YoutubeApiQuota { detail: Some("quotaExceeded".into()) }).is_uncounted_failure());
    assert!(SourceError::new(400, SourceErrorKind::YoutubeApiKeyInvalid { detail: Some("keyInvalid".into()) }).is_uncounted_failure());
    assert!(SourceError::new(429, SourceErrorKind::BiliRateLimit).is_uncounted_failure());
    assert!(SourceError::new(403, SourceErrorKind::BiliRisk { detail: None }).is_uncounted_failure());
    assert!(!SourceError::api_error(500, "boom").is_uncounted_failure());
    assert!(!SourceError::source_timeout(300).is_uncounted_failure());
    assert!(!SourceError::coded(404, "err.repo_not_found", ["404"]).is_uncounted_failure());
    assert!(!SourceError::coded(404, "err.youtube_channel_not_found", ["x"]).is_uncounted_failure());
}

fn outcome(id: i64, failed: bool) -> TaskOutcome {
    TaskOutcome { source_id: id, failed, checked: true, new_ids: vec![], saved: vec![] }
}

/// panic / 取消的失败留痕必须**双写**：源健康状态置 error + 日志页留一条。
/// 只写 sources 的话，用户在源列表看到红字却在日志页查不到记录，与其他所有失败
/// 路径（都走 `record_check_failure` 双写）口径不一致。
#[tokio::test]
async fn record_task_panic_writes_source_and_log() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let id = {
        let conn = pool.get().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_LANGUAGE, "zh-CN").unwrap();
        db::sources::add_source(&conn, "github", "o", "r", "").unwrap()
    };

    record_task_panic(&pool, id).await;

    let conn = pool.get().unwrap();
    let source = db::sources::get_source(&conn, id).unwrap().unwrap();
    assert_eq!(source.last_check_status, "error", "panic 轮必须记失败，否则健康灯常绿");
    assert_eq!(source.last_check_message.as_deref(), Some("err.check_task_panic"));
    assert_eq!(
        source.consecutive_failures, 1,
        "panic 计入断路器（整轮同时挂掉由 mass-failure guard 回退）"
    );

    let logged = db::logs::get_logs(&conn, 100).unwrap_or_default();
    let entry = logged
        .iter()
        .find(|l| l.message_key.as_deref() == Some("err.check_task_panic"))
        .expect("日志页必须能看到本条——只写 sources 表会在这里断");
    assert_eq!(entry.level, "ERROR");
    assert!(
        entry.rendered_message.as_deref().unwrap_or_default().contains("异常终止"),
        "rendered_message 应是 i18n 渲染文本而非裸 key: {:?}",
        entry.rendered_message
    );
}

/// 整轮全挂（网络/代理层抖动）→ 回退本轮累加，源不会被逐个自动禁用。
/// 现网实测过的场景：10 个源在同一秒报同一个 err.source_timeout|300。
#[tokio::test]
async fn mass_failure_guard_rolls_back_whole_round() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let ids: Vec<i64> = {
        let conn = pool.get().unwrap();
        (0..4)
            .map(|i| {
                let id = db::sources::add_source(&conn, "github", &format!("o{i}"), "r", "").unwrap();
                db::sources::record_check_failure(&conn, id, "err.source_timeout|300").unwrap();
                id
            })
            .collect()
    };

    let outcomes: Vec<TaskOutcome> = ids.iter().map(|id| outcome(*id, true)).collect();
    apply_mass_failure_guard(&pool, &outcomes).await;

    let conn = pool.get().unwrap();
    for id in &ids {
        let s = db::sources::get_source(&conn, *id).unwrap().unwrap();
        assert_eq!(s.consecutive_failures, 0, "整轮全挂不应累加连续失败（否则 3 轮抖动即全禁用）");
        assert_eq!(s.last_check_status, "error", "状态仍应保留失败标记，源列表照旧提示");
    }
    let logged = db::logs::get_logs(&conn, 100).unwrap_or_default();
    assert!(
        logged.iter().any(|l| l.message_key.as_deref() == Some("check.round_failed")),
        "整轮回退应留一条 check.round_failed 日志"
    );
}

/// 单源失败（比例未达阈、或源数不足）→ 照旧累加，真坏源仍会被自动禁用。
#[tokio::test]
async fn mass_failure_guard_keeps_single_source_failures() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let ids: Vec<i64> = {
        let conn = pool.get().unwrap();
        (0..4)
            .map(|i| {
                let id = db::sources::add_source(&conn, "github", &format!("o{i}"), "r", "").unwrap();
                db::sources::record_check_failure(&conn, id, "err.api_error|500|x").unwrap();
                id
            })
            .collect()
    };

    let mixed = vec![
        outcome(ids[0], true),
        outcome(ids[1], false),
        outcome(ids[2], false),
        outcome(ids[3], false),
    ];
    apply_mass_failure_guard(&pool, &mixed).await;

    let conn = pool.get().unwrap();
    assert_eq!(
        db::sources::get_source(&conn, ids[0]).unwrap().unwrap().consecutive_failures,
        1,
        "25% 失败不应被当成网络层抖动"
    );

    // 只有 1 个源时 100% 失败也不豁免（单源用户的失败就是单源问题）
    let single = vec![outcome(ids[1], true)];
    apply_mass_failure_guard(&pool, &single).await;
    assert_eq!(
        db::sources::get_source(&conn, ids[1]).unwrap().unwrap().consecutive_failures,
        1,
        "源数不足阈值时不走整轮回退"
    );
}

/// 模拟 poll.rs 编排主链路：
/// 1. 从远程拉取（这里用构造的 JSON 代替 HTTP，聚焦 save→mark→record 这段）
/// 2. github::save_releases 入库（trait 实现内部调它）
/// 3. mark_older_as_read 标记非最新
/// 4. record_check_success 更新源健康状态
#[test]
fn test_pipeline_single_new_release_stays_pending() {
    let conn = init_memory_db().unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
    let source = make_source(&conn, "o", "r");

    let fetched = vec![gh_release("v1.0", "2024-01-01T00:00:00Z", Some("body"))];
    let saved = github::save_releases(&conn, source.id, &fetched, 1);
    assert_eq!(saved.len(), 1);
    let (_ids, saved) = post_save_mark_record(&conn, source.id, &saved);
    assert_eq!(saved.len(), 1);

    // 唯一的新版本应保持 pending，可被通知
    let releases = db::releases::get_releases_with_state(&conn).unwrap();
    assert_eq!(releases[0].notification_status, "pending");

    // 源健康状态更新为 ok，new_count=1
    let s = db::sources::get_source(&conn, source.id).unwrap().unwrap();
    assert_eq!(s.last_check_status, "ok");
    assert_eq!(s.last_new_count, 1);
    assert_eq!(s.consecutive_failures, 0);
}

#[test]
fn test_pipeline_multiple_new_only_latest_stays_pending() {
    let conn = init_memory_db().unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
    let source = make_source(&conn, "o", "r");

    // 首次拉取 3 个新版本（历史模式 max_count=3）
    let fetched = vec![
        gh_release("v3.0", "2024-03-01T00:00:00Z", Some("v3")),
        gh_release("v2.0", "2024-02-01T00:00:00Z", Some("v2")),
        gh_release("v1.0", "2024-01-01T00:00:00Z", Some("v1")),
    ];
    let saved = github::save_releases(&conn, source.id, &fetched, 3);
    assert_eq!(saved.len(), 3);
    let (_ids, saved) = post_save_mark_record(&conn, source.id, &saved);
    assert_eq!(saved.len(), 3);

    // 只有最新 v3.0 保持 pending，v2/v1 被标记为 clicked
    let releases = db::releases::get_releases_with_state(&conn).unwrap();
    let status_of = |tag: &str| -> String {
        releases.iter().find(|r| r.tag_name == tag).unwrap().notification_status.clone()
    };
    assert_eq!(status_of("v3.0"), "pending");
    assert_eq!(status_of("v2.0"), "clicked");
    assert_eq!(status_of("v1.0"), "clicked");
}

#[test]
fn test_pipeline_subsequent_poll_with_no_new_release() {
    let conn = init_memory_db().unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
    let source = make_source(&conn, "o", "r");

    // 第一轮：拉到 v1.0
    let fetched1 = vec![gh_release("v1.0", "2024-01-01T00:00:00Z", Some("v1"))];
    let saved = github::save_releases(&conn, source.id, &fetched1, 1);
    let (_ids, _saved) = post_save_mark_record(&conn, source.id, &saved);

    // 第二轮：同样的数据，save 返回空（已入库），但源健康状态仍应更新
    let saved2 = github::save_releases(&conn, source.id, &fetched1, 1);
    assert!(saved2.is_empty(), "重复数据不应再次保存");
    let (_ids2, _saved2) = post_save_mark_record(&conn, source.id, &saved2);

    let s = db::sources::get_source(&conn, source.id).unwrap().unwrap();
    assert_eq!(s.last_check_status, "ok");
    assert_eq!(s.last_new_count, 0, "无新版本时 last_new_count 应为 0");

    // release 数量不变，状态不变
    let releases = db::releases::get_releases_with_state(&conn).unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].notification_status, "pending");
}

#[test]
fn test_pipeline_failure_records_error_without_saving() {
    let conn = init_memory_db().unwrap();
    let source = make_source(&conn, "o", "r");

    // 模拟 check 失败：不 save，直接 record_check_failure（与 poll.rs 失败分支一致）
    let _ = db::sources::record_check_failure(&conn, source.id, "err.api_error|503|Service Unavailable");

    let s = db::sources::get_source(&conn, source.id).unwrap().unwrap();
    assert_eq!(s.last_check_status, "error");
    assert_eq!(s.consecutive_failures, 1);
    assert_eq!(s.last_new_count, 0);
    assert!(s.last_check_message.as_deref().unwrap().contains("503"));

    // 无 release 被保存
    assert!(db::releases::get_releases_with_state(&conn).unwrap().is_empty());
}

// ── collect_pending_and_notify 编排链路注入式测试 ──
//
// 直接测真实的私有 async fn collect_pending_and_notify，注入 NoopEmitter
// + init_memory_pool，覆盖「通知派发次数 / muted 源跳过 / new_ids 过滤」状态机。

/// 单条 pending release：通知派发 1 次，new_releases 含该条。
#[tokio::test]
async fn test_collect_pending_notifies_single_release() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let saved = {
        let conn = pool.get().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source = make_source(&conn, "o", "r");
        github::save_releases(&conn, source.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("b"))], 1)
    };
    assert_eq!(saved.len(), 1);
    let new_ids: Vec<i64> = saved.iter().map(|(id, _)| *id).collect();

    let emitter = crate::types::NoopEmitter::new();
    let (all_pending, new_releases) =
        collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;

    assert_eq!(emitter.call_count(), 1, "单条 pending 应派发 1 次通知");
    assert_eq!(new_releases.len(), 1, "new_ids 全包含 → new_releases 含该条");
    // 标记 last_notified_at 后该 release 不再未读pending（only_notified_missing=true），
    // 故 all_pending（末尾重读）应为空——这是 collect 真实行为。
    assert!(all_pending.is_empty(), "已通知的 release 应从 unread pending 列表移除");
}

/// 静音源：其 pending release 不派发通知，但仍计入 new_releases 过滤结果。
#[tokio::test]
async fn test_collect_pending_skips_muted_source() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let (id_a, id_b) = {
        let conn = pool.get().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source_a = make_source(&conn, "oa", "ra");
        let source_b = make_source(&conn, "ob", "rb");
        let a = github::save_releases(&conn, source_a.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("a"))], 1);
        let b = github::save_releases(&conn, source_b.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("b"))], 1);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        db::sources::set_source_muted(&conn, source_a.id, true).unwrap();
        (a[0].0, b[0].0)
    };
    let new_ids = vec![id_a, id_b];

    let emitter = crate::types::NoopEmitter::new();
    let (_all_pending, new_releases) =
        collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;

    assert_eq!(emitter.call_count(), 1, "静音源不派发通知，仅 source_b 通知");
    assert_eq!(new_releases.len(), 2, "muted 不剔除 new_releases（仅跳过通知）");
}

/// 视频源（YouTube/B 站）通知参数：标题用频道名/UP 主名（description），
/// tag（videoId/bvid）置空只留视频标题——与列表页可读性修复保持一致。
#[tokio::test]
async fn test_collect_pending_video_sources_readable_notification() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let (yt_id, bili_id) = {
        let conn = pool.get().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let yt = db::sources::add_source(&conn, "youtube", "UCabc123", "", "某频道名").unwrap();
        let bili = db::sources::add_source(&conn, "bilibili", "123456", "", "某UP主").unwrap();
        let yt_id = db::releases::insert_release(
            &conn, yt, "videoId123", "某视频标题",
            "https://youtube.com/watch?v=videoId123", "2024-01-01T00:00:00Z", false, None,
        ).unwrap();
        let bili_id = db::releases::insert_release(
            &conn, bili, "BV1xx411c7mD", "B站视频标题",
            "https://www.bilibili.com/video/BV1xx411c7mD", "2024-01-02T00:00:00Z", false, None,
        ).unwrap();
        (yt_id, bili_id)
    };
    let new_ids = vec![yt_id, bili_id];

    let emitter = crate::types::NoopEmitter::new();
    let (_all_pending, new_releases) =
        collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;
    assert_eq!(new_releases.len(), 2);

    let params = emitter.params();
    assert_eq!(params.len(), 2);
    // YouTube：标题为「来源标签 / 频道名」（非 channel_id），正文仅视频标题（无 videoId）
    let yt = params
        .iter()
        .find(|p| p.owner == "YouTube / 某频道名")
        .expect("YT 通知应带来源标签与频道名");
    assert_eq!(yt.tag, "");
    assert_eq!(yt.name, "某视频标题");
    assert_eq!(crate::notify::notification_title(&yt.owner, &yt.repo), "YouTube / 某频道名");
    assert_eq!(crate::notify::notification_body(&yt.tag, &yt.name, None), "某视频标题");
    // B 站：标题为「哔哩哔哩 / UP主名」（非 UID），正文仅视频标题（无 bvid）
    let bili = params
        .iter()
        .find(|p| p.owner == "哔哩哔哩 / 某UP主")
        .expect("B 站通知应带来源标签与 UP 主名");
    assert_eq!(bili.tag, "");
    assert_eq!(bili.name, "B站视频标题");
    assert_eq!(
        crate::notify::notification_title(&bili.owner, &bili.repo),
        "哔哩哔哩 / 某UP主"
    );
    assert_eq!(crate::notify::notification_body(&bili.tag, &bili.name, None), "B站视频标题");
}

/// GitHub（默认能力）：通知标题仍为 owner / repo，正文保留 tag - name。
#[tokio::test]
async fn test_collect_pending_github_notification_keeps_tag() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let id = {
        let conn = pool.get().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source = make_source(&conn, "o", "r");
        github::save_releases(&conn, source.id, &[gh_release("v1", "2024-01-01T00:00:00Z", Some("b"))], 1)[0].0
    };

    let emitter = crate::types::NoopEmitter::new();
    let _ = collect_pending_and_notify(&pool, &emitter, &[id], false).await;

    let params = emitter.params();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].owner, "o");
    assert_eq!(params[0].repo, "r");
    assert_eq!(params[0].tag, "v1");
    assert_eq!(crate::notify::notification_title(&params[0].owner, &params[0].repo), "o / r");
    assert_eq!(crate::notify::notification_body(&params[0].tag, &params[0].name, None), "v1 - v1");
}

/// 通知派发阶段对全部 pending 都触发；new_releases 仅按 new_ids 子集过滤。
#[tokio::test]
async fn test_collect_pending_new_ids_subset_filters_result() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let id_v2 = {
        let conn = pool.get().unwrap();
        db::settings::set_setting(&conn, db::settings::KEY_CHECK_PRERELEASES, "false").unwrap();
        let source = make_source(&conn, "o", "r");
        // 直接 save_releases（不走 post_save_mark_record），两条都保持 pending
        let saved = github::save_releases(
            &conn,
            source.id,
            &[
                gh_release("v2", "2024-03-01T00:00:00Z", Some("v2")),
                gh_release("v1", "2024-02-01T00:00:00Z", Some("v1")),
            ],
            2,
        );
        assert_eq!(saved.len(), 2);
        saved[0].0
    };
    // new_ids 只含 v2 → new_releases 只返回 v2，但通知阶段对两条 pending 都派发
    let new_ids = vec![id_v2];

    let emitter = crate::types::NoopEmitter::new();
    let (_all_pending, new_releases) =
        collect_pending_and_notify(&pool, &emitter, &new_ids, false).await;

    assert_eq!(emitter.call_count(), 2, "通知派发对全部 pending 触发，与 new_ids 无关");
    assert_eq!(new_releases.len(), 1, "new_releases 按 new_ids 子集过滤后仅含 v2");
    assert_eq!(new_releases[0].tag_name, "v2");
}

// ── filter_ai_eligible：跳过 ai_eligible=false 源（youtube/bilibili）的 AI 摘要/翻译 ──

#[tokio::test]
async fn test_filter_ai_eligible_keeps_only_eligible_types() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let (gh_id, yt_id, bl_id) = {
        let conn = pool.get().unwrap();
        let gh = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        let yt = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let bl = db::sources::add_source(&conn, "bilibili", "12345678", "", "").unwrap();
        let gh_id = db::releases::insert_release(&conn, gh, "v1", "R", "https://x", "2024-01-01T00:00:00Z", false, Some("gh")).unwrap();
        let yt_id = db::releases::insert_release(&conn, yt, "vid1", "V", "https://y", "2024-01-02T00:00:00Z", false, Some("yt")).unwrap();
        let bl_id = db::releases::insert_release(&conn, bl, "BV1xx", "V", "https://b", "2024-01-03T00:00:00Z", false, Some("bili")).unwrap();
        (gh_id, yt_id, bl_id)
    };

    let saved = vec![
        (gh_id, Some("gh".into())),
        (yt_id, Some("yt".into())),
        (bl_id, Some("bili".into())),
    ];
    let filtered = filter_ai_eligible(&pool, &saved).await.unwrap();
    assert_eq!(filtered.len(), 1, "youtube/bilibili 源条目应被过滤掉");
    assert_eq!(filtered[0].0, gh_id);
}

#[tokio::test]
async fn test_filter_ai_eligible_all_youtube_empty() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let yt_id = {
        let conn = pool.get().unwrap();
        let yt = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        db::releases::insert_release(&conn, yt, "vid1", "V", "https://y", "2024-01-01T00:00:00Z", false, Some("yt")).unwrap()
    };
    let filtered = filter_ai_eligible(&pool, &[(yt_id, Some("yt".into()))])
        .await
        .unwrap();
    assert!(filtered.is_empty(), "纯 youtube 条目应全部过滤");
}

#[tokio::test]
async fn test_filter_ai_eligible_empty_input() {
    let pool = crate::db::init::init_memory_pool().unwrap();
    assert!(filter_ai_eligible(&pool, &[]).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_filter_ai_eligible_db_error_fails_closed() {
    // 占住唯一连接使后续 get() 超时失败 → 必须返回 Err（fail-closed），
    // 绝不能降级为空集把 youtube 条目放行给 DeepSeek。
    use crate::db::SqliteConnectionManager;
    let manager = SqliteConnectionManager::memory();
    // connection_timeout 同时约束两处 get：①占连接的 get（若 refill 线程
    // 还没建好连接，需在窗口内等到）；②filter 内部那个**期望失败**的 get
    // （连接已被占住，等再久也拿不到，窗口只影响它报 Err 的延迟）。
    // 曾用 5ms：①在测试并行高负载下（refill 建连 competing CPU）偶发超时
    // panic。放宽到 100ms 消除①的竞态，代价仅是②多等 ≤100ms，断言不变。
    let pool = r2d2::Pool::builder()
        .max_size(1)
        .connection_timeout(std::time::Duration::from_millis(100))
        .build(manager)
        .unwrap();
    let _held = pool.get().unwrap(); // 占用唯一连接
    let err = filter_ai_eligible(&pool, &[(1, Some("x".into()))])
        .await
        .expect_err("DB 连接失败应返回 Err（fail-closed）");
    assert!(
        err.contains("err.db_lock"),
        "错误应含 err.db_lock，实际: {}",
        err
    );
}

// ── 后台翻译批（schedule_translations_background）──
//
// 行为契约：
// - 派发后调用方不等待（fire-and-forget），译文在后台任务中落库；
// - 成功落库的 release 会触发 emitter.emit_release_state_changed（前端刷新）；
// - AI/翻译未启用时不派发（不 spawn、不请求、不写库）。
//
// TRANSLATION_BATCH_RUNNING 是进程级全局门控：操纵它的测试必须互斥执行，
// 否则并行测试互相干扰（全量 cargo test 下会出现「门控被他人置 true →
// schedule 被跳过」与「他人抢到执行权 → 意外派发批」两种假失败）。
// 此锁串行化所有动该门控的测试；不动它的测试无需拿锁。
static TRANSLATION_GATE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 构造已启用 AI+翻译的 memory pool，并插入一条有 body 的 release。
fn setup_translate_pool(base_url: &str) -> r2d2::Pool<crate::db::SqliteConnectionManager> {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let conn = pool.get().unwrap();
    crate::crypto::set_test_master_key();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, base_url).unwrap();
    db::settings::set_setting(
        &conn,
        db::settings::KEY_DEEPSEEK_API_KEY,
        &crate::crypto::encrypt("test-key"),
    )
    .unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_TRANSLATE_RELEASE, "true").unwrap();
    let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
    db::releases::insert_release(
        &conn,
        sid,
        "v1",
        "v1",
        "https://github.com/o/r/releases/tag/v1",
        "2024-01-01T00:00:00Z",
        false,
        Some("release body to translate"),
    )
    .unwrap();
    drop(conn);
    pool
}

fn release_translated(
    pool: &r2d2::Pool<crate::db::SqliteConnectionManager>,
    id: i64,
) -> bool {
    let conn = pool.get().unwrap();
    db::releases::get_release(&conn, id)
        .ok()
        .flatten()
        .map(|r| r.body_translated.is_some())
        .unwrap_or(false)
}

/// 后台批成功路径：mock 翻译 200 → schedule 派发 → 轮询等待批完成 →
/// 断言译文落库 + emit_release_state_changed 已触发。
#[tokio::test]
async fn test_schedule_translations_background_translates_and_emits() {
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path};
    // 拿全局门控互斥锁：末尾才释放（覆盖整个批生命周期），
    // 防止并行测试置位/复位门控干扰本批派发
    let _gate = TRANSLATION_GATE_TEST_LOCK.lock().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "这是译文" } }]
        })))
        .mount(&mock)
        .await;

    let pool = setup_translate_pool(&mock.uri());
    let id = {
        let conn = pool.get().unwrap();
        db::releases::get_releases_without_translation(&conn, &[]).unwrap()[0].0
    };
    let sem = Arc::new(tokio::sync::Semaphore::new(2));
    // noop 与 emitter 指向同一 NoopEmitter：emitter 传给调度（后台持 Arc<dyn>），
    // noop 保留具体类型用于断言 emit 记录。
    let noop = std::sync::Arc::new(crate::types::NoopEmitter::new());
    let emitter: std::sync::Arc<dyn crate::types::Emitter> = noop.clone();

    // 门控状态复位（同一进程内其他测试可能动过；已拿互斥锁，此刻无人在跑）
    RUNTIME.translation_batch_running.store(false, Ordering::Release);

    schedule_translations_background(&pool, &sem, &emitter);

    // fire-and-forget：轮询等后台批收尾（最多 5s）。等的是 emit 而不是 DB：
    // 译文在批内子任务里先落库，emitter 要到批收尾才触发，等 DB 会正好卡在
    // 两者之间的窗口上（CI 慢机上必现：落库了但 emit 记录还是空）。
    let mut emitted = false;
    for _ in 0..50 {
        if noop.state_changed_ids().contains(&id) {
            emitted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(release_translated(&pool, id), "后台翻译批应在超时前把译文写入 DB");
    // 成功落库 → 触发 release-state-changed 刷新
    assert!(
        emitted,
        "成功落库后应 emit release-state-changed, 实际: {:?}",
        noop.state_changed_ids()
    );
}

/// 翻译开关关闭（AI 开但 translate_release=false）：schedule 不派发，
/// 不产生任何 AI 请求（通过断言 retry 计数保持 0 与译文未写证明未发起请求）。
#[tokio::test]
async fn test_schedule_translations_background_noop_when_translate_off() {
    // 拿门控互斥锁：本测试要复位全局门控，防止干扰并行测试的在跑批
    let _gate = TRANSLATION_GATE_TEST_LOCK.lock().await;
    let pool = crate::db::init::init_memory_pool().unwrap();
    let id = {
        let conn = pool.get().unwrap();
        crate::crypto::set_test_master_key();
        db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
        // 不设 KEY_DEEPSEEK_TRANSLATE_RELEASE → 默认 false
        let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
        db::releases::insert_release(
            &conn, sid, "v1", "v1", "https://x", "2024-01-01T00:00:00Z", false,
            Some("body"),
        )
        .unwrap()
    };
    let sem = Arc::new(tokio::sync::Semaphore::new(2));
    let noop = std::sync::Arc::new(crate::types::NoopEmitter::new());
    let emitter: std::sync::Arc<dyn crate::types::Emitter> = noop.clone();
    RUNTIME.translation_batch_running.store(false, Ordering::Release);

    schedule_translations_background(&pool, &sem, &emitter);

    // 给潜在（不应发生的）派发一点时间
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!release_translated(&pool, id), "翻译关闭时不应写入译文");
    assert!(
        noop.state_changed_ids().is_empty(),
        "翻译关闭时不应触发任何刷新"
    );
    let retry = {
        let conn = pool.get().unwrap();
        conn.query_row(
            "SELECT COALESCE(translate_retry_count, 0) FROM releases WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(retry, 0, "翻译关闭时不应发起请求（retry 计数保持 0）");
}

/// 批**失败**时同样要通知日志刷新——这是「译文失败了但日志 tab 里看不到
/// 原因」的直接成因：失败也写了 ERROR 日志，但 fire-and-forget 的批收尾时
/// 发起它的那轮轮询早已结束，无人触发刷新。
#[tokio::test]
async fn test_translation_batch_notifies_log_refresh_even_on_failure() {
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path};
    let mock = MockServer::start().await;
    // 全部返回 524：模拟上游暂时不可用，批内必然失败并写 ERROR 日志
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(524).set_body_json(serde_json::json!({
            "error": { "message": "Upstream model provider is temporarily unavailable" }
        })))
        .mount(&mock)
        .await;

    let pool = setup_translate_pool(&mock.uri());
    let sem = Arc::new(tokio::sync::Semaphore::new(2));
    let noop = std::sync::Arc::new(crate::types::NoopEmitter::new());
    let emitter: std::sync::Arc<dyn crate::types::Emitter> = noop.clone();
    RUNTIME.translation_batch_running.store(false, Ordering::Release);

    // 直接跑单批（绕过调度门控，聚焦收尾行为）
    run_single_translation_batch(&pool, &sem, &emitter).await;

    assert_eq!(
        noop.log_appended_count(),
        1,
        "批失败也应通知日志刷新一次（失败日志同样需要用户看见）"
    );
    assert!(
        noop.state_changed_ids().is_empty(),
        "全失败时不应有 release-state-changed"
    );
    // 且失败确实写进了 DB 日志，否则刷新也无内容可看
    let has_error_log = {
        let conn = pool.get().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM logs WHERE level='ERROR'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        n > 0
    };
    assert!(has_error_log, "翻译失败应写入 ERROR 日志");
}

// ── 摘要后台补全批（schedule_summaries_background）──
//
// 行为契约：历史无摘要的 release 由后台任务补摘要，补全不触发任何 UI emit
// （无新通知，仅落库）。

/// 构造已启用 AI 的 memory pool，插入一条有 body 但无摘要的 release。
fn setup_summary_pool(base_url: &str) -> r2d2::Pool<crate::db::SqliteConnectionManager> {
    let pool = crate::db::init::init_memory_pool().unwrap();
    let conn = pool.get().unwrap();
    crate::crypto::set_test_master_key();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_ENABLED, "true").unwrap();
    db::settings::set_setting(&conn, db::settings::KEY_DEEPSEEK_BASE_URL, base_url).unwrap();
    db::settings::set_setting(
        &conn,
        db::settings::KEY_DEEPSEEK_API_KEY,
        &crate::crypto::encrypt("test-key"),
    )
    .unwrap();
    let sid = db::sources::add_source(&conn, "github", "o", "r", "").unwrap();
    db::releases::insert_release(
        &conn,
        sid,
        "v1",
        "v1",
        "https://github.com/o/r/releases/tag/v1",
        "2024-01-01T00:00:00Z",
        false,
        Some("release body to summarize"),
    )
    .unwrap();
    drop(conn);
    pool
}

/// 后台摘要补全成功路径：mock 摘要 200 → schedule → 等待落库 → 断言 ai_summary
/// 已写入（摘要补全不触发 emit——旧 release 无新通知）。
#[tokio::test]
async fn test_schedule_summaries_background_backfills() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "{\"summary\":\"测试摘要\",\"importance\":\"中\"}" } }]
        })))
        .mount(&mock)
        .await;

    let pool = setup_summary_pool(&mock.uri());
    let id = {
        let conn = pool.get().unwrap();
        db::releases::get_releases_without_summary(&conn, &[]).unwrap()[0].0
    };
    let sem = Arc::new(tokio::sync::Semaphore::new(2));
    let noop = std::sync::Arc::new(crate::types::NoopEmitter::new());
    let emitter: std::sync::Arc<dyn crate::types::Emitter> = noop.clone();
    RUNTIME.summary_batch_running.store(false, Ordering::Release);

    schedule_summaries_background(&pool, &sem, &emitter);

    // 轮询等待后台补全完成（最多 5s）
    let mut done = false;
    for _ in 0..50 {
        let has = {
            let conn = pool.get().unwrap();
            db::releases::get_release(&conn, id)
                .unwrap()
                .unwrap()
                .ai_summary
                .is_some()
        };
        if has {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(done, "后台摘要补全批应在超时前写入 ai_summary");
    let conn = pool.get().unwrap();
    let rel = db::releases::get_release(&conn, id).unwrap().unwrap();
    assert_eq!(rel.ai_summary.as_deref(), Some("测试摘要"));
    assert_eq!(rel.ai_importance.as_deref(), Some("中"));
}

/// 门控语义：上一批在跑（flag=true）时本轮**直接跳过**——不派发、不发请求、
/// 不递增 retry 计数，新待办由下一轮轮询兜底（见 `schedule_translations_background`
/// 的门控注释）。这是删掉跟随批后的核心语义哨兵：若有人改回「在跑时仍派发」
/// 或重新引入等待/续跑机制，此测试会抓住。
#[tokio::test]
async fn test_schedule_translations_background_skips_when_batch_running() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    // 拿门控互斥锁：置 true 的窗口内若有并行测试调用 schedule，会被门控
    // 跳过而假失败；本测试的 swap 也可能被他人的批收尾复位而假派发。
    // 锁覆盖全程（置位 → 观察 → 断言 → 复位）。
    let _gate = TRANSLATION_GATE_TEST_LOCK.lock().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "这是译文" } }]
        })))
        .mount(&mock)
        .await;

    let pool = setup_translate_pool(&mock.uri());
    let id = {
        let conn = pool.get().unwrap();
        db::releases::get_releases_without_translation(&conn, &[]).unwrap()[0].0
    };
    let sem = Arc::new(tokio::sync::Semaphore::new(2));
    let noop = std::sync::Arc::new(crate::types::NoopEmitter::new());
    let emitter: std::sync::Arc<dyn crate::types::Emitter> = noop.clone();

    // 模拟「上一批在跑」：置 true 后 schedule 应同步返回、不派发。
    // 已拿互斥锁，窗口内不会有并行测试复位门控，可安全观察。
    RUNTIME.translation_batch_running.store(true, Ordering::Release);
    schedule_translations_background(&pool, &sem, &emitter);
    // 给潜在（不应发生的）派发留观察窗口
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // 断言完成后复位，再放锁（drop 顺序：_gate 在函数末尾）
    RUNTIME.translation_batch_running.store(false, Ordering::Release);

    assert!(!release_translated(&pool, id), "批在跑时本轮不应翻译");
    assert_eq!(
        mock.received_requests().await.unwrap().len(),
        0,
        "批在跑时本轮不应发出任何 AI 请求"
    );
    let retry = {
        let conn = pool.get().unwrap();
        conn.query_row(
            "SELECT COALESCE(translate_retry_count, 0) FROM releases WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(retry, 0, "跳过不应递增 retry 计数");
}

// ── BatchGuard：panic 时的门控复位 ──
//
// 行为契约：批内 panic 后门控必须回到 false，否则后续批次永久无法派发
// （静默失效，只能重启应用恢复）。守卫借栈展开 Drop 实现，与正常收尾
// 共用一条复位路径。

/// 批内 panic → 门控仍应复位。若复位写在 async 体末尾，panic 会跳过它，
/// flag 永久停在 true；此测试即守卫该回归。
#[tokio::test]
async fn test_batch_guard_resets_flag_on_panic() {
    // 用一对私有的测试用门控，避免动真实批的全局状态（测试并行跑）
    static RUNNING: AtomicBool = AtomicBool::new(false);

    RUNNING.store(true, Ordering::Release);
    let handle = tauri::async_runtime::spawn(async move {
        let _guard = BatchGuard::new(&RUNNING, "测试");
        panic!("批内模拟 panic");
    });
    // panic 的 task 返回 JoinError；吞掉以免测试因 panic 输出噪声失败
    let _ = handle.await;

    assert!(
        !RUNNING.load(Ordering::Acquire),
        "批内 panic 后门控必须复位，否则后续批次永久停摆"
    );
}

/// 正常收尾 → 门控复位（守卫未改变 happy path 行为）。
#[tokio::test]
async fn test_batch_guard_resets_flag_on_normal_completion() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    RUNNING.store(true, Ordering::Release);

    tauri::async_runtime::spawn(async move {
        let _guard = BatchGuard::new(&RUNNING, "测试");
        // 正常返回，无 panic
    })
    .await
    .unwrap();

    assert!(
        !RUNNING.load(Ordering::Acquire),
        "正常收尾后门控应复位"
    );
}
