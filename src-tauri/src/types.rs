use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::atomic::AtomicI64;
use tokio::sync::Semaphore;

use crate::db::logs::LogEntry;
use crate::db::releases::ReleaseInfo;

/// 可测试的事件发射器抽象层。
///
/// 编排层核心函数通过 `&dyn Emitter` 依赖注入替代 `&tauri::AppHandle`，
/// 使并发/状态机/错误传播编排链路可在纯测试环境中验证。
pub trait Emitter: Send + Sync {
    /// 发送 release 桌面通知。
    /// 这是 `collect_pending_and_notify` 对 Tauri 主线程的唯一依赖。
    fn notify_release(&self, params: ReleaseNotifyParams);
}

/// `Emitter::notify_release` 的参数载体：把原先 8 个位置参数收敛为结构体，
/// 既消除 `clippy::too_many_arguments` 警告，也便于未来扩展通知字段。
#[derive(Clone)]
pub struct ReleaseNotifyParams {
    pub release_id: i64,
    pub html_url: String,
    pub owner: String,
    pub repo: String,
    pub tag: String,
    pub name: String,
    pub importance: Option<String>,
}

/// Tauri AppHandle 实现：通过 `run_on_main_thread` 派发通知。
impl Emitter for tauri::AppHandle {
    fn notify_release(&self, params: ReleaseNotifyParams) {
        let app = self.clone();
        let _ = self.run_on_main_thread(move || {
            let ReleaseNotifyParams {
                release_id,
                html_url,
                owner,
                repo,
                tag,
                name,
                importance,
            } = params;
            crate::notify::send_release_notification(
                &app, release_id, html_url, owner, repo, tag, name, importance,
            );
        });
    }
}

/// 测试用 Noop Emitter：不发送任何通知，记录调用参数（供断言通知内容可读性）。
#[cfg(test)]
pub struct NoopEmitter {
    calls: std::sync::Mutex<Vec<ReleaseNotifyParams>>,
}

#[cfg(test)]
impl NoopEmitter {
    pub const fn new() -> Self {
        NoopEmitter {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 已记录的通知调用次数。
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// 全部通知参数（按调用顺序），供断言标题/正文可读性。
    pub fn params(&self) -> Vec<ReleaseNotifyParams> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Emitter for NoopEmitter {
    fn notify_release(&self, params: ReleaseNotifyParams) {
        self.calls.lock().unwrap().push(params);
    }
}

pub struct AppState {
    pub db: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    pub next_poll_at: std::sync::Arc<AtomicI64>,
    pub deepseek_semaphore: std::sync::Arc<Semaphore>,
    /// 无头 Agent 子进程并发上限（Agent 进程较重，限制同时运行数量）。
    pub agent_semaphore: std::sync::Arc<Semaphore>,
    /// pi RPC 常驻进程管理器（工作区对话驱动核心）。
    pub agent_rpc: std::sync::Arc<crate::agent_rpc::RpcManager>,
    /// 用户请求取消的 run 集合（dispatch 结束写入 cancelled 状态）。
    pub agent_cancelled: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
}

/// Agent 类型能力描述（前端配置表单驱动）。

#[derive(Serialize, Type)]
pub struct PollResult {
    pub new_releases: Vec<ReleaseInfo>,
}

#[derive(Serialize, Type)]
pub struct LogSearchResult {
    pub entries: Vec<LogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// 设置读写共用同一结构：get_settings 返回它，update_settings 直接接收它。
/// 前端 payload 与后端结构字段一一对应（snake_case），
/// 新增设置项只需改 AppSettings 一处 + get_settings/apply_settings 两处。
#[derive(Serialize, Deserialize, Type)]
pub struct AppSettings {
    pub poll_interval_minutes: i64,
    pub proxy_url: String,
    pub proxy_mode: String,
    pub auto_start: bool,
    pub minimize_to_tray: bool,
    pub log_retention_days: i64,
    pub deepseek_enabled: bool,
    pub deepseek_model: String,
    pub deepseek_base_url: String,
    /// 派生只读标志（凭据是否已设置），update_settings 忽略其值。
    pub deepseek_api_key_set: bool,
    pub deepseek_proxy_bypass: bool,
    pub deepseek_prompt: String,
    pub deepseek_min_importance: String,
    pub deepseek_translate_release: bool,

    pub check_prereleases: bool,
    pub fetch_history: bool,
    pub fetch_history_count: i64,
    pub language: String,
    pub theme: String,
    pub show_source_type_icons: bool,
    pub enable_usage_stats: bool,
    pub github_token_set: bool,
    pub youtube_api_key_set: bool,
    pub bilibili_cookie_set: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_app_settings() -> AppSettings {
        AppSettings {
            poll_interval_minutes: 30,
            proxy_url: "http://127.0.0.1:7890".into(),
            proxy_mode: "manual".into(),
            auto_start: true,
            minimize_to_tray: true,
            log_retention_days: 7,
            deepseek_enabled: true,
            deepseek_model: "deepseek-chat".into(),
            deepseek_base_url: "https://api.deepseek.com".into(),
            deepseek_api_key_set: true,
            deepseek_proxy_bypass: false,
            deepseek_prompt: "请用中文总结".into(),
            deepseek_min_importance: "中".into(),
            deepseek_translate_release: true,
            check_prereleases: true,
            fetch_history: false,
            fetch_history_count: 100,
            language: "zh-CN".into(),
            theme: "system".into(),
            show_source_type_icons: true,
            enable_usage_stats: true,
            github_token_set: true,
            youtube_api_key_set: true,
            bilibili_cookie_set: true,
        }
    }

    #[test]
    fn app_settings_round_trip_json() {
        let original = sample_app_settings();
        let value = serde_json::to_value(&original).unwrap();
        let restored: AppSettings = serde_json::from_value(value.clone()).unwrap();

        // 序列化结果使用 snake_case 字段名，且反序列化后信息不丢失
        assert_eq!(serde_json::to_value(&restored).unwrap(), value);
        assert_eq!(restored.poll_interval_minutes, 30);
        assert_eq!(restored.language, "zh-CN");
        assert!(restored.deepseek_api_key_set);
    }

    #[test]
    fn app_settings_serializes_snake_case_keys() {
        let value = serde_json::to_value(sample_app_settings()).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("poll_interval_minutes"));
        assert!(obj.contains_key("minimize_to_tray"));
        assert!(obj.contains_key("show_source_type_icons"));
        assert_eq!(obj["proxy_mode"], "manual");
        assert_eq!(obj["deepseek_min_importance"], "中");
    }

    #[test]
    fn poll_result_serializes_release_list() {
        let release = ReleaseInfo {
            id: 1,
            source_id: 2,
            source_type: "github".into(),
            owner: "vuejs".into(),
            repo: "core".into(),
            tag_name: "v1.0.0".into(),
            release_name: "v1.0.0".into(),
            html_url: "https://github.com/vuejs/core/releases/tag/v1.0.0".into(),
            published_at: "2025-01-01T00:00:00Z".into(),
            prerelease: false,
            body: Some("release body".into()),
            detected_at: "2025-01-02T00:00:00Z".into(),
            notification_status: "unread".into(),
            snooze_until: None,
            ai_summary: Some("摘要".into()),
            ai_importance: Some("大".into()),
            body_translated: None,
            extra_metadata: None,
            source_description: None,
        };
        let value = serde_json::to_value(PollResult {
            new_releases: vec![release],
        })
        .unwrap();

        assert_eq!(value["new_releases"][0]["owner"], "vuejs");
        assert_eq!(value["new_releases"][0]["tag_name"], "v1.0.0");
        assert_eq!(value["new_releases"][0]["ai_importance"], "大");
        assert_eq!(value["new_releases"][0]["snooze_until"], json!(null));
    }

    #[test]
    fn log_search_result_serializes_entries() {
        let entry = LogEntry {
            id: 9,
            level: "error".into(),
            message: "boom".into(),
            created_at: "2025-06-01T00:00:00Z".into(),
            message_key: Some("err.poll_failed".into()),
            message_args: Some("github|vuejs/core".into()),
            rendered_message: None,
        };
        let value = serde_json::to_value(LogSearchResult {
            entries: vec![entry],
            total: 1,
            page: 1,
            page_size: 50,
        })
        .unwrap();

        assert_eq!(value["total"], 1);
        assert_eq!(value["page_size"], 50);
        assert_eq!(value["entries"][0]["message_key"], "err.poll_failed");
        assert_eq!(value["entries"][0]["rendered_message"], json!(null));
    }

    #[test]
    fn release_notify_params_clones_independently() {
        let params = ReleaseNotifyParams {
            release_id: 42,
            html_url: "https://example.com/r".into(),
            owner: "o".into(),
            repo: "r".into(),
            tag: "v1".into(),
            name: "release".into(),
            importance: Some("小".into()),
        };
        let cloned = params.clone();

        assert_eq!(cloned.release_id, 42);
        assert_eq!(cloned.owner, "o");
        assert_eq!(cloned.tag, "v1");
        assert_eq!(cloned.importance, Some("小".into()));
    }

    #[test]
    fn noop_emitter_counts_calls() {
        let emitter = NoopEmitter::new();
        emitter.notify_release(ReleaseNotifyParams {
            release_id: 1,
            html_url: "u".into(),
            owner: "o".into(),
            repo: "r".into(),
            tag: "t".into(),
            name: "n".into(),
            importance: None,
        });
        emitter.notify_release(ReleaseNotifyParams {
            release_id: 2,
            html_url: "u".into(),
            owner: "o".into(),
            repo: "r".into(),
            tag: "t".into(),
            name: "n".into(),
            importance: None,
        });

        assert_eq!(emitter.call_count(), 2);
    }
}
