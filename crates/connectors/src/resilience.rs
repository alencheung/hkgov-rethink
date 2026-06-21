//! Per-source resilience: a token-bucket rate limiter and a circuit breaker.
//!
//! These wrap every outbound connector call so that one slow/degraded HKGOV
//! endpoint can never starve the others or take the process down. Both are
//! intentionally dependency-free (no extra crates) and lock-light.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Simple token-bucket limiter. `capacity` tokens, refilled at
/// `tokens_per_sec`. `acquire()` blocks until a token is available.
pub struct RateLimiter {
    capacity: u64,
    refill_per_sec: f64,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(capacity: u64, tokens_per_sec: f64) -> Self {
        Self {
            capacity,
            refill_per_sec: tokens_per_sec,
            state: Mutex::new(BucketState {
                tokens: capacity as f64,
                last: Instant::now(),
            }),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut s = self.state.lock().await;
                let now = Instant::now();
                let elapsed = now.duration_since(s.last).as_secs_f64();
                s.tokens = (s.tokens + elapsed * self.refill_per_sec).min(self.capacity as f64);
                s.last = now;
                if s.tokens >= 1.0 {
                    s.tokens -= 1.0;
                    return;
                }
                // time until one token refills
                Duration::from_secs_f64((1.0 - s.tokens) / self.refill_per_sec)
            };
            if wait.is_zero() {
                tokio::task::yield_now().await;
            } else {
                tokio::time::sleep(wait.min(Duration::from_secs(1))).await;
            }
        }
    }
}

/// Three-state circuit breaker: Closed (normal) → Open (failing fast) →
/// HalfOpen (probe). Opens after `failure_threshold` consecutive failures,
/// stays open for `cooldown`, then allows a single probe.
pub struct CircuitBreaker {
    state: AtomicU8, // 0=closed, 1=open, 2=half
    failures: AtomicU64,
    opened_at_ms: AtomicU64,
    failure_threshold: u64,
    cooldown: Duration,
}

const CLOSED: u8 = 0;
const OPEN: u8 = 1;
const HALF_OPEN: u8 = 2;

impl CircuitBreaker {
    pub fn new(failure_threshold: u64, cooldown: Duration) -> Self {
        Self {
            state: AtomicU8::new(CLOSED),
            failures: AtomicU64::new(0),
            opened_at_ms: AtomicU64::new(0),
            failure_threshold,
            cooldown,
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Returns Ok(()) to proceed, Err if the circuit is open.
    pub fn before_call(&self) -> Result<(), &'static str> {
        let state = self.state.load(Ordering::Relaxed);
        match state {
            CLOSED => Ok(()),
            OPEN => {
                let opened = self.opened_at_ms.load(Ordering::Relaxed);
                if Self::now_ms().saturating_sub(opened) >= self.cooldown.as_millis() as u64 {
                    // Transition to half-open; allow one probe.
                    self.state.store(HALF_OPEN, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err("circuit open")
                }
            }
            HALF_OPEN => {
                // Only one probe allowed at a time; if already probing, reject.
                Err("circuit half-open (probe in flight)")
            }
            _ => Ok(()),
        }
    }

    pub fn on_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
        self.state.store(CLOSED, Ordering::Relaxed);
    }

    pub fn on_failure(&self) {
        let f = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
        if f >= self.failure_threshold {
            self.state.store(OPEN, Ordering::Relaxed);
            self.opened_at_ms.store(Self::now_ms(), Ordering::Relaxed);
        } else if self.state.load(Ordering::Relaxed) == HALF_OPEN {
            // Probe failed: reopen.
            self.state.store(OPEN, Ordering::Relaxed);
            self.opened_at_ms.store(Self::now_ms(), Ordering::Relaxed);
        }
    }

    pub fn state_label(&self) -> &'static str {
        match self.state.load(Ordering::Relaxed) {
            CLOSED => "closed",
            OPEN => "open",
            HALF_OPEN => "half-open",
            _ => "closed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limiter_releases_tokens() {
        let rl = RateLimiter::new(2, 100.0);
        rl.acquire().await;
        rl.acquire().await;
        // third would block briefly; just assert we can still acquire after refill
        tokio::time::timeout(Duration::from_millis(500), rl.acquire())
            .await
            .expect("third acquire within 500ms");
    }

    #[test]
    fn circuit_opens_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert!(cb.before_call().is_ok());
        cb.on_failure();
        cb.on_failure();
        assert!(cb.before_call().is_ok()); // still closed
        cb.on_failure(); // 3rd → open
        assert_eq!(cb.state_label(), "open");
        assert!(cb.before_call().is_err());
    }

    #[test]
    fn circuit_closes_on_success_after_cooldown() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(10));
        cb.on_failure();
        assert_eq!(cb.state_label(), "open");
        std::thread::sleep(Duration::from_millis(20));
        assert!(cb.before_call().is_ok()); // transitions to half-open
        cb.on_success();
        assert_eq!(cb.state_label(), "closed");
    }
}
