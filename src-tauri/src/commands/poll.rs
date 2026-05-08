use crate::poll;
use crate::types::{AppState, PollResult};

#[tauri::command]
pub async fn trigger_poll(app: tauri::AppHandle) -> Result<PollResult, String> {
    poll::trigger_poll(app).await
}

#[tauri::command]
pub fn get_poll_countdown(state: tauri::State<AppState>) -> i64 {
    let now = chrono::Utc::now().timestamp();
    let next = state
        .next_poll_at
        .load(std::sync::atomic::Ordering::Relaxed);
    let remaining = next - now;
    if remaining < 0 {
        0
    } else {
        remaining
    }
}
