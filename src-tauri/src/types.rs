use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicI64;
use std::sync::Mutex;

use crate::db::releases::ReleaseInfo;

pub struct AppState {
    pub db: Mutex<Connection>,
    pub next_poll_at: std::sync::Arc<AtomicI64>,
}

#[derive(Serialize)]
pub struct PollResult {
    pub new_releases: Vec<ReleaseInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    pub poll_interval_minutes: i64,
    pub proxy_url: String,
    pub minimize_to_tray: bool,
    pub log_retention_days: i64,
    pub deepseek_enabled: bool,
    pub deepseek_model: String,
    pub deepseek_base_url: String,
    pub deepseek_api_key_set: bool,
    pub deepseek_proxy_enabled: bool,
    pub check_prereleases: bool,
    pub language: String,
    pub github_token_set: bool,
}
