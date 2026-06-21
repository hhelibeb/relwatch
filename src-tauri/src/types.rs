use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicI64;
use tokio::sync::Semaphore;

use crate::db::logs::LogEntry;
use crate::db::releases::ReleaseInfo;

pub struct AppState {
    pub db: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    pub next_poll_at: std::sync::Arc<AtomicI64>,
    pub deepseek_semaphore: std::sync::Arc<Semaphore>,
}

#[derive(Serialize)]
pub struct PollResult {
    pub new_releases: Vec<ReleaseInfo>,
}

#[derive(Serialize)]
pub struct LogSearchResult {
    pub entries: Vec<LogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Serialize, Deserialize)]
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
    pub github_token_set: bool,
}
