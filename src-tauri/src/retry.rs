use std::future::Future;
use std::time::Duration;
use rand::Rng;

/// 指数退避重试配置。
pub struct RetryConfig {
    /// 最大重试次数（不含首次尝试）。
    pub max_retries: u32,
    /// 基础延迟秒数，第 n 次重试延迟 = base_delay_secs * 2^n。
    pub base_delay_secs: u32,
    /// 最大延迟上限（秒）。
    pub max_delay_secs: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_secs: 2,
            max_delay_secs: 60,
        }
    }
}

/// 通用指数退避重试。
///
/// - `f`: 异步操作工厂（每次重试重新调用）。
/// - `should_retry`: 判断错误是否应该重试，传入错误引用。
///
/// 首次失败后，第 1 次重试延迟 2s，第 2 次 4s，第 3 次 8s，依此类推，
/// 最大不超过 `max_delay_secs`。总重试次数不超过 `max_retries`。
pub async fn retry_with_backoff<F, Fut, T, E>(
    config: &RetryConfig,
    should_retry: impl Fn(&E) -> bool,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 0u32;
    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if !should_retry(&e) || attempt >= config.max_retries {
                    return Err(e);
                }
                attempt += 1;
                let delay_secs = (config.base_delay_secs.saturating_mul(1 << (attempt - 1)))
                    .min(config.max_delay_secs)
                    .max(1);
                let jitter = rand::thread_rng().gen_range(0.75..1.25);
                let delay_ms = (delay_secs as f64 * jitter * 1000.0) as u64;
                log::info!(
                    "retry {}/{}: sleeping {}ms (base_delay={}s, jitter={:.2})",
                    attempt, config.max_retries, delay_ms, config.base_delay_secs, jitter
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn test_success_on_first_try() {
        let config = RetryConfig::default();
        let result = retry_with_backoff(&config, |_| false, || async { Ok::<_, &str>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_then_success() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_secs: 1,
            max_delay_secs: 2,
        };
        let counter = Mutex::new(0u32);
        let result = retry_with_backoff(&config, |_| true, || async {
            let mut c = counter.lock().unwrap();
            *c += 1;
            if *c < 3 {
                Err("fail")
            } else {
                Ok("ok")
            }
        })
        .await;
        assert_eq!(result.unwrap(), "ok");
        assert_eq!(*counter.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn test_no_retry_on_non_retryable() {
        let config = RetryConfig::default();
        let result: Result<(), &str> = retry_with_backoff(&config, |_| false, || async { Err("fatal") }).await;
        assert_eq!(result.unwrap_err(), "fatal");
    }

    #[tokio::test]
    async fn test_max_retries_exhausted() {
        let config = RetryConfig {
            max_retries: 2,
            base_delay_secs: 1,
            max_delay_secs: 2,
        };
        let mut calls = 0u32;
        let result: Result<(), &str> = retry_with_backoff::<_, _, _, &str>(&config, |_| true, || {
            calls += 1;
            async { Err("always_fail") }
        })
        .await;
        assert!(result.is_err());
        // 1 initial + 2 retries = 3 total
        assert_eq!(calls, 3);
    }
}
