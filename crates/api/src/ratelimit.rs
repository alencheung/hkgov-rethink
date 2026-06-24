//! Per-client rate limiting for the abuse-prone write endpoints.
//!
//! Protects the token-burning `POST /v1/ask` (and the other expensive POSTs —
//! signals/preview, investigations, feedback) from runaway loops, casual
//! scripting, and deliberate token-burn attacks. A caller is metered along
//! three independent dimensions, all of which must pass:
//!
//! - **session** — the `Authorization: Bearer {token}` (P-108 identity), when present.
//! - **device** — the `X-Reader-Id` pseudo-identity header, when present.
//! - **ip** — the client IP (peer `SocketAddr`, optionally via `X-Forwarded-For`).
//!
//! Limits are configured in `[api]` (`ask_per_window`, `ask_warn_at`,
//! `ask_window_secs`). At `ask_warn_at` requests in the window the response
//! carries an `X-RateLimit-Warning` header (so a well-behaved client can slow
//! down before being blocked); at `ask_per_window` the request is rejected with
//! `429 Too Many Requests` + `Retry-After`.
//!
//! Implementation notes:
//! - Fixed-window (not sliding) — cheaper, and the abuse model (stop a burst)
//!   doesn't need sliding-window precision. The window key is `now_secs //
//!   window_secs`, so every request in the same window shares one counter.
//! - In-process (`tokio::Mutex<HashMap>`), matching the single-node `memory`
//!   store backend. The horizontal-scale path is a Redis-backed limiter; until
//!   then this is volatile and resets on restart (acceptable — abuse protection
//!   is best-effort, not accounting).
//! - Stale counters from prior windows are evicted opportunistically on each
//!   check so the map can't grow unbounded under a rotating-identity attack.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// A rate-limit decision for one request against one dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Under the warn threshold — proceed normally.
    Allow,
    /// At or above the warn threshold but still under the hard cap — proceed,
    /// but the response should carry `X-RateLimit-Warning`.
    Warn,
    /// At or over the hard cap — reject with 429. Carries the seconds remaining
    /// in the current window (for the `Retry-After` header).
    Block { retry_after_secs: u64 },
}

impl Verdict {
    #[cfg(test)]
    pub fn is_block(self) -> bool {
        matches!(self, Verdict::Block { .. })
    }
}

/// A fixed-window counter store, keyed by an opaque principal string
/// (`"session:{token}"`, `"device:{id}"`, `"ip:{addr}"`). One `Limiter` is
/// shared by all dimensions and all protected routes — the dimension is baked
/// into the key prefix so counters never collide.
pub struct Limiter {
    state: Mutex<HashMap<String, WindowCount>>,
}

#[derive(Debug, Clone, Copy)]
struct WindowCount {
    /// The fixed window this count belongs to (`now_secs // window_secs`).
    window: u64,
    /// Requests counted in `window` so far (including the in-flight one once
    /// recorded).
    count: u64,
}

impl Default for Limiter {
    fn default() -> Self {
        // The map grows with distinct principals; a pre-allocated capacity keeps
        // small fleets off the allocator hot path. Resizes normally beyond this.
        Self::new(256)
    }
}

impl Limiter {
    pub fn new(capacity_hint: usize) -> Self {
        Self {
            state: Mutex::new(HashMap::with_capacity(capacity_hint)),
        }
    }

    /// Record one request for `key` and return the verdict for this dimension.
    ///
    /// `now` is injected so tests can drive the clock; production passes
    /// `SystemTime::now()`. `per_window == 0` means "no limit on this
    /// dimension" (the call is a no-op `Allow`).
    pub async fn check(
        &self,
        key: &str,
        per_window: u32,
        warn_at: u32,
        window_secs: u64,
        now: SystemTime,
    ) -> Verdict {
        if per_window == 0 {
            return Verdict::Allow;
        }
        let now_secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let window = now_secs.checked_div(window_secs.max(1)).unwrap_or(0);
        let retry_after_secs = (window + 1)
            .saturating_mul(window_secs)
            .saturating_sub(now_secs);

        let mut state = self.state.lock().await;
        // Opportunistic GC: drop every counter that isn't in the current
        // window. Cheap (one pass) and bounded by the number of distinct keys
        // seen this process lifetime. We do it before inserting so the in-flight
        // key is never itself collected.
        if !state.is_empty() {
            state.retain(|_, c| c.window == window);
        }

        let entry = state.entry(key.to_string()).or_insert(WindowCount { window, count: 0 });
        // The window may have rolled since this slot was last touched; if so,
        // reset its count before crediting this request.
        if entry.window != window {
            entry.window = window;
            entry.count = 0;
        }
        entry.count += 1;
        let count = entry.count;

        if count > per_window as u64 {
            Verdict::Block { retry_after_secs }
        } else if count >= warn_at.max(1) as u64 {
            Verdict::Warn
        } else {
            Verdict::Allow
        }
    }

    /// Number of distinct keys currently tracked. Exposed for tests + a future
    /// `/health`-style observability hook.
    #[cfg(test)]
    pub async fn tracked_keys(&self) -> usize {
        self.state.lock().await.len()
    }
}

/// Key prefixes for the three metered dimensions. Embedded in the limiter key
/// so a session id and a device id that happen to collide as strings still get
/// independent counters.
pub mod keys {
    pub fn session(s: &str) -> String {
        format!("session:{s}")
    }
    pub fn device(s: &str) -> String {
        format!("device:{s}")
    }
    pub fn ip(s: &str) -> String {
        format!("ip:{s}")
    }
}

/// Convenience: a [`Duration`] derived from `Retry-After` seconds, capped so a
/// caller just over the line isn't told to wait an entire window if the window
/// is about to roll. Unused outside the middleware currently; kept for parity.
#[allow(dead_code)]
pub fn retry_after(verdict: Verdict) -> Duration {
    match verdict {
        Verdict::Block { retry_after_secs } => Duration::from_secs(retry_after_secs),
        _ => Duration::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    #[tokio::test]
    async fn allows_under_warn_then_warns_then_blocks_in_one_window() {
        let l = Limiter::default();
        let t = now();
        // per_window=5, warn_at=4: requests 1..3 Allow, 4..5 Warn, 6+ Block.
        assert_eq!(l.check("ip:a", 5, 4, 60, t).await, Verdict::Allow);
        assert_eq!(l.check("ip:a", 5, 4, 60, t).await, Verdict::Allow);
        assert_eq!(l.check("ip:a", 5, 4, 60, t).await, Verdict::Allow);
        assert_eq!(l.check("ip:a", 5, 4, 60, t).await, Verdict::Warn);
        assert_eq!(l.check("ip:a", 5, 4, 60, t).await, Verdict::Warn);
        let v = l.check("ip:a", 5, 4, 60, t).await;
        assert!(matches!(v, Verdict::Block { .. }), "6th request must block: {v:?}");
    }

    #[tokio::test]
    async fn counter_resets_across_window_boundary() {
        let l = Limiter::default();
        let t0 = now();
        // Exhaust the window (5/5, last is Warn).
        for _ in 0..5 {
            l.check("ip:b", 5, 4, 60, t0).await;
        }
        // Move into the next window (60s later): counter resets, request 1 Allow.
        let t1 = t0 + Duration::from_secs(60);
        assert_eq!(l.check("ip:b", 5, 4, 60, t1).await, Verdict::Allow);
    }

    #[tokio::test]
    async fn distinct_dimensions_are_independent_counters() {
        let l = Limiter::default();
        let t = now();
        // Same principal string under different key prefixes must not collide.
        for _ in 0..5 {
            l.check(&keys::session("alice"), 5, 4, 60, t).await;
        }
        // Alice's session is exhausted; her device (same string) is a fresh bucket.
        assert_eq!(
            l.check(&keys::device("alice"), 5, 4, 60, t).await,
            Verdict::Allow
        );
        // ...and her IP yet another.
        assert_eq!(l.check(&keys::ip("alice"), 5, 4, 60, t).await, Verdict::Allow);
    }

    #[tokio::test]
    async fn zero_per_window_is_unlimited() {
        let l = Limiter::default();
        let t = now();
        for _ in 0..100 {
            assert_eq!(l.check("ip:c", 0, 0, 60, t).await, Verdict::Allow);
        }
    }

    #[tokio::test]
    async fn stale_prior_window_entries_evicted() {
        let l = Limiter::new(8);
        let t0 = now();
        // Populate a prior window with many distinct keys (a rotating-identity
        // attack shape).
        for i in 0..20 {
            l.check(&format!("ip:rot-{i}"), 5, 4, 60, t0).await;
        }
        assert_eq!(l.tracked_keys().await, 20);
        // A check in the next window evicts every prior-window key; only the
        // newly-touched key survives.
        let t1 = t0 + Duration::from_secs(60);
        l.check("ip:fresh", 5, 4, 60, t1).await;
        assert_eq!(l.tracked_keys().await, 1, "prior-window keys must be evicted");
    }

    #[tokio::test]
    async fn block_carries_remaining_window_seconds() {
        let l = Limiter::default();
        // Pick a window boundary the limiter agrees with: window = now // 60.
        // We want `now` to sit 10s into a 60s window, so retry-after = 50s.
        // 1_700_000_020 // 60 = 28_333_333 (×60 = 1_699_999_980) → 40s in. Use a
        // value that lands exactly 10s past a boundary instead.
        let window_start = 1_699_999_980_u64; // divisible by 60 → a window edge
        let t = UNIX_EPOCH + Duration::from_secs(window_start + 10);
        for _ in 0..5 {
            l.check("ip:d", 5, 4, 60, t).await;
        }
        match l.check("ip:d", 5, 4, 60, t).await {
            Verdict::Block { retry_after_secs } => {
                assert_eq!(retry_after_secs, 50, "60s window - 10s elapsed = 50s remaining");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }
}
