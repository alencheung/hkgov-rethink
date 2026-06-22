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
    //
    // Match EXACT paths only. This guard lives on `api_routes`, which axum
    // mounts under the configured prefix (stripping it for inner middleware),
    // so the health endpoints always resolve to `/health` and `/health/sources`
    // here regardless of the prefix. A suffix/substring check would be an auth
    // bypass: `/v1/datasets/hkma/health` ends with `/health` and
    // `/v1/datasets/health/records` contains `/health/`, but both are data
    // routes that must require a key (D-005).
    let path = req.uri().path();
    if path == "/" || path == "/health" || path == "/health/sources" {
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

    // ---- D-005 regression: auth bypass via path suffix/substring matching ----
    //
    // The exemption list must match EXACT health paths. Earlier the guard used
    // `ends_with("/health")` and `contains("/health/")`, which let unauthenticated
    // requests through on any data route whose path happened to end in `/health`
    // or contain `/health/` — e.g. `/v1/datasets/hkma/health`. axum's `Next` is
    // not publicly constructible, so we exercise the guard through the real
    // router() (which wires the middleware) — a more faithful test anyway.

    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Build a key-enabled router over a seeded store for auth tests.
    fn key_enabled_router() -> axum::Router {
        use crate::routes::router;
        use crate::state::AppState;
        use hkgov_common::Settings;

        let mut settings = Settings::default();
        settings.api.api_key = Some("secret".into());
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let store = Arc::new(hkgov_store::MemoryStore::new(10, 60));
        // Seed a real dataset so a 200 on it is a genuine hit, not an empty store.
        let id = hkgov_store::DatasetId::new(
            hkgov_common::DataSource::Hkma,
            "daily-interbank-liquidity",
        );
        // Register synchronously via a runtime (the test fns are not async).
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(store.register(
            id,
            "Daily Interbank Liquidity".into(),
            None,
            3600,
            hkgov_common::Category::Monetary,
            vec!["hibor".into()],
            hkgov_common::Cadence::Daily,
        ));
        let state = AppState {
            registry,
            store,
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            llm: Arc::new(hkgov_agent::HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            settings: Arc::new(settings),
        };
        router(state)
    }

    /// GET a path through the router and return the status code.
    fn status_for(router: axum::Router, path: &str, key: Option<&str>) -> u16 {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut b = axum::http::Request::builder().uri(path);
            if let Some(k) = key {
                b = b.header("X-API-Key", k);
            }
            let resp = router
                .oneshot(b.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = resp.status().as_u16();
            let _ = resp.into_body().collect().await;
            status
        })
    }

    #[test]
    fn health_paths_exempt_without_key() {
        let app = key_enabled_router();
        assert_eq!(status_for(app.clone(), "/health", None), 200);
        assert_eq!(status_for(app.clone(), "/v1/health/sources", None), 200);
        assert_eq!(status_for(app, "/", None), 200);
    }

    #[test]
    fn dataset_route_named_health_requires_key() {
        // D-005: `/v1/datasets/hkma/health` ends with `/health` but is a data
        // route — must require a key (was 200 before the fix).
        let app = key_enabled_router();
        assert_eq!(
            status_for(app.clone(), "/v1/datasets/hkma/health", None),
            401,
            "D-005: dataset named *health must require a key"
        );
        // The records variant (contains `/health/`) must also require a key.
        assert_eq!(
            status_for(app.clone(), "/v1/datasets/hkma/health/records", None),
            401
        );
        // A correct key unlocks it. `dataset_meta` returns 200 + `null` for an
        // unregistered dataset (unknown datasets aren't a 404 at the meta layer),
        // so the assertion is "not 401" — auth passed.
        let authed = status_for(app, "/v1/datasets/hkma/health", Some("secret"));
        assert_ne!(authed, 401, "correct key must pass auth (got {authed})");
    }

    #[test]
    fn normal_protected_routes_require_key() {
        let app = key_enabled_router();
        assert_eq!(status_for(app.clone(), "/v1/sources", None), 401);
        assert_eq!(status_for(app.clone(), "/v1/sources", Some("secret")), 200);
        assert_eq!(status_for(app, "/v1/insights", None), 401);
    }

    #[test]
    fn wrong_key_rejected() {
        let app = key_enabled_router();
        assert_eq!(status_for(app, "/v1/sources", Some("wrong")), 401);
    }
}
