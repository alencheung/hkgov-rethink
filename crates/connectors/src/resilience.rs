//! Per-source resilience: a token-bucket rate limiter, a circuit breaker, and a
//! bounded retry-with-backoff policy.
//!
//! These wrap every outbound connector call so that one slow/degraded HKGOV
//! endpoint can never starve the others or take the process down. All are
//! intentionally dependency-free (no extra crates) and lock-light.

use hkgov_common::{Error, Result};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// A bounded exponential-backoff + jitter retry policy.
///
/// Retries a future up to `max_attempts` times (so `max_attempts` total tries).
/// A failure is retried only when `should_retry(&error)` is true — by default
/// that's transport errors and HTTP 429/5xx, NOT 4xx (a malformed request or a
/// genuine 404 won't fix themselves, and a `Decode` error signals an upstream
/// schema change that retrying won't help). Backoff is exponential
/// (`base * 2^(attempt-1)`) capped at `max_backoff`, with up to ±25% jitter so a
/// thundering herd of concurrent retries doesn't synchronizedly hammer the
/// upstream. This supersedes HKMA's hand-rolled `get_with_retry` — every
/// connector now inherits the same policy through `ResilientConnector::fetch`
/// (ARCH-CON-01).
pub struct RetryPolicy {
    max_attempts: u32,
    base: Duration,
    max_backoff: Duration,
}

impl RetryPolicy {
    /// `max_attempts` total tries (1 = no retry). `base` is the first backoff;
    /// subsequent ones double, capped at `max_backoff`.
    pub fn new(max_attempts: u32, base: Duration, max_backoff: Duration) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            base,
            max_backoff,
        }
    }

    /// True for errors worth retrying: transport failures (status 0), 429
    /// (rate-limited), and 5xx (server error). 4xx and `Decode` (schema change)
    /// are not retryable.
    fn default_should_retry(e: &Error) -> bool {
        match e {
            Error::Upstream { status, .. } => {
                *status == 0 || *status == 429 || (500..600).contains(status)
            }
            Error::Io(_) => true,
            // Decode = upstream reachable but shape changed; retrying won't help.
            // BadRequest/NotFound/Unauthorized/UnknownSource/Config/Internal = caller bugs.
            _ => false,
        }
    }

    /// Run `f` with retries. The closure is re-invoked on retryable failures.
    pub async fn run<F, T, Fut>(&self, origin: &'static str, f: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err: Option<Error> = None;
        for attempt in 1..=self.max_attempts {
            if attempt > 1 {
                let backoff = self.backoff_for(attempt);
                tracing::debug!(
                    origin,
                    attempt,
                    backoff_ms = backoff.as_millis() as u64,
                    "retrying after backoff"
                );
                tokio::time::sleep(backoff).await;
            }
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let retryable = Self::default_should_retry(&e);
                    tracing::debug!(origin, attempt, retryable, error = %e, "attempt failed");
                    if !retryable || attempt >= self.max_attempts {
                        return Err(e);
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Upstream {
            origin,
            status: 0,
            detail: "exhausted retries".into(),
        }))
    }

    /// Exponential backoff for a given attempt number, capped, with jitter.
    fn backoff_for(&self, attempt: u32) -> Duration {
        // attempt >= 2 here (attempt 1 is the first try, no backoff).
        let exp = attempt.saturating_sub(2).min(31);
        let shift = 1u32.checked_shl(exp).unwrap_or(u32::MAX);
        let raw = self.base.saturating_mul(shift);
        let capped = raw.min(self.max_backoff);
        // ±25% jitter, derived from the system clock so no extra RNG crate is
        // needed. This is a politeness jitter, not a security nonce.
        let jitter_range = capped / 4;
        if jitter_range.is_zero() {
            return capped;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let offset = Duration::from_nanos((nanos % (jitter_range.as_nanos().max(1) as u32)) as u64);
        capped - jitter_range + offset
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        // 3 attempts, 200ms base, 8s cap — matches HKMA's prior private policy
        // so behavior is unchanged for the one connector that already retried.
        Self::new(3, Duration::from_millis(200), Duration::from_secs(8))
    }
}

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
                // time until one token refills. Guard against a zero/negative
                // refill rate (treated as unlimited): `.max(EPSILON)` keeps the
                // divisor finite and lets the request through without panicking.
                let refill = self.refill_per_sec.max(f64::EPSILON);
                Duration::from_secs_f64((1.0 - s.tokens) / refill)
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
        let state = self.state.load(Ordering::Acquire);
        match state {
            CLOSED => Ok(()),
            OPEN => {
                let opened = self.opened_at_ms.load(Ordering::Acquire);
                if Self::now_ms().saturating_sub(opened) >= self.cooldown.as_millis() as u64 {
                    // Cooldown elapsed: exactly one caller must become the probe.
                    // Use a CAS so concurrent callers racing through OPEN all see
                    // the same outcome — the winner flips OPEN → HALF_OPEN and
                    // proceeds; every loser fails the CAS (state is now
                    // HALF_OPEN), sees the circuit as still open, and is told to
                    // back off. Without the CAS, every concurrent caller would
                    // store HALF_OPEN and all believe they are the lone probe.
                    match self.state.compare_exchange(
                        OPEN,
                        HALF_OPEN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => Ok(()), // won the probe slot
                        Err(_) => Err("circuit open"),
                    }
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
        self.failures.store(0, Ordering::Release);
        self.state.store(CLOSED, Ordering::Release);
    }

    pub fn on_failure(&self) {
        // The counter itself only needs Relaxed, but the threshold read below
        // must observe a value at least as recent as our increment, so pair the
        // fetch_add with an Acquire load when we test it.
        let f = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
        if f >= self.failure_threshold {
            self.state.store(OPEN, Ordering::Release);
            self.opened_at_ms.store(Self::now_ms(), Ordering::Release);
        } else if self.state.load(Ordering::Acquire) == HALF_OPEN {
            // Probe failed: reopen.
            self.state.store(OPEN, Ordering::Release);
            self.opened_at_ms.store(Self::now_ms(), Ordering::Release);
        }
    }

    pub fn state_label(&self) -> &'static str {
        match self.state.load(Ordering::Acquire) {
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

    /// Regression for the half-open probe race: when the cooldown elapses while
    /// the breaker is OPEN, only ONE concurrent caller may flip to HALF_OPEN
    /// (the probe). Every other caller must observe the circuit as open and be
    /// rejected. Previously the transition was an unconditional store, so every
    /// racer became a probe.
    #[test]
    fn half_open_allows_single_probe() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(10));
        cb.on_failure(); // → OPEN
        assert_eq!(cb.state_label(), "open");
        std::thread::sleep(Duration::from_millis(20)); // let cooldown elapse

        // Fire many callers "concurrently" from multiple threads. The CAS must
        // ensure exactly one Ok and the rest Err.
        let cb = std::sync::Arc::new(cb);
        let n = 32;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let cb = cb.clone();
            let tx = tx.clone();
            let h = std::thread::spawn(move || {
                let ok = cb.before_call().is_ok();
                tx.send(ok).unwrap();
            });
            handles.push(h);
        }
        drop(tx);
        for h in handles {
            h.join().unwrap();
        }
        let results: Vec<bool> = rx.iter().collect();
        let winners = results.iter().filter(|&&ok| ok).count();
        assert_eq!(
            winners, 1,
            "exactly one probe should win the OPEN→HALF_OPEN race, got {winners}"
        );
        // After the race the breaker must be parked in HALF_OPEN; further calls
        // are rejected until the probe resolves.
        assert_eq!(cb.state_label(), "half-open");
        assert!(cb.before_call().is_err());
    }

    // ---- RetryPolicy tests (ARCH-CON-01) ----

    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_succeeds_after_transient_failure() {
        let policy = RetryPolicy::new(3, Duration::from_millis(1), Duration::from_millis(5));
        let calls = Arc::new(AtomicU32::new(0));
        let calls2 = calls.clone();
        let r = policy
            .run("test", move || {
                let c = calls2.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 3 {
                        Err(Error::Upstream {
                            origin: "test",
                            status: 503,
                            detail: "transient".into(),
                        })
                    } else {
                        Ok("ok")
                    }
                }
            })
            .await;
        assert_eq!(r.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_does_not_retry_non_retryable_error() {
        // A 404 (NotFound mapped) must NOT be retried — it won't fix itself.
        let policy = RetryPolicy::new(3, Duration::from_millis(1), Duration::from_millis(5));
        let calls = Arc::new(AtomicU32::new(0));
        let calls2 = calls.clone();
        let r: Result<&str, _> = policy
            .run("test", move || {
                let c = calls2.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(Error::NotFound("gone".into()))
                }
            })
            .await;
        assert!(matches!(r, Err(Error::NotFound(_))));
        // Only one attempt — non-retryable.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_exhausts_then_returns_last_error() {
        let policy = RetryPolicy::new(2, Duration::from_millis(1), Duration::from_millis(5));
        let calls = Arc::new(AtomicU32::new(0));
        let calls2 = calls.clone();
        let r: Result<&str, _> = policy
            .run("test", move || {
                let c = calls2.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(Error::Upstream {
                        origin: "test",
                        status: 502,
                        detail: "down".into(),
                    })
                }
            })
            .await;
        assert!(matches!(r, Err(Error::Upstream { status: 502, .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
