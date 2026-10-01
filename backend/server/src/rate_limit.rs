use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// 内存中最多保留的限流 key 数。攻击者用随机 key 发起请求时，
/// 超出上限后先淘汰窗口已清空的 key，仍超限则整体清空，保证内存有界。
const MAX_KEYS: usize = 10_000;

/// 限流判定结果。拒绝时 `retry_after` 为距窗口最旧 Attempt 过期、
/// 请求可恢复的最短剩余时长，用于 HTTP `Retry-After` 响应头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    allowed: bool,
    retry_after: Duration,
}

impl RateLimitDecision {
    pub fn allowed(&self) -> bool {
        self.allowed
    }

    pub fn retry_after(&self) -> Duration {
        self.retry_after
    }
}

#[derive(Clone, Default)]
pub struct RateLimiter {
    entries: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl RateLimiter {
    pub fn check(&self, key: &str, limit: usize, window: Duration) -> RateLimitDecision {
        let now = Instant::now();
        let Ok(mut entries) = self.entries.lock() else {
            // Mutex 中毒视为内部故障：保守拒绝，按整个窗口给重试时间。
            return RateLimitDecision {
                allowed: false,
                retry_after: window,
            };
        };
        // 新 key 且容量已满：惰性清理窗口已全部过期的 key；仍超上限则整体清空。
        // 清空是安全的——各 key 的窗口独立，清空只是让所有限流计数归零。
        if !entries.contains_key(key) && entries.len() >= MAX_KEYS {
            entries.retain(|_, attempts| {
                attempts
                    .back()
                    .is_some_and(|latest| now.duration_since(*latest) < window)
            });
            if entries.len() >= MAX_KEYS {
                entries.clear();
            }
        }
        let attempts = entries.entry(key.to_owned()).or_default();
        while attempts
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) >= window)
        {
            attempts.pop_front();
        }
        if attempts.len() >= limit {
            let retry_after = attempts
                .front()
                .map(|oldest| window.saturating_sub(now.duration_since(*oldest)))
                .unwrap_or(window);
            return RateLimitDecision {
                allowed: false,
                retry_after,
            };
        }
        attempts.push_back(now);
        RateLimitDecision {
            allowed: true,
            retry_after: window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_requests_after_the_limit() {
        let limiter = RateLimiter::default();
        let window = Duration::from_secs(60);

        assert!(limiter.check("login:owner", 2, window).allowed());
        assert!(limiter.check("login:owner", 2, window).allowed());
        assert!(!limiter.check("login:owner", 2, window).allowed());
        assert!(limiter.check("login:editor", 2, window).allowed());
    }

    #[test]
    fn rejected_decision_carries_remaining_window_for_retry_after() {
        let limiter = RateLimiter::default();
        let window = Duration::from_secs(60);

        assert!(limiter.check("login:owner", 1, window).allowed());
        let rejected = limiter.check("login:owner", 1, window);
        assert!(!rejected.allowed());
        // 剩余窗口随时间流逝只会缩短，且不超过窗口总长。
        let remaining = rejected.retry_after();
        assert!(
            remaining <= window,
            "retry_after {remaining:?} must not exceed the window"
        );
        assert!(
            remaining > window - Duration::from_secs(5),
            "retry_after {remaining:?} should be close to the full window just after rejection"
        );
    }

    #[test]
    fn retry_after_shrinks_as_the_window_expires() {
        let limiter = RateLimiter::default();
        let window = Duration::from_millis(300);

        assert!(limiter.check("k", 1, window).allowed());
        std::thread::sleep(Duration::from_millis(100));
        let rejected = limiter.check("k", 1, window);
        assert!(!rejected.allowed());
        let remaining = rejected.retry_after();
        assert!(remaining <= Duration::from_millis(200));
        assert!(remaining > Duration::from_millis(100));
    }

    #[test]
    fn key_count_stays_bounded_under_random_key_flood() {
        let limiter = RateLimiter::default();
        let window = Duration::from_secs(60);

        for index in 0..(MAX_KEYS + 500) {
            assert!(
                limiter
                    .check(&format!("flood:{index}"), 1, window)
                    .allowed()
            );
        }
        let guard = limiter.entries.lock().unwrap();
        assert!(
            guard.len() <= MAX_KEYS,
            "key count {} exceeded the capacity bound",
            guard.len()
        );
    }

    #[test]
    fn capacity_eviction_prefers_expired_keys_over_full_reset() {
        let limiter = RateLimiter::default();
        let window = Duration::from_millis(30);

        // 填满容量，随后让整个窗口过期。
        for index in 0..MAX_KEYS {
            assert!(
                limiter
                    .check(&format!("flood:{index}"), 1, window)
                    .allowed()
            );
        }
        std::thread::sleep(Duration::from_millis(40));
        // 新 key 触发惰性淘汰：窗口已过期的 key 被移除，无需整体清空。
        assert!(limiter.check("fresh", 1, window).allowed());
        let guard = limiter.entries.lock().unwrap();
        assert_eq!(guard.len(), 1);
        assert!(guard.contains_key("fresh"));
        drop(guard);
        // 被驱逐的 key 计数已归零，可以再次请求。
        assert!(limiter.check("flood:0", 1, window).allowed());
    }
}
