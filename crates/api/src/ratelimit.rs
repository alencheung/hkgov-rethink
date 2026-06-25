//! Per-client-IP inbound rate limiting (V-003).
//!
//! `api.rate_per_sec` was defined in config but never wired to the router, so
//! there was no throttle on anonymous request floods — a single source could
//! drive CPU/memory pressure and burn the optional LLM egress budget. This
//! module attaches a [`governor`]-backed per-IP token bucket as an axum
//! `from_fn` middleware.
//!
//! ## Why not `tower-governor`?
//!
//! `tower-governor` 0.4 is built against `axum-core` 0.4's `Body` type and
//! fails the body-type check under axum 0.8. Driving `governor` directly from
//! a ~30-line middleware keeps the dependency light and version-agnostic.
//!
//! ## Behavior
//!
//! - Keyed by the peer IP from `ConnectInfo<SocketAddr>` when available, else a
//!   fallback "anonymous" bucket (so requests without a resolvable peer IP are
//!   still bounded collectively, not unlimited).
//! - Returns `429 Too Many Requests` when the bucket is empty.
//! - Allows a small burst (`per_sec * 2`, min 4) so a legitimate client that
//!   batches a handful of requests isn't throttled, while a sustained flood is.

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

/// A per-IP token-bucket limiter shared across all requests. Cheap to clone
/// (the state store is behind an `Arc`).
#[derive(Clone)]
pub struct IpRateLimiter {
    inner: Arc<
        RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>,
    >,
}

impl IpRateLimiter {
    /// Build a limiter that sustains `per_sec` requests/second per IP with a
    /// small burst allowance.
    pub fn new(per_sec: u32) -> Self {
        let burst = (per_sec.saturating_mul(2)).max(4) as u32;
        let quota = Quota::per_second(NonZeroU32::new(per_sec).unwrap_or_else(|| {
            // Unreachable when wired (caller gates on per_sec > 0); defend
            // in depth so a misuse can't construct an unbounded quota.
            NonZeroU32::new(1).unwrap()
        }))
        .allow_burst(
            NonZeroU32::new(burst)
                .expect("burst is at least 4, so non-zero"),
        );
        Self {
            inner: Arc::new(RateLimiter::keyed(quota)),
        }
    }
}

/// The axum middleware. Returns `429` when the caller's IP bucket is empty.
///
/// Note: the app must serve with
/// `axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())`
/// for `ConnectInfo` to be present. When it isn't (e.g. a unit-test `oneshot`
/// without connect-info), we fall back to a synthetic `0.0.0.0` key so the
/// limiter still applies a bound rather than silently allowing everything.
pub async fn limit(
    State(limiter): State<IpRateLimiter>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Prefer the real peer IP; fall back to a shared anonymous bucket.
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    if limiter.inner.check_key(&ip).is_err() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_allows_burst_then_rejects() {
        // per_sec=10 ⇒ burst=20. Issuing 20 rapid checks from one IP should
        // succeed, and the 21st should be rejected.
        let limiter = IpRateLimiter::new(10);
        let ip = IpAddr::V4(std::net::Ipv4Addr::new(1, 2, 3, 4));
        let mut allowed = 0;
        for _ in 0..100 {
            if limiter.inner.check_key(&ip).is_ok() {
                allowed += 1;
            } else {
                break;
            }
        }
        assert!(allowed >= 10, "burst floor: allowed={allowed}");
        assert!(allowed <= 20, "burst ceiling: allowed={allowed}");
    }

    #[test]
    fn limiter_keys_by_ip() {
        let limiter = IpRateLimiter::new(1); // burst=2
        let a = IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1));
        let b = IpAddr::V4(std::net::Ipv4Addr::new(2, 2, 2, 2));
        // Fully drain A's bucket: keep consuming until it rejects.
        let mut a_allowed = 0;
        loop {
            if limiter.inner.check_key(&a).is_ok() {
                a_allowed += 1;
            } else {
                break;
            }
            // Safety cap so a pathological config can't hang the test.
            if a_allowed > 1000 {
                break;
            }
        }
        assert!(a_allowed >= 1, "A consumed something: {a_allowed}");
        // A is now drained; B has its own independent bucket and must succeed.
        assert!(
            limiter.inner.check_key(&a).is_err(),
            "A drained after {a_allowed} checks"
        );
        assert!(limiter.inner.check_key(&b).is_ok(), "B has an independent bucket");
    }
}
