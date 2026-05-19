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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicI64;

    /// Test the pure countdown math that get_poll_countdown performs.
    /// The function itself takes State<AppState> (Tauri-internal), so we
    /// test the equivalent computation directly.
    #[test]
    fn test_poll_countdown_future() {
        let now = chrono::Utc::now().timestamp();
        let next_at = Arc::new(AtomicI64::new(now + 300)); // 5 min in future
        let remaining = next_at.load(std::sync::atomic::Ordering::Relaxed) - now;
        let result = if remaining < 0 { 0 } else { remaining };
        assert_eq!(result, 300);
    }

    #[test]
    fn test_poll_countdown_past() {
        let now = chrono::Utc::now().timestamp();
        let next_at = Arc::new(AtomicI64::new(now - 300)); // 5 min in past
        let remaining = next_at.load(std::sync::atomic::Ordering::Relaxed) - now;
        let result = if remaining < 0 { 0 } else { remaining };
        assert_eq!(result, 0);
    }

    #[test]
    fn test_poll_countdown_exact_now() {
        let now = chrono::Utc::now().timestamp();
        let next_at = Arc::new(AtomicI64::new(now));
        let remaining = next_at.load(std::sync::atomic::Ordering::Relaxed) - now;
        let result = if remaining < 0 { 0 } else { remaining };
        assert_eq!(result, 0);
    }

    #[test]
    fn test_poll_countdown_large_value() {
        let now = chrono::Utc::now().timestamp();
        let next_at = Arc::new(AtomicI64::new(now + 86400)); // 1 day
        let remaining = next_at.load(std::sync::atomic::Ordering::Relaxed) - now;
        let result = if remaining < 0 { 0 } else { remaining };
        assert_eq!(result, 86400);
    }
}
