//! Optional API-key auth middleware.
//!
//! When `api.api_key` is set in config, every request must carry the key in the
//! `X-API-Key` header (or `?api_key=` query). `/health` and `/health/*`
//! are always exempt so liveness probes work unauthenticated.
//!
//! When no key is configured, the middleware passes everything through — this
//! keeps local dev zero-config.

use axum::extract::Request;
use axum::http::{StatusCode, Uri};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

/// Returns the shared expected key, or None when auth is disabled.
pub fn make_guard(expected: Option<String>) -> Option<Arc<str>> {
    expected.filter(|s| !s.is_empty()).map(Into::into)
}

/// Build an axum `from_fn` middleware closure bound to the expected key.
/// Used as: `.layer(from_fn(move |req, next| auth::guard(key.clone(), req, next)))`
pub async fn guard(expected: Arc<str>, req: Request, next: Next) -> Result<Response, StatusCode> {
    // Always allow health/root endpoints (liveness must work without auth).
    let path = req.uri().path();
    if path.ends_with("/health") || path.contains("/health/") || path == "/" {
        return Ok(next.run(req).await);
    }

    let provided = provided_key(&req);
    match provided {
        Some(p) if p == expected.as_ref() => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

fn provided_key(req: &Request) -> Option<String> {
    req.headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            let uri: &Uri = req.uri();
            uri.query()
                .and_then(|q| url_query_value(q, "api_key").map(str::to_string))
        })
}

fn url_query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some(key) {
            return it.next();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_key_query() {
        assert_eq!(
            url_query_value("foo=1&api_key=secret&bar=2", "api_key"),
            Some("secret")
        );
        assert_eq!(url_query_value("api_key=", "api_key"), Some(""));
        assert_eq!(url_query_value("foo=1", "api_key"), None);
    }
}
