//! Client-identity resolution for rate limiting.
//!
//! A request is metered along three independent dimensions (session / device /
//! IP); this module resolves each from the incoming request. Resolution is
//! best-effort and degrades gracefully: a missing session or device simply
//! means that dimension is skipped (only IP always resolves).

use axum::http::{HeaderMap, HeaderValue};
use std::net::SocketAddr;

/// The resolved identity of one request, for metering purposes. Any of
/// `session` / `device` may be `None` (caller skips that dimension); `ip` is
/// always present — it falls back to a sentinel when no peer is reachable
/// (e.g. in unit tests that bypass `ConnectInfo`).
#[derive(Debug, Clone)]
pub struct ClientIdentity {
    pub session: Option<String>,
    pub device: Option<String>,
    pub ip: String,
}

/// Sentinel IP used when no peer address is available. Metered as its own
/// bucket so test/non-connect-info traffic is isolated rather than masquerading
/// as some real address.
pub const UNKNOWN_IP: &str = "0.0.0.0";

/// Read the `Bearer {token}` value from `Authorization`, if present and shaped
/// correctly. Shared by the rate-limit middleware and the existing `/auth/me`
/// handler (kept here so the limiter doesn't depend on `routes`).
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let auth = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Read the pseudo-device id from `X-Reader-Id` (the established client UUID
/// convention). Trimmed; empty → None.
pub fn device_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("X-Reader-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Resolve the client IP for rate-limit metering.
///
/// Order:
/// 1. `X-Forwarded-For` — when `trusted_proxies > 0`, the request is assumed to
///    have arrived through that many trusted proxy hops. XFF lists the
///    original client first, then each successive proxy; the leftmost entries
///    are client-controlled, so we take the entry at position
///    `len - trusted_proxies` (the rightmost untrusted one). When
///    `trusted_proxies == 0` we ignore XFF entirely and trust the peer only —
///    the safe default for direct exposure.
/// 2. The TCP peer address from `ConnectInfo<SocketAddr>`.
/// 3. [`UNKNOWN_IP`] sentinel when neither is present.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trusted_proxies: u32) -> String {
    if trusted_proxies > 0 {
        if let Some(ip) = forwarded_client_ip(headers, trusted_proxies) {
            return ip;
        }
    }
    peer.map(|sa| sa.ip().to_string()).unwrap_or_else(|| UNKNOWN_IP.to_string())
}

/// Pick the trustworthy client entry out of an `X-Forwarded-For` header, given
/// `trusted_proxies` hops between the client and us.
///
/// XFF is `client, proxy1, proxy2, ...`. If we trust N proxies, the rightmost N
/// entries are those trusted proxies (who appended themselves); the entry just
/// left of them — index `len - 1 - N` — is the first hop we DON'T control,
/// i.e. the client (or the last untrusted proxy). Returns None on a malformed
/// or too-short chain (caller falls back to the peer address).
fn forwarded_client_ip(headers: &HeaderMap, trusted_proxies: u32) -> Option<String> {
    let hv: HeaderValue = headers.get("X-Forwarded-For")?.clone();
    let raw = hv.to_str().ok()?;
    let entries: Vec<&str> = raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    // XFF is `client, proxy1, ..., proxyN`. We trust `trusted_proxies` of the
    // trailing entries as our own proxies; the client is the entry just before
    // them, i.e. at index `len - 1 - trusted_proxies`. If the chain isn't long
    // enough to contain that many proxies PLUS a client, we can't trust it —
    // return None so the caller falls back to the peer address (an attacker who
    // controls too few hops can't spoof via a short chain).
    let client_idx = entries.len().checked_sub(trusted_proxies as usize + 1)?;
    Some(entries[client_idx].to_string())
}

/// Resolve the full identity for one request, given the headers and (optionally)
/// the peer address. The `ConnectInfo` extractor is applied by the caller (the
/// middleware); this fn stays free of axum-extractor magic so it is unit-testable
/// with plain `HeaderMap`s.
pub fn resolve(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: u32,
) -> ClientIdentity {
    ClientIdentity {
        session: bearer_token(headers),
        device: device_id(headers),
        ip: client_ip(headers, peer, trusted_proxies),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn hdr(name: &str, val: &str) -> HeaderMap {
        // HeaderMap::insert needs an owned HeaderName; headers are case-
        // insensitive on lookup, so lowercase the input (from_lowercase rejects
        // anything non-lowercase). Lookup in client_ip/etc. is by lowercase too.
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::HeaderName::from_lowercase(name.to_ascii_lowercase().as_bytes()).unwrap(),
            HeaderValue::from_str(val).unwrap(),
        );
        h
    }
    fn peer() -> SocketAddr {
        "203.0.113.9:5000".parse().unwrap()
    }

    #[test]
    fn session_from_bearer() {
        assert_eq!(bearer_token(&hdr("Authorization", "Bearer abc123")).as_deref(), Some("abc123"));
        // Not a bearer scheme → None.
        assert!(bearer_token(&hdr("Authorization", "Basic abc123")).is_none());
        // Empty token → None.
        assert!(bearer_token(&hdr("Authorization", "Bearer ")).is_none());
    }

    #[test]
    fn device_from_reader_id() {
        assert_eq!(
            device_id(&hdr("X-Reader-Id", "  uuid-7  ")).as_deref(),
            Some("uuid-7"),
            "trimmed"
        );
        assert!(device_id(&hdr("X-Reader-Id", "   ")).is_none(), "blank → None");
        assert!(device_id(&HeaderMap::new()).is_none(), "missing → None");
    }

    #[test]
    fn ip_uses_peer_when_no_trusted_proxies() {
        // trusted_proxies=0 → XFF is ignored, peer is authoritative.
        let h = hdr("X-Forwarded-For", "1.2.3.4");
        assert_eq!(client_ip(&h, Some(peer()), 0), "203.0.113.9");
    }

    #[test]
    fn ip_uses_xff_when_proxies_trusted() {
        // One trusted proxy: XFF = [client, proxy]; take index len-1-1 = 0 → client.
        let h = hdr("X-Forwarded-For", "198.51.100.7, 10.0.0.1");
        assert_eq!(client_ip(&h, Some(peer()), 1), "198.51.100.7");
    }

    #[test]
    fn ip_xff_picks_last_untrusted_hop() {
        // Two trusted proxies: XFF = [client, p1, p2]; take index 3-1-2 = 0.
        let h = hdr("X-Forwarded-For", "198.51.100.7, 10.0.0.1, 10.0.0.2");
        assert_eq!(client_ip(&h, Some(peer()), 2), "198.51.100.7");
    }

    #[test]
    fn ip_xff_too_short_falls_back_to_peer() {
        // trusted_proxies=3 but only 1 XFF entry → can't be all proxies → peer.
        let h = hdr("X-Forwarded-For", "10.0.0.1");
        assert_eq!(client_ip(&h, Some(peer()), 3), "203.0.113.9");
    }

    #[test]
    fn ip_no_xff_no_peer_is_sentinel() {
        assert_eq!(client_ip(&HeaderMap::new(), None, 0), UNKNOWN_IP);
    }

    #[test]
    fn resolve_composes_all_three() {
        let h = {
            let mut m = hdr("Authorization", "Bearer sesh");
            m.insert("X-Reader-Id", HeaderValue::from_str("dev1").unwrap());
            m.insert("X-Forwarded-For", HeaderValue::from_str("9.9.9.9, 10.0.0.1").unwrap());
            m
        };
        let id = resolve(&h, Some(peer()), 1);
        assert_eq!(id.session.as_deref(), Some("sesh"));
        assert_eq!(id.device.as_deref(), Some("dev1"));
        assert_eq!(id.ip, "9.9.9.9");
    }

    #[test]
    fn resolve_degrades_to_ip_only() {
        let id = resolve(&HeaderMap::new(), Some(peer()), 0);
        assert!(id.session.is_none());
        assert!(id.device.is_none());
        assert_eq!(id.ip, "203.0.113.9");
    }
}
