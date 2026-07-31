//! Route definitions and the tower middleware stack.
//!
//! Surface (v6):
//!   GET  /health                      — liveness
//!   GET  /health/sources              — per-source circuit breaker state
//!   GET  /sources                     — list ingested datasets
//!   GET  /market-players?dept=&category= — curated related-market-players directory
//!   GET  /datasets/{source}/{dataset} — dataset metadata
//!   GET  /datasets/{source}/{dataset}/records?offset=&limit=
//!                                    — paginated records from cache
//!   GET  /insights?limit=             — AI-agent generated insights
//!   GET  /alerts?limit=               — proactive alert dispatch log
//!   POST /ask                         — natural-language Q&A over the data

mod audit;
mod auth_routes;
mod gateway;
mod investigations;
mod property;
mod signals;

// Bring the extracted handlers into scope so `router()` can reference them.
use audit::{attestation, insight_provenance, list_audit};
use auth_routes::{auth_me, bearer_token, redeem_auth_token, request_auth_token};
use gateway::{dataset_lineage, list_lineage, register_dataset};
use investigations::{
    add_investigation_note, append_investigation_step, create_investigation, delete_investigation,
    get_investigation, list_investigations,
};
use property::{property_composite, property_divergence, property_portals};
use signals::{
    create_signal, delete_signal, get_signal, list_signals, preview_signal_route, update_signal,
};

use crate::auth::{guard, make_guard};
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hkgov_agent::{
    heuristic_answer, run_agent_loop, Answer, HeuristicClient, LlmClient, ToolBelt, UserStore,
};
use hkgov_common::DataSource;
use hkgov_store::{DatasetId, RecordStore};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub fn router(state: AppState) -> Router {
    let timeout = Duration::from_millis(state.settings.api.request_timeout_ms);
    let api_key = make_guard(state.settings.api.api_key.clone());
    let prefix = state.settings.api.api_prefix.trim_matches('/').to_string();

    // The versioned API routes. State is applied here so the nested router is
    // fully resolved before it's mounted under the prefix.
    //
    // Cache policy (Phase 3 of the daily-view perf work): the hero read
    // endpoints are split into a `cacheable_routes` group that gets a
    // `Cache-Control: public, max-age=N, stale-while-revalidate=M` layer; the
    // rest inherit the global `no-store` default set further down. The global
    // `SetResponseHeaderLayer::if_not_present` means a route-level layer that
    // sets the header first wins — so the hero group opts into caching while
    // auth/mutations/health stay no-store.
    //
    // `max-age=300` (5 min) matches the dashboard's "good enough" freshness
    // bar; the snapshot regenerates every 6h so a 5-min browser cache can
    // never serve data older than one snapshot cycle. Records get a shorter
    // `max-age=60` (1 min) because the moka cache keeps shifting under them as
    // warmup progresses.
    let cacheable_hero = Router::new()
        .route("/silence-index", get(silence_index))
        .route("/transparency-index", get(transparency_index))
        .route("/transparency-index/report", get(transparency_report_route))
        .route("/brief", get(get_brief))
        .route("/property/composite", get(property_composite))
        .route("/property/portals", get(property_portals))
        .route("/property/divergence", get(property_divergence))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=300, stale-while-revalidate=600"),
        ));
    let cacheable_records = Router::new()
        .route("/datasets/{source}/{dataset}/records", get(dataset_records))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        ));

    let mut api_routes = Router::new()
        .route("/health", get(health))
        .route("/health/sources", get(health_sources))
        .route("/sources", get(list_sources))
        .route("/categories", get(list_categories))
        .route("/market-players", get(list_market_players))
        .route("/datasets", post(register_dataset))
        .route("/datasets/{source}/{dataset}", get(dataset_meta))
        .route("/datasets/{source}/{dataset}/lineage", get(dataset_lineage))
        .route("/lineage", get(list_lineage))
        .route("/insights", get(list_insights))
        .route(
            "/insights/{id}/feedback",
            post(submit_feedback).get(get_feedback),
        )
        .route("/insights/{id}/cite", get(cite_insight))
        .route("/insights/{id}/history", get(insight_history))
        .route("/insights/{id}/provenance", get(insight_provenance))
        .route("/audit", get(list_audit))
        .route("/audit/attestation/{id}", get(attestation))
        .route("/alerts", get(list_alerts))
        .route("/unprecedentedness", get(unprecedentedness))
        .route("/signals", post(create_signal).get(list_signals))
        .route("/signals/preview", post(preview_signal_route))
        .route(
            "/signals/{id}",
            get(get_signal).delete(delete_signal).patch(update_signal),
        )
        .route(
            "/investigations",
            post(create_investigation).get(list_investigations),
        )
        .route(
            "/investigations/{id}",
            get(get_investigation).delete(delete_investigation),
        )
        .route(
            "/investigations/{id}/steps",
            post(append_investigation_step),
        )
        .route("/investigations/{id}/notes", post(add_investigation_note))
        .route("/auth/request-token", post(request_auth_token))
        .route("/auth/redeem", post(redeem_auth_token))
        .route("/auth/me", get(auth_me))
        .route("/ask", post(ask))
        .merge(cacheable_hero)
        .merge(cacheable_records);

    if let Some(key) = api_key {
        api_routes = api_routes.layer(from_fn(move |req, next| {
            let key = key.clone();
            async move { guard(key, req, next).await }
        }));
    }

    // Root routes (stateless root info + LB-probe health), with the versioned
    // API nested under the prefix.
    //
    // When a prefix is set (the default `/v1`), we mount a root `/health` for
    // LB/k8s probes and a `/` directory, then nest the API under the prefix.
    // When the prefix is empty, the API routes merge into the root — and since
    // `api_routes` already defines `/health`, we must NOT add a second root
    // `/health` here or axum panics with "Overlapping method route".
    //
    // `/dashboard` serves the static insights dashboard (embedded at compile
    // time via include_str!) so the binary — and the Docker image — are
    // self-contained: open http://host:port/dashboard in a browser. It is
    // exempt from API-key auth (a static asset, not data).
    //
    // `/cite/{id}` serves the *same* dashboard HTML so a Cite-It permalink
    // (emitted as `/cite/{insight_id}` by `build_citation`) resolves to the
    // app instead of 404ing on the Rust-served deploy. The dashboard's
    // `checkShareLanding()` reads the path and opens the citation drawer
    // client-side. Auth-exempt for the same reason as `/dashboard`: it is a
    // static asset, not a data route. (Netlify deploys get the equivalent via
    // the SPA rewrite in netlify.toml.)
    let router = Router::new()
        .route("/", get(root))
        .route("/dashboard", get(dashboard))
        .route("/dashboard/", get(dashboard))
        .route("/cite/{id}", get(dashboard))
        // Dashboard JS modules (split from app.js for navigability). Embedded at
        // compile time, same as the dashboard HTML. Auth-exempt (static assets).
        .route("/api.js", get(dashboard_js_api))
        .route("/i18n.js", get(dashboard_js_i18n))
        .route("/features.js", get(dashboard_js_features))
        .route("/pages.js", get(dashboard_js_pages))
        .route("/boot.js", get(dashboard_js_boot))
        // /llms.txt — a curated agent index for the llms.txt convention (and the
        // kind of predictable, crawlable text surface Cloudflare's "Markdown for
        // Agents" model targets). Embedded at compile time, same as the
        // dashboard, and exempt from API-key auth (a static index, not data).
        .route("/llms.txt", get(llms_txt));
    let router = if prefix.is_empty() {
        // api_routes already carries `/health`; merge brings it to root.
        router.merge(api_routes).route("/ready", get(ready))
    } else {
        // Nested: root `/health` + `/ready` for probes, api_routes under /{prefix}.
        router
            .route("/health", get(health))
            .route("/ready", get(ready))
            .nest(&format!("/{prefix}"), api_routes)
    };

    let router = router
        .with_state(state.clone())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(CompressionLayer::new())
        // SEC-API-05: cap inbound request bodies. Without this every `Json<T>`
        // handler (ask, feedback, signals, investigations, …) accepted an
        // unbounded body — an attacker could POST arbitrarily large payloads to
        // burn memory/LLM budget. 256 KiB is generous for every JSON payload
        // this API accepts (the largest legitimate one is a signal preview);
        // file/multipart upload isn't supported.
        .layer(tower_http::limit::RequestBodyLimitLayer::new(256 * 1024));

    // V-003: per-IP inbound rate limiting. `api.rate_per_sec` was previously
    // dead config — defined in config.rs but never wired to the router, so
    // there was no throttle on anonymous request floods. When the operator
    // sets a non-zero rate, attach a per-IP token-bucket middleware. When 0
    // (the default, for back-compat) we skip it to keep local dev unlimited.
    let router = if state.settings.api.rate_per_sec > 0 {
        let limiter = crate::ratelimit::IpRateLimiter::new(state.settings.api.rate_per_sec);
        router.layer(from_fn_with_state(limiter, crate::ratelimit::limit))
    } else {
        router
    };

    // Concurrency load-shedding. `api.max_concurrency` was previously dead
    // config (the same class of defect as the old `store.backend` /
    // `rate_per_sec`): defined in config.rs and advertised in README/
    // CAPACITY/ARCHITECTURE, but never wired to the router, so a connection or
    // slow-downstream flood could saturate the node to exhaustion with no
    // shedding. Cap in-flight requests; once the limit is reached the next
    // request is shed with 503 (tower's default for a saturated limit). The
    // default (50_000) is permissive enough to be a safety net, not a ceiling
    // under normal load — operators tune it to fleet capacity.
    let router = router.layer(ConcurrencyLimitLayer::new(
        state.settings.api.max_concurrency,
    ));

    router
        // V-007: CORS is now an operator allow-list, not permissive. Empty
        // `cors_origins` (the default) ⇒ same-origin only (no ACAO). Each
        // configured origin is matched exactly. (PR-007: a duplicate layer was
        // applied here twice — removed; one layer is correct and idempotent.)
        .layer(cors_layer(&state.settings.api.cors_origins))
        // V-008: security headers on every response — defense-in-depth behind
        // the (already-correct) output escaping. See `security_header_layers`.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                // Dashboard loads uPlot + remixicon from jsdelivr; everything
                // else is 'self'. No 'unsafe-eval'. frame-ancestors blocks
                // clickjacking. This blocks injected third-party loads even if
                // a future code change introduces an unescaped DOM sink.
                "default-src 'self'; \
                 script-src 'self' https://cdn.jsdelivr.net; \
                 style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
                 img-src 'self' data:; \
                 font-src 'self' https://cdn.jsdelivr.net; \
                 connect-src 'self'; \
                 frame-ancestors 'none'",
            ),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            // `Permissions-Policy` has no const in axum's header module; use the
            // literal name. Deny the high-risk device/browser features by default.
            axum::http::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ))
        .layer(TraceLayer::new_for_http())
}

/// Build the CORS layer from the configured allow-list. V-007: replaced
/// `CorsLayer::permissive()` (`Access-Control-Allow-Origin: *`). Empty list ⇒
/// same-origin only (no ACAO emitted). Non-empty ⇒ exact-origin matching.
fn cors_layer(origins: &[String]) -> CorsLayer {
    if origins.is_empty() {
        // No cross-origin access by default. A `CorsLayer` with no allowed
        // origin suppresses the ACAO header.
        CorsLayer::new()
    } else {
        let parsed: Vec<HeaderValue> = origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        CorsLayer::new().allow_origin(AllowOrigin::list(parsed))
    }
}

/// Resolve the authenticated principal's stable id from the request's
/// `Authorization: Bearer {session}` header. V-004/V-005 fix: the per-user
/// features (signals, investigations) used to take `owner` from the request
/// BODY — so any caller could claim to be any user, and an empty owner listed
/// every user's records. The owner is now derived from the authenticated
/// session; there is no way for the body to set it.
async fn principal_id(users: &UserStore, headers: &HeaderMap) -> Option<String> {
    let token = bearer_token(headers)?;
    let user = users.lookup_session(&token).await?;
    Some(user.id)
}

/// Require an authenticated principal, returning its id, or a 401. Used by the
/// mutating routes so they can never operate without a resolved owner. Returns
/// `Unauthorized` (401), not `BadRequest` (400): a missing/invalid credential is
/// a 401 condition, distinct from a malformed request.
fn require_principal(id: Option<String>) -> Result<String, ApiError> {
    id.ok_or_else(|| {
        ApiError(hkgov_common::Error::Unauthorized(
            "authentication required: send a valid Authorization: Bearer {session} \
             (obtain one from POST /v1/auth/request-token + /v1/auth/redeem)"
                .into(),
        ))
    })
}

#[derive(Serialize)]
struct Root {
    name: &'static str,
    version: &'static str,
    endpoints: &'static [&'static str],
}

async fn root(State(_): State<AppState>) -> Json<Root> {
    Json(Root {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        endpoints: &[
            "GET /health",
            "GET /dashboard",
            "GET /llms.txt",
            "GET /v1/health/sources",
            "GET /v1/sources",
            "GET /v1/categories",
            "GET /v1/market-players",
            "GET /v1/property/composite",
            "GET /v1/property/portals",
            "GET /v1/property/divergence",
            "POST /v1/datasets",
            "GET /v1/datasets/{source}/{dataset}",
            "GET /v1/datasets/{source}/{dataset}/records",
            "GET /v1/datasets/{source}/{dataset}/lineage",
            "GET /v1/lineage",
            "GET /v1/insights",
            "POST /v1/insights/{id}/feedback",
            "GET /v1/insights/{id}/cite",
            "GET /v1/insights/{id}/history",
            "GET /v1/insights/{id}/provenance",
            "GET /v1/audit",
            "GET /v1/audit/attestation/{id}",
            "GET /v1/brief",
            "GET /v1/alerts",
            "GET /v1/silence-index",
            "GET /v1/transparency-index",
            "GET /v1/transparency-index/report",
            "GET /v1/unprecedentedness",
            "POST /v1/signals",
            "GET /v1/signals",
            "POST /v1/signals/preview",
            "GET /v1/signals/{id}",
            "PATCH /v1/signals/{id}",
            "DELETE /v1/signals/{id}",
            "POST /v1/investigations",
            "GET /v1/investigations",
            "GET /v1/investigations/{id}",
            "DELETE /v1/investigations/{id}",
            "POST /v1/investigations/{id}/steps",
            "POST /v1/investigations/{id}/notes",
            "POST /v1/auth/request-token",
            "POST /v1/auth/redeem",
            "GET /v1/auth/me",
            "POST /v1/ask",
        ],
    })
}

/// Serve the static insights dashboard. The HTML is embedded at compile time
/// (`include_str!`) so the deployed binary — and the Docker image — carry it
/// with no external file dependency. Open `http://host:port/dashboard`.
///
/// The path uses `CARGO_MANIFEST_DIR` (the api crate's directory) + a relative
/// hop to the workspace `dashboard/` dir. This is more robust than
/// `include_str!("../../../../...")` which can resolve incorrectly on network
/// mounts (Z: drive) and doesn't trigger recompilation when the dashboard file
/// changes (cargo doesn't track include_str! dependencies outside src/).
async fn dashboard(State(_): State<AppState>) -> axum::response::Response {
    const HTML: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../dashboard/index.html"
    ));
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        axum::response::Html(HTML),
    )
        .into_response()
}

/// Serve a dashboard JS module. Each module is embedded at compile time (same
/// pattern as `dashboard`). The content type is `application/javascript`.
macro_rules! dashboard_js_handler {
    ($fn_name:ident, $file:literal) => {
        async fn $fn_name(State(_): State<AppState>) -> axum::response::Response {
            const JS: &str = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../dashboard/",
                $file
            ));
            (
                [(
                    axum::http::header::CONTENT_TYPE,
                    "application/javascript; charset=utf-8",
                )],
                JS,
            )
                .into_response()
        }
    };
}

dashboard_js_handler!(dashboard_js_api, "api.js");
dashboard_js_handler!(dashboard_js_i18n, "i18n.js");
dashboard_js_handler!(dashboard_js_features, "features.js");
dashboard_js_handler!(dashboard_js_pages, "pages.js");
dashboard_js_handler!(dashboard_js_boot, "boot.js");

/// Serve the curated agent index (`llms.txt`). This is a single static
/// markdown file that orients AI agents to the app, its data, and its API. It
/// is embedded at compile time (`include_str!`) — the same pattern as
/// `dashboard` — so the deployed binary carries it with no external file
/// dependency. The identical file is also served as a static asset at `/llms.txt`
/// by the dashboard host (e.g. Netlify, which publishes `dashboard/` as its
/// root), so both deploy paths expose the same content with no drift.
///
/// Content type is `text/markdown` (no negotiation: by the llms.txt convention
/// `/llms.txt` is always markdown).
async fn llms_txt(State(_): State<AppState>) -> axum::response::Response {
    const MD: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../dashboard/llms.txt"
    ));
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        MD,
    )
        .into_response()
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    /// PR-004: true when the process is up but degraded — any source circuit is
    /// open, or no dataset has warmed yet. Pure-liveness `/health` always
    /// returns `status:"ok"` (the process answers), but `degraded` lets a load
    /// balancer / readiness rule gate traffic off a broken-warm container
    /// without giving up liveness. `/ready` is the stricter probe that returns
    /// 503 when degraded.
    degraded: bool,
}

/// Is the serving tier healthy enough to receive traffic? PR-004. `degraded`
/// when any upstream circuit is open OR no dataset has warmed yet — both are
/// states where a fresh request cannot be served from the warmed cache.
async fn is_degraded(state: &AppState) -> bool {
    let breakers_open = state
        .registry
        .breaker_states()
        .iter()
        .any(|(_, c)| *c != "closed");
    let warmed = match state.store.list(None).await {
        Ok(datasets) => datasets.iter().any(|d| d.record_count > 0),
        Err(_) => true, // store error ⇒ treat as degraded (don't fail open)
    };
    breakers_open || !warmed
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        degraded: is_degraded(&state).await,
    })
}

/// PR-004: the readiness probe. Returns 200 when the serving tier can actually
/// serve (all breakers closed AND at least one dataset warmed), 503 otherwise.
/// Point load-balancer / k8s readinessProbes here, not at `/health` (which is
/// pure liveness). Body carries the breaker summary for operators.
async fn ready(State(state): State<AppState>) -> axum::response::Response {
    use axum::http::StatusCode;
    let degraded = is_degraded(&state).await;
    #[derive(serde::Serialize)]
    struct ReadyBody {
        status: &'static str,
        degraded: bool,
        sources: Vec<SourceHealth>,
    }
    let sources = health_sources_inner(&state);
    let body = ReadyBody {
        status: if degraded { "degraded" } else { "ready" },
        degraded,
        sources,
    };
    let code = if degraded {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (code, Json(body)).into_response()
}

#[derive(Serialize)]
struct SourceHealth {
    source: String,
    circuit: &'static str,
}

async fn health_sources(State(state): State<AppState>) -> Json<Vec<SourceHealth>> {
    Json(health_sources_inner(&state))
}

/// Shared breaker-state summary used by both `/health/sources` and `/ready`.
fn health_sources_inner(state: &AppState) -> Vec<SourceHealth> {
    state
        .registry
        .breaker_states()
        .into_iter()
        .map(|(s, circuit)| SourceHealth {
            source: s.as_str().to_string(),
            circuit,
        })
        .collect()
}

// ---- GET /sources — filterable dataset catalog ---------------------------

/// Query params for `/sources`. All optional; omitted = no filter. Filters
/// compose with AND across dimensions; `tag` is repeated (matches ANY tag).
///
/// Note: `tag` is intentionally NOT a field here. `serde_urlencoded` (axum's
/// `Query` extractor) rejects both a lone `?tag=hibor` ("expected a sequence"
/// for `Vec<String>`) and a repeated `?tag=a&tag=b` ("duplicate field") for any
/// type — so any `tag` field on this struct breaks one or both forms. Instead
/// `tag` is parsed straight off the raw query string in [`DatasetFilter::tags`],
/// which handles all three forms: single (`?tag=hibor`), repeated
/// (`?tag=a&tag=b`), and comma-separated (`?tag=a,b`).
#[derive(Deserialize, Default)]
struct DatasetFilter {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    cadence: Option<String>,
    /// Free-text substring (case-insensitive) over title + description + id.
    #[serde(default)]
    q: Option<String>,
}

impl DatasetFilter {
    /// Resolve the effective tag list straight from the raw query string.
    /// Handles all three documented forms:
    /// - single: `?tag=hibor`
    /// - repeated: `?tag=hibor&tag=liquidity`
    /// - comma-separated: `?tag=hibor,liquidity`
    fn tags(&self, raw_query: Option<&str>) -> Vec<String> {
        let mut tags: Vec<String> = Vec::new();
        if let Some(q) = raw_query {
            for pair in q.split('&') {
                let mut it = pair.splitn(2, '=');
                if it.next() == Some("tag") {
                    if let Some(v) = it.next() {
                        for t in v.split(',') {
                            let t = t.trim();
                            if !t.is_empty() {
                                tags.push(t.to_owned());
                            }
                        }
                    }
                }
            }
        }
        tags
    }
}

fn dataset_matches(meta: &hkgov_common::DatasetMeta, f: &DatasetFilter, tags: &[String]) -> bool {
    if let Some(ref cat) = f.category {
        if hkgov_common::Category::parse(cat) != Some(meta.category) {
            return false;
        }
    }
    if let Some(ref cad) = f.cadence {
        let want = hkgov_common::Cadence::parse(cad);
        if want.is_none() || want != Some(meta.cadence) {
            return false;
        }
    }
    if !tags.is_empty() && !tags.iter().any(|t| meta.tags.iter().any(|mt| mt == t)) {
        return false;
    }
    if let Some(ref q) = f.q {
        let needle = q.to_ascii_lowercase();
        let haystack = format!(
            "{} {} {}",
            meta.title,
            meta.description.as_deref().unwrap_or(""),
            meta.dataset
        )
        .to_ascii_lowercase();
        if !haystack.contains(&needle) {
            return false;
        }
    }
    true
}

async fn list_sources(
    State(state): State<AppState>,
    Query(f): Query<DatasetFilter>,
    raw: axum::extract::RawQuery,
) -> Result<Json<Vec<hkgov_common::DatasetMeta>>, ApiError> {
    let source = f.source.as_deref().and_then(DataSource::parse);
    let tags = f.tags(raw.0.as_deref());
    let mut all = state.store.list(source).await?;
    if f.category.is_some() || !tags.is_empty() || f.cadence.is_some() || f.q.is_some() {
        all.retain(|m| dataset_matches(m, &f, &tags));
    }
    Ok(Json(all))
}

// ---- GET /categories — the browse entry point -----------------------------

#[derive(Serialize)]
struct CategoryGroup {
    category: String,
    count: usize,
    datasets: Vec<String>,
}

async fn list_categories(
    State(state): State<AppState>,
) -> Result<Json<Vec<CategoryGroup>>, ApiError> {
    let all = state.store.list(None).await?;
    let mut groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for m in all {
        groups
            .entry(m.category.to_string())
            .or_default()
            .push(format!("{}/{}", m.source, m.dataset));
    }
    let out: Vec<CategoryGroup> = groups
        .into_iter()
        .map(|(category, mut datasets)| {
            let count = datasets.len();
            datasets.sort();
            CategoryGroup {
                category,
                count,
                datasets,
            }
        })
        .collect();
    Ok(Json(out))
}

async fn dataset_meta(
    State(state): State<AppState>,
    Path((source, dataset)): Path<(String, String)>,
) -> Result<Json<Option<hkgov_common::DatasetMeta>>, ApiError> {
    let source = parse_source(&source)?;
    let id = DatasetId::new(source, dataset);
    Ok(Json(state.store.meta(&id).await?))
}

/// Filter params for `/v1/market-players`. Both optional and case-insensitive:
/// `?dept=HKMA` joins to the dashboard directory; `?category=monetary` slices by
/// business stream. Either, both, or neither may be set.
#[derive(Deserialize)]
struct PlayerQuery {
    #[serde(default)]
    dept: Option<String>,
    #[serde(default)]
    category: Option<String>,
}

/// `GET /v1/market-players` — the curated "related market players" directory.
///
/// Serves the configured `[reference]` set, falling back to
/// [`hkgov_common::default_market_players`] when none are configured (same
/// empty-means-defaults contract as `agent.scan`). Used by the Licences page
/// to show the named private-sector players holding each department's licences.
async fn list_market_players(
    State(state): State<AppState>,
    Query(q): Query<PlayerQuery>,
) -> Result<Json<Vec<hkgov_common::MarketPlayerGroup>>, ApiError> {
    // Empty config → ship the defaults so out-of-the-box behavior is unchanged.
    let groups: Vec<hkgov_common::MarketPlayerGroup> =
        if state.settings.reference.market_players.is_empty() {
            hkgov_common::default_market_players()
        } else {
            state.settings.reference.market_players.clone()
        };

    let dept = q.dept.as_deref().map(|s| s.to_ascii_uppercase());
    let category = q.category.as_deref().map(|s| s.to_ascii_lowercase());
    let filtered: Vec<_> = groups
        .into_iter()
        .filter(|g| {
            dept.as_deref()
                .is_none_or(|d| g.dept.eq_ignore_ascii_case(d))
        })
        .filter(|g| {
            category
                .as_deref()
                .is_none_or(|c| g.category.as_str().eq_ignore_ascii_case(c))
        })
        .collect();
    Ok(Json(filtered))
}

#[derive(Deserialize)]
struct PageQuery {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

/// Upper bound on `PageQuery.limit` for the records endpoint. A client sending
/// `?limit=1000000000` used to push that straight into the store, exposing it to
/// unbounded materialization. Clamp at the handler boundary.
const MAX_RECORDS_LIMIT: usize = 500;

async fn dataset_records(
    State(state): State<AppState>,
    Path((source, dataset)): Path<(String, String)>,
    Query(q): Query<PageQuery>,
) -> Result<Json<hkgov_store::RecordPage>, ApiError> {
    let limit = q.limit.clamp(1, MAX_RECORDS_LIMIT);
    let source = parse_source(&source)?;
    let id = DatasetId::new(source, dataset);
    Ok(Json(state.store.get_page(&id, q.offset, limit).await?))
}

#[derive(Deserialize)]
struct InsightsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    /// P-104 Lifeline: when set (RFC 3339 or epoch seconds), only return
    /// insights first-seen or evolved after this timestamp — the
    /// "what's new since you left" filter.
    #[serde(default)]
    since: Option<String>,
    /// P-106 Bilingual: `zh-HK` selects the deterministic zh-HK summary frame;
    /// any other value (or unset) keeps the stored English summary.
    #[serde(default)]
    lang: Option<String>,
}

/// Upper bound on `InsightsQuery.limit` (insights/alerts endpoints). Same
/// unbounded-materialization rationale as `MAX_RECORDS_LIMIT`.
const MAX_INSIGHTS_LIMIT: usize = 500;

async fn list_insights(
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> Result<Json<Vec<hkgov_agent::Insight>>, ApiError> {
    let limit = q.limit.clamp(1, MAX_INSIGHTS_LIMIT);
    let lang = hkgov_agent::Language::parse(q.lang.as_deref());
    // D-007: a present-but-unparseable `since` is a client error, not a silent
    // fallback to the full list. Previously a typo like `?since=banana`
    // returned every insight as if "everything is new since banana" —
    // misleading and a potential surprise leak surface. Now it 400s with a
    // message naming the bad value and the accepted formats.
    let mut insights = if let Some(s) = q.since.as_deref().filter(|s| !s.is_empty()) {
        match parse_since(s) {
            Ok(ts) => state.insights.list_since(limit, ts).await,
            Err(()) => {
                return Err(ApiError(hkgov_common::Error::BadRequest(format!(
                    "invalid `since` value: {s:?} (expected RFC 3339 datetime or epoch seconds)"
                ))));
            }
        }
    } else {
        state.insights.list(limit).await
    };
    // P-106: apply the language selection to each summary in place.
    if lang == hkgov_agent::Language::ZhHk {
        for i in insights.iter_mut() {
            i.summary = hkgov_agent::select_summary(i, lang);
        }
    }
    Ok(Json(insights))
}

/// Parse a `since` query value: RFC 3339 datetime, or epoch seconds.
fn parse_since(s: &str) -> Result<chrono::DateTime<chrono::Utc>, ()> {
    // Try RFC 3339 first.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Fall back to epoch seconds.
    if let Ok(secs) = s.parse::<i64>() {
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
            return Ok(dt);
        }
    }
    Err(())
}

/// P-104 Lifeline: `GET /v1/insights/{id}/history` — the prior versions of one
/// insight, newest-first. Powers the case-file "evolved" diff view.
async fn insight_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Vec<hkgov_agent::InsightRevision>> {
    Json(state.insights.history(&id, 50).await)
}

// ---- GET /brief — the daily brief (product layer) -------------------------

async fn get_brief(
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> Json<hkgov_agent::Brief> {
    let limit = q.limit.clamp(1, MAX_INSIGHTS_LIMIT);
    // Snapshot fast path: the default brief (`limit=50`, no filters) is the
    // shape the materializer precomputes. Custom limits fall through to live.
    if limit == 50 {
        if let Some(v) = crate::daily_view::read_hero(
            &state.daily_view,
            crate::daily_view::HeroField::Brief,
            chrono::Utc::now(),
        )
        .await
        {
            // The value is the serialized `Brief`; re-typed via JSON.
            if let Ok(brief) = serde_json::from_value::<hkgov_agent::Brief>(v) {
                return Json(brief);
            }
        }
    }
    let brief = hkgov_agent::build_brief(&state.insights, limit, chrono::Utc::now()).await;
    Json(brief)
}

// ---- POST + GET /insights/{id}/feedback — the success metric --------------

#[derive(Deserialize)]
struct FeedbackRequest {
    /// `true` = useful, `false` = not useful.
    useful: bool,
    /// Optional reason (esp. for "not useful").
    #[serde(default)]
    note: Option<String>,
}

async fn submit_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<FeedbackRequest>,
) -> Json<serde_json::Value> {
    let fb = hkgov_agent::Feedback {
        insight_id: id,
        useful: req.useful,
        note: req.note,
        submitted_at: chrono::Utc::now(),
    };
    state.feedback.record(fb).await;
    Json(serde_json::json!({ "recorded": true }))
}

async fn get_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let net = state.feedback.net_useful(&id).await;
    Json(serde_json::json!({ "insight_id": id, "net_useful": net }))
}

// ---- GET /insights/{id}/cite — citation-grade export (P-101) ---------------
//
// From any insight, build a citation bundle: a stable permalink, citation
// strings in BibTeX/RIS/APA/Chicago/Markdown, and a reproducibility manifest
// (detector + threshold + a SHA-256 content hash over the evidence). The hash
// is the drift detector: recompute against current data and if it differs, the
// manifest won't match — so a citation never false-claims reproducibility.

#[derive(Deserialize, Default)]
struct CiteQuery {
    /// Optional citation format. When set, the response is a `text/plain`
    /// rendered string (e.g. `?format=bibtex`); otherwise the full bundle JSON.
    #[serde(default)]
    format: Option<String>,
    /// The public base URL for the permalink (e.g. `https://example.com`).
    /// When omitted, the permalink falls back to `http://localhost:8080`.
    ///
    /// D-008 note: this does NOT auto-derive from the request's `Host` header
    /// — behind a reverse proxy the `Host`/`X-Forwarded-Host` semantics are
    /// operator-specific, so we require the caller to pass the intended public
    /// origin explicitly. If you deploy behind a proxy, set this per-request
    /// (or wrap the route) rather than relying on header inference.
    #[serde(default)]
    base_url: Option<String>,
}

async fn cite_insight(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<CiteQuery>,
) -> Result<axum::response::Response, ApiError> {
    // Look up the insight. InsightStore::get is the by-id accessor (P-101 adds it).
    let Some(insight) = state.insights.get(&id).await else {
        return Err(ApiError(hkgov_common::Error::NotFound(id)));
    };
    // Pull the evidence records from the store to compute the content hash.
    // PR-003: hash over the insight's *actual* evidence record_ids, not an
    // arbitrary 500-row page head. The evidence list is self-describing — two
    // reviewers with the same data must get the same `data_sha256` regardless
    // of row ordering. Fall back to a page only when the insight has no
    // evidence refs (legacy/derived signals).
    let dataset_id = DatasetId::new(insight.source, insight.dataset.clone());
    let evidence_ids: Vec<String> = insight
        .evidence
        .iter()
        .map(|e| e.record_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let records = if evidence_ids.is_empty() {
        state.store.get_page(&dataset_id, 0, 500).await?.records
    } else {
        let recs = state.store.get_by_ids(&dataset_id, &evidence_ids).await?;
        // A-003: the manifest's data_sha256 is the drift-detection anchor. If
        // `get_by_ids` returned a *partial* set (some real evidence record_ids
        // missing because the cache is cold, an upstream deletion, or an
        // LRU/TTL eviction), hashing the subset would produce a stable-but-wrong
        // hash — two reviewers would agree on a hash that doesn't reflect the
        // real data, so a genuine revision would never be flagged. Refuse to
        // produce a manifest unless every NON-synthetic evidence id resolved.
        // Synthetic ids (`"series"`, `"threshold"`, `"joined_history"`) carry a
        // detector-derived value in `evidence.value` and are intentionally not
        // rows — they're allowed to be absent from the record set.
        let found: std::collections::HashSet<&str> =
            recs.iter().map(|r| r.record_id.as_str()).collect();
        let real_ids: Vec<&str> = evidence_ids
            .iter()
            .map(String::as_str)
            .filter(|id| !hkgov_agent::cite::is_synthetic_evidence_id(id))
            .collect();
        let missing: Vec<&str> = real_ids
            .iter()
            .filter(|id| !found.contains(*id))
            .copied()
            .collect();
        if !missing.is_empty() {
            tracing::warn!(
                source = %dataset_id.source,
                dataset = %dataset_id.dataset,
                missing = ?missing,
                "cite: partial evidence set returned from store; refusing to hash (A-003)"
            );
            return Err(ApiError(hkgov_common::Error::StoreUnavailable(format!(
                "evidence records for {}/{} are not fully cached ({} of {} non-synthetic ids present); retry shortly",
                dataset_id.source, dataset_id.dataset,
                real_ids.len() - missing.len(),
                real_ids.len()
            ))));
        }
        recs
    };
    let base_url = match q.base_url.as_deref() {
        Some(u) => sanitize_base_url(u)?,
        None => "http://localhost:8080".to_string(),
    };
    let citation = hkgov_agent::build_citation(
        &insight,
        &records,
        &base_url,
        Some(env!("CARGO_PKG_VERSION")),
    );

    // If a format is requested, render and return as text/plain; else JSON.
    use axum::http::header::CONTENT_TYPE;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    if let Some(fmt_str) = q.format {
        let fmt = match fmt_str.to_ascii_lowercase().as_str() {
            "bibtex" => hkgov_agent::CitationFormat::Bibtex,
            "ris" => hkgov_agent::CitationFormat::Ris,
            "apa" => hkgov_agent::CitationFormat::Apa,
            "chicago" => hkgov_agent::CitationFormat::Chicago,
            "markdown" | "md" => hkgov_agent::CitationFormat::Markdown,
            _ => {
                return Err(ApiError(hkgov_common::Error::BadRequest(format!(
                    "unknown citation format: {fmt_str} (try bibtex|ris|apa|chicago|markdown)"
                ))))
            }
        };
        let body = citation.render(fmt);
        Ok((
            StatusCode::OK,
            [(CONTENT_TYPE, "text/plain; charset=utf-8")],
            body,
        )
            .into_response())
    } else {
        Ok(Json(citation).into_response())
    }
}

// ---- GET /alerts — proactive dispatch log ---------------------------------

async fn list_alerts(
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> Json<Vec<hkgov_agent::AlertLogEntry>> {
    let limit = q.limit.clamp(1, MAX_INSIGHTS_LIMIT);
    Json(state.alert_log.recent(limit))
}

// ---- GET /silence-index — government opacity, quantified (P-100) -----------
//
// Productizes the project's thesis: a 0–100 score for "how much did HKGOV not
// explain this period", built purely from existing deterministic findings
// (cross_source_gap + unattributed series_jump + missing-data days). No LLM,
// no API key — the determinism guarantee is the defense against "your opacity
// score is biased": critics can reproduce it from the evidence.
//
// v1 is HKMA-scoped (see silence.rs `COVERED_SOURCE`); widens as data.gov.hk
// coverage expands without a methodology bump.

#[derive(Deserialize, Default)]
struct SilenceIndexQuery {
    /// Period key like "2026-Q2" to scope the score to one quarter. Empty/omitted
    /// scores the FULL held insight corpus (all history), NOT the latest quarter
    /// — pass an explicit `?period=YYYY-Qn` to scope. (PR-008: an earlier version
    /// of this doc claimed empty defaulted to "the latest complete quarter",
    /// which the implementation does not do; the docs now match the behavior.)
    #[serde(default)]
    period: Option<String>,
    /// The institution whose opacity to score, e.g. `hkma`, `immigration`,
    /// `landregistry`, `rvd`. Defaults to `hkma` (backward-compatible with the
    /// v1 single-source index). Each source produces its own honest, scoped
    /// number — they are never blended into a composite.
    #[serde(default)]
    source: Option<String>,
}

async fn silence_index(
    State(state): State<AppState>,
    Query(q): Query<SilenceIndexQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let period = q.period.unwrap_or_default();
    // A-010: validate the period shape. Without this, a non-quarter key like
    // "2026" or "2026-Q9" silently fell through to `starts_with` in the silence
    // indexer and matched nothing (or, for "2026", matched any record_id
    // starting "2026" including synthetic ones). Accept: "" (all-time),
    // "YYYY", "YYYY-MM", "YYYY-Qn". Reject anything else with a 400 so a typo
    // surfaces instead of returning a misleading 0.0 score.
    if !period.is_empty() && !is_valid_silence_period(&period) {
        return Err(ApiError(hkgov_common::Error::BadRequest(format!(
            "invalid `period` ({period:?}): expected one of '', 'YYYY', 'YYYY-MM', or 'YYYY-Qn'"
        ))));
    }
    // Default to HKMA for backward compat with v1 callers that omit `source`.
    let source = match &q.source {
        Some(s) if !s.is_empty() => parse_source(s)?,
        _ => DataSource::Hkma,
    };
    // Snapshot fast path: if the daily-view materializer already rolled up
    // this (source, period) bucket, serve the precomputed value without
    // walking the InsightStore. Falls back to live compute when the snapshot
    // is missing or stale.
    if let Some(v) =
        crate::daily_view::read_silence(&state.daily_view, source, &period, chrono::Utc::now())
            .await
    {
        return Ok(Json(v));
    }
    let idx =
        hkgov_agent::build_silence_index(&state.insights, source, &period, chrono::Utc::now())
            .await;
    Ok(Json(
        serde_json::to_value(&idx).unwrap_or(serde_json::Value::Null),
    ))
}

/// A-010: shape-check a silence-index `period` query value. Accepts `YYYY`,
/// `YYYY-MM`, `YYYY-Qn` (n ∈ 1..=4). Empty is the all-time view (handled by
/// the caller). Rejects typos like `2026-Q9`, `2026-13`, `2026Q2`, `banana`.
fn is_valid_silence_period(period: &str) -> bool {
    // YYYY (4 digits, plausible year 1900..=9999).
    if period.len() == 4 {
        return period.chars().all(|c| c.is_ascii_digit());
    }
    // YYYY-MM (month 01..=12).
    if let Some((y, m)) = period.split_once('-') {
        if y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()) {
            // YYYY-Qn
            if let Some(qstr) = m.strip_prefix('Q') {
                if let Ok(q) = qstr.parse::<u8>() {
                    return (1..=4).contains(&q);
                }
                return false;
            }
            // YYYY-MM
            if m.len() == 2 && m.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(month) = m.parse::<u8>() {
                    return (1..=12).contains(&month);
                }
            }
        }
    }
    false
}

// ---- M6: Transparency Foundation (composite index + report) -----------------

/// `GET /v1/transparency-index?sources=&period=` — the multi-source composite.
/// Each named source is scored independently (via the generalized signal
/// registry), then the composite is the events-weighted average. `?sources=`
/// is a comma-separated list (e.g. `hkma,rvd,landregistry`); omitting it
/// scores all sources the system has insights for.
#[derive(Deserialize, Default)]
struct TransparencyIndexQuery {
    #[serde(default)]
    sources: Option<String>,
    #[serde(default)]
    period: Option<String>,
}

async fn transparency_index(
    State(state): State<AppState>,
    Query(q): Query<TransparencyIndexQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let period = q.period.unwrap_or_default();
    if !period.is_empty() && !is_valid_silence_period(&period) {
        return Err(ApiError(hkgov_common::Error::BadRequest(format!(
            "invalid `period` ({period:?}): expected one of '', 'YYYY', 'YYYY-MM', or 'YYYY-Qn'"
        ))));
    }
    // Parse the comma-separated source list; fall back to all sources the
    // registry knows if omitted.
    let sources: Vec<DataSource> = match q.sources.as_deref() {
        Some(s) if !s.trim().is_empty() => {
            let mut out = Vec::new();
            for part in s.split(',') {
                let part = part.trim();
                if !part.is_empty() {
                    out.push(parse_source(part)?);
                }
            }
            out
        }
        _ => state.registry.sources(),
    };
    // Snapshot fast path: serve the precomputed composite when the caller used
    // the default (no source override, no period override — the dashboard's
    // typical call). A custom source/period query falls through to live
    // compute so ad-hoc analyst queries still work.
    let snapshot_applicable = q.sources.is_none() && period.is_empty();
    if snapshot_applicable {
        if let Some(v) = crate::daily_view::read_hero(
            &state.daily_view,
            crate::daily_view::HeroField::TransparencyIndex,
            chrono::Utc::now(),
        )
        .await
        {
            return Ok(Json(v));
        }
    }
    let idx =
        hkgov_agent::build_composite_index(&state.insights, &sources, &period, chrono::Utc::now())
            .await;
    Ok(Json(
        serde_json::to_value(&idx).unwrap_or(serde_json::Value::Null),
    ))
}

/// `GET /v1/transparency-index/report?source=&period=&format=` — the quarterly
/// transparency report. `format=markdown` (default) returns text/markdown;
/// `format=json` returns the structured report; `format=pdf-data` returns the
/// JSON payload a PDF renderer consumes.
#[derive(Deserialize, Default)]
struct TransparencyReportQuery {
    /// The source to report on (default hkma — backward compat).
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    period: Option<String>,
    /// markdown | json | pdf-data. Defaults to markdown.
    #[serde(default)]
    format: Option<String>,
    /// Public origin for cite permalinks (default http://localhost:8080).
    #[serde(default)]
    base_url: Option<String>,
    /// Institution name for the report header.
    #[serde(default)]
    publisher: Option<String>,
    /// Max contributing insights to list (default 10, clamped 1..=50).
    #[serde(default = "default_report_top_n")]
    top_n: usize,
}

fn default_report_top_n() -> usize {
    10
}

async fn transparency_report_route(
    State(state): State<AppState>,
    Query(q): Query<TransparencyReportQuery>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let period = q.period.unwrap_or_default();
    if !period.is_empty() && !is_valid_silence_period(&period) {
        return Err(ApiError(hkgov_common::Error::BadRequest(format!(
            "invalid `period` ({period:?}): expected one of '', 'YYYY', 'YYYY-MM', or 'YYYY-Qn'"
        ))));
    }
    let source = match q.source.as_deref() {
        Some(s) if !s.trim().is_empty() => parse_source(s)?,
        _ => DataSource::Hkma,
    };
    let base_url = match q
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(u) => sanitize_base_url(u)?,
        None => "http://localhost:8080".to_string(),
    };
    let publisher = q
        .publisher
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| hkgov_agent::DEFAULT_PUBLISHER.to_string());
    let top_n = q.top_n.clamp(1, 50);
    // Snapshot fast path: serve the precomputed report when the caller used
    // the default HKMA + current-quarter shape. Honors markdown, json, and
    // pdf-data formats (the snapshot stores both the Markdown render and the
    // structured JSON). Custom source/period/top_n queries fall through to
    // live compute.
    let snapshot_applicable = matches!(q.source.as_deref(), None | Some("") | Some("hkma"))
        && period.is_empty()
        && matches!(
            q.format.as_deref(),
            None | Some("") | Some("markdown") | Some("md") | Some("json") | Some("pdf-data")
        );
    if snapshot_applicable {
        use axum::response::IntoResponse;
        match q.format.as_deref() {
            None | Some("") | Some("markdown") | Some("md") => {
                if let Some(md) =
                    crate::daily_view::read_report_markdown(&state.daily_view, chrono::Utc::now())
                        .await
                {
                    return Ok(
                        ([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md)
                            .into_response(),
                    );
                }
            }
            Some("json") | Some("pdf-data") => {
                if let Some(v) = crate::daily_view::read_hero(
                    &state.daily_view,
                    crate::daily_view::HeroField::TransparencyReportJson,
                    chrono::Utc::now(),
                )
                .await
                {
                    return Ok(
                        ([(header::CONTENT_TYPE, "application/json")], Json(v)).into_response()
                    );
                }
            }
            _ => {}
        }
    }
    let opts = hkgov_agent::ReportOptions::new(source, period)
        .base_url(base_url)
        .publisher(publisher)
        .top_n(top_n);
    let report = hkgov_agent::build_report(
        &state.insights,
        &state.provenance,
        &opts,
        chrono::Utc::now(),
    )
    .await;
    let format = q.format.as_deref().unwrap_or("markdown");
    match format {
        "json" | "pdf-data" => {
            Ok(([(header::CONTENT_TYPE, "application/json")], Json(report)).into_response())
        }
        "markdown" | "md" => {
            let md = hkgov_agent::render_markdown(&report);
            Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md).into_response())
        }
        other => Err(ApiError(hkgov_common::Error::BadRequest(format!(
            "unknown format {other:?}: expected markdown, json, or pdf-data"
        )))),
    }
}

// ---- GET /unprecedentedness — how rare is this value? (P-103) --------------
//
// Scores a numeric value against its own stored history: percentile rank, a
// median ± k·MAD "normal range" band, a 1-in-N return period, and the most
// recent prior exceedance ("last time this happened"). Pure Rust over the
// warmed cache; composes from the same MAD math the `outlier` detector uses.

#[derive(Deserialize)]
struct UnprecedentednessQuery {
    /// The dataset to read history from, e.g. `hkma/daily-interbank-liquidity`.
    source: String,
    dataset: String,
    /// The numeric field whose history defines "normal".
    field: String,
    /// The value to score (the current observation).
    value: f64,
    /// Optional band multiplier (defaults to 3.5, matching the outlier z).
    #[serde(default)]
    k: Option<f64>,
}

async fn unprecedentedness(
    State(state): State<AppState>,
    Query(q): Query<UnprecedentednessQuery>,
) -> Result<Json<hkgov_agent::Unprecedentedness>, ApiError> {
    let source = parse_source(&q.source)?;
    let id = DatasetId::new(source, q.dataset.clone());
    // Pull the full history (cap at the page size the store supports; the
    // 90-day default window is well inside it).
    let page = state.store.get_page(&id, 0, 500).await?;
    // D-031: if the records cache is cold (refresh in flight / LRU eviction),
    // `get_page` now returns an empty page rather than 502. But scoring a value
    // against zero history would silently produce a misleading "no historical
    // data" result. Detect that case via the registry's persisted count and
    // surface a retryable 503 instead, so the comparator UI can tell the user
    // "data temporarily unavailable, retry" rather than showing a false band.
    if page.records.is_empty() {
        if let Some(meta) = state.store.meta(&id).await? {
            if meta.record_count > 0 {
                return Err(ApiError(hkgov_common::Error::StoreUnavailable(format!(
                    "records for {}/{} are not cached (refresh in progress or cache cold); retry shortly",
                    id.source, id.dataset
                ))));
            }
        }
    }
    let k = q.k.unwrap_or(hkgov_agent::DEFAULT_BAND_K);
    // SEC-API-06: validate the user-supplied f64 params. `?value=NaN` or
    // `?k=Infinity` is accepted by serde_urlencoded/`f64::parse` and would
    // flow unchecked into the scoring math, risking misleading or
    // panic-prone downstream computation. Reject non-finite values explicitly.
    if !q.value.is_finite() {
        return Err(ApiError(hkgov_common::Error::BadRequest(
            "value must be a finite number (NaN/Infinity rejected)".into(),
        )));
    }
    if let Some(kv) = q.k {
        if !kv.is_finite() || kv <= 0.0 {
            return Err(ApiError(hkgov_common::Error::BadRequest(
                "k must be a finite, positive number".into(),
            )));
        }
    }
    // History = all field values in chronological order.
    let history: Vec<f64> = page
        .records
        .iter()
        .filter_map(|r| match r.fields.get(&q.field)? {
            hkgov_common::RecordValue::Float(f) => Some(*f),
            hkgov_common::RecordValue::Int(i) => Some(*i as f64),
            _ => None,
        })
        .collect();
    let records = page.records;
    let read = hkgov_agent::score_unprecedentedness(q.value, &history, &records, &q.field, k);
    Ok(Json(read))
}

// ---- POST /ask — natural-language Q&A -------------------------------------

#[derive(Deserialize)]
struct AskRequest {
    question: String,
}

/// Answer a natural-language question about the data.
async fn ask(
    State(state): State<AppState>,
    Json(req): Json<AskRequest>,
) -> Result<Json<Answer>, ApiError> {
    let belt = ToolBelt::for_store(state.store.clone());

    // Heuristic client → can't reason; fall straight to the keyword matcher.
    if state.llm.name() == HeuristicClient::new().name() {
        let answer = heuristic_answer(&req.question, &belt)
            .await
            .map_err(ApiError)?;
        return Ok(Json(answer));
    }

    // Rich mode: let the LLM reason over the tool belt.
    let system = "You are a financial-data analyst for Hong Kong government \
        open data. Answer the user's question by calling the provided tools \
        (list_datasets, query_dataset, run_detector) to gather evidence, then \
        give a concise answer grounded in what the tools returned.";
    let outcome = run_agent_loop(state.llm.as_ref(), &belt, system, &req.question, 6)
        .await
        .map_err(|e| ApiError(hkgov_common::Error::Agent(e.to_string())))?;
    match outcome {
        hkgov_agent::AgentOutcome::Answer(a) => Ok(Json(a)),
        // If the loop surfaced findings instead of an answer, frame them.
        hkgov_agent::AgentOutcome::Findings(_) => Ok(Json(Answer {
            text: "The agent surfaced findings but no direct answer. See /v1/insights.".into(),
            confidence: 0.4,
            trace: vec![],
        })),
    }
}

fn parse_source(s: &str) -> Result<DataSource, ApiError> {
    DataSource::parse(s).ok_or_else(|| ApiError(hkgov_common::Error::UnknownSource(s.to_string())))
}

/// SEC-API-07: validate an operator-supplied `base_url` before reflecting it
/// into citation permalinks or report headers. A malicious `?base_url=` value
/// with embedded CRLF, HTML, or a `javascript:` scheme could be reflected into
/// the rendered Markdown/BibTeX and the report header, enabling content
/// injection into downstream PDF/Markdown consumers. Require an absolute
/// http(s) URL and reject any control characters / angle brackets.
fn sanitize_base_url(raw: &str) -> Result<String, ApiError> {
    // Reject embedded CRLF / control chars / HTML metacharacters outright —
    // these have no place in a URL and enable header/markdown injection.
    if raw
        .chars()
        .any(|c| c.is_control() || c == '<' || c == '>' || c == '"' || c == '\'')
    {
        return Err(ApiError(hkgov_common::Error::BadRequest(
            "base_url contains illegal characters".into(),
        )));
    }
    // Require an http(s) scheme prefix. We avoid pulling the `url` crate into
    // this binary just for this check; a scheme-prefix test is sufficient for
    // the content-injection threat model (javascript:/data: are the dangerous
    // schemes, and they're rejected here).
    let lower = raw.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(ApiError(hkgov_common::Error::BadRequest(
            "base_url must be an absolute http(s) URL".into(),
        )));
    }
    Ok(raw.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::auth_routes::RequestTokenRequest;
    use super::investigations::ListInvestigationsQuery;
    use super::signals::ListSignalsQuery;
    use super::*;
    use hkgov_common::Settings;
    use hkgov_store::RecordStore;
    use serde_json::json;
    use std::sync::Arc;

    /// Build an AppState backed by a tiny in-process store, no network. We
    /// still construct the real Registry (it only builds reqwest clients at
    /// construction; no calls happen until fetch).
    async fn test_state() -> AppState {
        let settings = Settings::default();
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let store = Arc::new(hkgov_store::MemoryStore::new(10, 60));
        // Seed one dataset so the heuristic matcher has something to find.
        let id = DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        store
            .register(
                id.clone(),
                "Daily Interbank Liquidity".into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec!["hibor".into()],
                hkgov_common::Cadence::Daily,
            )
            .await;
        let rec = hkgov_common::NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            record_id: "2026-01".into(),
            fields: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "hibor_overnight".into(),
                    hkgov_common::RecordValue::Float(2.0),
                );
                m
            },
            fetched_at: chrono::Utc::now(),
        };
        store.put_dataset(&id, vec![rec]).await.unwrap();

        AppState {
            registry,
            store,
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            daily_view: crate::daily_view::empty_slot(),
            settings: Arc::new(settings),
        }
    }

    #[tokio::test]
    async fn ask_heuristic_answers_on_keyword_match() {
        let state = test_state().await;
        let req = AskRequest {
            question: "what is the interbank liquidity?".into(),
        };
        let resp = ask(State(state), Json(req)).await.unwrap();
        assert!(resp.0.text.contains("Daily Interbank Liquidity"));
        assert!(resp.0.confidence > 0.3);
    }

    #[tokio::test]
    async fn ask_heuristic_falls_back_to_inventory() {
        let state = test_state().await;
        let req = AskRequest {
            question: "tell me about marigolds".into(),
        };
        let resp = ask(State(state), Json(req)).await.unwrap();
        // No keyword match → inventory fallback mentions the dataset name.
        assert!(resp.0.text.contains("daily-interbank-liquidity"));
        assert!(resp.0.confidence <= 0.4);
    }

    /// The root endpoint directory lists /ask (regression guard).
    #[tokio::test]
    async fn root_lists_ask_endpoint() {
        let state = test_state().await;
        let resp = root(State(state)).await;
        let body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&resp.0).unwrap()).unwrap();
        let endpoints = body["endpoints"].as_array().unwrap();
        let has_ask = endpoints
            .iter()
            .any(|e| e.as_str().unwrap_or("").contains("/ask"));
        assert!(has_ask, "root should advertise POST /v1/ask");
        // Touch `json!` so the import isn't flagged unused.
        let _ = json!({"x": 1});
    }

    // ---- D-007: bad `?since=` must 400, not silently fall back -----------
    //
    // Before D-007, an unparseable `since` (e.g. `?since=banana`) silently
    // returned the FULL unfiltered insight list — misleading and a surprise
    // leak surface. The handler now returns 400 BadRequest naming the bad value.

    #[tokio::test]
    async fn d007_bad_since_returns_400() {
        let state = test_state().await;
        let q = InsightsQuery {
            limit: 10,
            since: Some("banana".into()),
            lang: None,
        };
        let result = list_insights(State(state), Query(q)).await;
        assert!(result.is_err(), "bad since must error, not fall back");
        let err = result.unwrap_err();
        assert_eq!(err.0.status_code(), 400, "bad since → 400");
    }

    #[tokio::test]
    async fn d007_valid_rfc3339_since_still_works() {
        let state = test_state().await;
        let q = InsightsQuery {
            limit: 10,
            since: Some("2026-01-01T00:00:00Z".into()),
            lang: None,
        };
        // Must NOT error — valid RFC3339 is accepted.
        let result = list_insights(State(state), Query(q)).await;
        assert!(result.is_ok(), "valid RFC3339 since must not 400");
    }

    #[tokio::test]
    async fn d007_epoch_seconds_since_still_works() {
        let state = test_state().await;
        let q = InsightsQuery {
            limit: 10,
            since: Some("1717200000".into()),
            lang: None,
        };
        let result = list_insights(State(state), Query(q)).await;
        assert!(result.is_ok(), "epoch-seconds since must not 400");
    }

    #[tokio::test]
    async fn d007_no_since_still_works() {
        let state = test_state().await;
        let q = InsightsQuery {
            limit: 10,
            since: None,
            lang: None,
        };
        let result = list_insights(State(state), Query(q)).await;
        assert!(result.is_ok(), "no since must not 400");
    }

    // ---- /sources filtering + /categories ---------------------------------

    /// A richer state with several categorized datasets for filter tests.
    async fn multi_state() -> AppState {
        let settings = hkgov_common::Settings::default();
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let store = Arc::new(hkgov_store::MemoryStore::new(20, 60));

        // Helper to seed one categorized dataset.
        async fn seed(
            store: &Arc<hkgov_store::MemoryStore>,
            source: DataSource,
            ds: &str,
            title: &str,
            cat: hkgov_common::Category,
            tags: Vec<String>,
            cad: hkgov_common::Cadence,
        ) {
            let id = DatasetId::new(source, ds);
            store
                .register(id.clone(), title.into(), None, 3600, cat, tags, cad)
                .await;
            store
                .put_dataset(
                    &id,
                    vec![hkgov_common::NormalizedRecord {
                        source,
                        dataset: ds.into(),
                        record_id: "2026-01".into(),
                        fields: std::collections::BTreeMap::new(),
                        fetched_at: chrono::Utc::now(),
                    }],
                )
                .await
                .unwrap();
        }

        seed(
            &store,
            DataSource::Hkma,
            "daily-interbank-liquidity",
            "Daily Interbank Liquidity",
            hkgov_common::Category::Monetary,
            vec!["hibor".into(), "liquidity".into()],
            hkgov_common::Cadence::Daily,
        )
        .await;
        seed(
            &store,
            DataSource::Hkma,
            "capital-market-statistics",
            "Capital Market Statistics",
            hkgov_common::Category::Monetary,
            vec!["hang-seng-index".into()],
            hkgov_common::Cadence::Monthly,
        )
        .await;
        seed(
            &store,
            DataSource::DataGovHk,
            "money-lenders-licensees",
            "Money Lenders Licensees",
            hkgov_common::Category::Fiscal,
            vec!["licensing".into()],
            hkgov_common::Cadence::Daily,
        )
        .await;

        AppState {
            registry,
            store,
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            daily_view: crate::daily_view::empty_slot(),
            settings: Arc::new(settings),
        }
    }

    #[tokio::test]
    async fn sources_returns_all_when_no_filter() {
        let state = multi_state().await;
        let resp = list_sources(
            State(state),
            Query(DatasetFilter::default()),
            axum::extract::RawQuery(None),
        )
        .await
        .unwrap();
        assert_eq!(resp.0.len(), 3);
    }

    #[tokio::test]
    async fn sources_filters_by_category() {
        let state = multi_state().await;
        let f = DatasetFilter {
            category: Some("monetary".into()),
            ..Default::default()
        };
        let resp = list_sources(State(state), Query(f), axum::extract::RawQuery(None))
            .await
            .unwrap();
        assert_eq!(resp.0.len(), 2);
        assert!(resp
            .0
            .iter()
            .all(|m| m.category == hkgov_common::Category::Monetary));
    }

    #[tokio::test]
    async fn sources_filters_by_tag() {
        let state = multi_state().await;
        // Single ?tag=hibor — the form that 400'd before the D-001 fix.
        let resp = list_sources(
            State(state),
            Query(DatasetFilter::default()),
            axum::extract::RawQuery(Some("tag=hibor".into())),
        )
        .await
        .unwrap();
        assert_eq!(resp.0.len(), 1);
        assert_eq!(resp.0[0].dataset, "daily-interbank-liquidity");
    }

    #[tokio::test]
    async fn sources_tag_matches_any_repeated() {
        let state = multi_state().await;
        // Repeated ?tag=hibor&tag=licensing → ANY match → 2 datasets.
        let resp = list_sources(
            State(state),
            Query(DatasetFilter::default()),
            axum::extract::RawQuery(Some("tag=hibor&tag=licensing".into())),
        )
        .await
        .unwrap();
        assert_eq!(resp.0.len(), 2);
    }

    #[tokio::test]
    async fn sources_tag_matches_any_comma() {
        let state = multi_state().await;
        // Comma-separated ?tag=hibor,licensing → ANY match → 2 datasets.
        let resp = list_sources(
            State(state),
            Query(DatasetFilter::default()),
            axum::extract::RawQuery(Some("tag=hibor,licensing".into())),
        )
        .await
        .unwrap();
        assert_eq!(resp.0.len(), 2);
    }

    #[tokio::test]
    async fn sources_filters_by_cadence() {
        let state = multi_state().await;
        let f = DatasetFilter {
            cadence: Some("monthly".into()),
            ..Default::default()
        };
        let resp = list_sources(State(state), Query(f), axum::extract::RawQuery(None))
            .await
            .unwrap();
        assert_eq!(resp.0.len(), 1);
        assert_eq!(resp.0[0].dataset, "capital-market-statistics");
    }

    #[tokio::test]
    async fn sources_free_text_search() {
        let state = multi_state().await;
        let f = DatasetFilter {
            q: Some("interbank".into()),
            ..Default::default()
        };
        let resp = list_sources(State(state), Query(f), axum::extract::RawQuery(None))
            .await
            .unwrap();
        assert_eq!(resp.0.len(), 1);
        assert_eq!(resp.0[0].dataset, "daily-interbank-liquidity");
    }

    #[tokio::test]
    async fn sources_composes_filters() {
        let state = multi_state().await;
        // monetary AND daily → 1 (the interbank one; capital-market is monthly).
        let f = DatasetFilter {
            category: Some("monetary".into()),
            cadence: Some("daily".into()),
            ..Default::default()
        };
        let resp = list_sources(State(state), Query(f), axum::extract::RawQuery(None))
            .await
            .unwrap();
        assert_eq!(resp.0.len(), 1);
        assert_eq!(resp.0[0].dataset, "daily-interbank-liquidity");
    }

    #[tokio::test]
    async fn sources_invalid_category_returns_empty() {
        let state = multi_state().await;
        let f = DatasetFilter {
            category: Some("nonsense".into()),
            ..Default::default()
        };
        let resp = list_sources(State(state), Query(f), axum::extract::RawQuery(None))
            .await
            .unwrap();
        assert!(resp.0.is_empty());
    }

    #[tokio::test]
    async fn categories_groups_with_counts() {
        let state = multi_state().await;
        let resp = list_categories(State(state)).await.unwrap();
        // Two categories present.
        assert_eq!(resp.0.len(), 2);
        let monetary = resp
            .0
            .iter()
            .find(|g| g.category == "monetary")
            .expect("monetary group");
        assert_eq!(monetary.count, 2);
        let fiscal = resp
            .0
            .iter()
            .find(|g| g.category == "fiscal")
            .expect("fiscal group");
        assert_eq!(fiscal.count, 1);
    }

    // ---- empty-prefix routing (D-003 regression guard) ---------------------
    //
    // When `api.api_prefix` is empty the versioned API routes must merge to the
    // root (no `/v1` segment) WITHOUT panicking on the duplicate `/health`. This
    // integration test drives the full `router()` through axum's `ServiceExt`
    // so it exercises the real route table — not just the handler fns — and
    // locks down every reachable path. A regression here means the merge branch
    // silently dropped routes (the failure mode the original D-003 fix risked).
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Send a GET through a built router and return the status code.
    async fn get_status(router: axum::Router, path: &str) -> u16 {
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        // Drain the body so the connection is fully consumed.
        let _ = resp.into_body().collect().await;
        status
    }

    /// Read the full body of a handler-produced `Response` into a UTF-8 string.
    /// Used by the cite route tests, which need to inspect the rendered JSON /
    /// text body (the handler returns `Response`, not `Json`).
    async fn body_string(resp: axum::response::Response) -> String {
        use http_body_util::BodyExt;
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("body is utf-8")
    }

    async fn state_for_routing() -> AppState {
        // Reuse the multi-dataset state so /sources has something to return.
        multi_state().await
    }

    /// Rebuild an AppState with a different `api_prefix`. The settings live
    /// behind an `Arc`, so we replace the whole field rather than mutate.
    fn with_prefix(mut state: AppState, prefix: &str) -> AppState {
        let mut settings = (*state.settings).clone();
        settings.api.api_prefix = prefix.into();
        state.settings = Arc::new(settings);
        state
    }

    #[tokio::test]
    async fn empty_prefix_mounts_all_routes_at_root() {
        let state = with_prefix(state_for_routing().await, "");
        let app = router(state);

        // Every API route must resolve at root (no /v1), and the static
        // dashboard + root directory must still be reachable.
        for path in [
            "/",
            "/dashboard",
            "/health",
            "/health/sources",
            "/ready",
            "/sources",
            "/categories",
            "/insights",
            "/brief",
            "/alerts",
            "/datasets/hkma/daily-interbank-liquidity",
            "/datasets/hkma/daily-interbank-liquidity/records",
        ] {
            assert_eq!(
                get_status(app.clone(), path).await,
                200,
                "empty-prefix: {path} should be 200 at root"
            );
        }
        // And the prefixed path must NOT exist (prefix is empty).
        assert_eq!(
            get_status(app.clone(), "/v1/sources").await,
            404,
            "empty-prefix: /v1/sources must be 404 (no prefix)"
        );
    }

    #[tokio::test]
    async fn default_prefix_nests_routes_under_v1() {
        // Symmetric guard: the default `/v1` prefix must keep routes under /v1.
        let state = with_prefix(state_for_routing().await, "/v1");
        let app = router(state);

        assert_eq!(get_status(app.clone(), "/v1/sources").await, 200);
        assert_eq!(get_status(app.clone(), "/v1/insights").await, 200);
        assert_eq!(get_status(app.clone(), "/health").await, 200);
        // PR-004: /ready is a root-level probe (like /health), reachable with
        // warmed data + closed breakers ⇒ 200.
        assert_eq!(get_status(app.clone(), "/ready").await, 200);
        assert_eq!(get_status(app.clone(), "/dashboard").await, 200);
        // Without the prefix, the API routes are NOT at root.
        assert_eq!(get_status(app.clone(), "/sources").await, 404);
    }

    // ---- /silence-index (P-100) -------------------------------------------
    //
    // The flagship: a deterministic 0–100 opacity score built from existing
    // findings. These tests guard the HTTP surface; the scoring math is unit-
    // tested in crates/agent/src/silence.rs.

    /// Build a state seeded with cross-source-gap + series-jump insights so the
    /// silence index has something to score.
    async fn silence_state() -> AppState {
        let settings = Settings::default();
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let insights = Arc::new(hkgov_agent::InsightStore::new());

        // A press-only cross_source_gap in 2026-Q2 (press release, no data row).
        let gap = hkgov_agent::Insight {
            id: "cross_source_gap:hkma:x:1".into(),
            kind: "cross_source_gap".into(),
            severity: hkgov_agent::InsightSeverity::Info,
            title: "gap".into(),
            summary: "press release with no data row".into(),
            source: DataSource::Hkma,
            dataset: "x".into(),
            evidence: vec![hkgov_agent::insight::EvidenceRef {
                record_id: "2026-05-10".into(),
                field: "date".into(),
                value: json!("2026-05-10"),
                context: Some("press release date without matching data".into()),
            }],
            confidence: 0.6,
            generated_at: chrono::Utc::now(),
            producer: "test".into(),
            experimental: false,
            first_seen: None,
            version: 1,
            evolution: None,
        };
        insights.upsert(gap).await;

        AppState {
            registry,
            store: Arc::new(hkgov_store::MemoryStore::new(10, 60)),
            insights,
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            daily_view: crate::daily_view::empty_slot(),
            settings: Arc::new(settings),
        }
    }

    #[tokio::test]
    async fn silence_index_returns_versioned_hkma_scoped_score() {
        let state = silence_state().await;
        let q = SilenceIndexQuery {
            period: Some("2026-Q2".into()),
            ..Default::default()
        };
        // The route now returns `Json<Value>` (snapshot fast path: serve the
        // precomputed value without re-deserializing through SilenceIndex's
        // `&'static str` field). Assert on the raw JSON payload — the wire
        // shape is unchanged.
        let idx = silence_index(State(state), Query(q)).await.unwrap().0;
        // CLAIM H: version is now "1.0.<fingerprint>" — assert the human prefix
        // rather than the exact string, since the fingerprint depends on the
        // current weights (and changes when they change, by design).
        let mv = idx["methodology_version"].as_str().unwrap_or("");
        assert!(
            mv.starts_with("1.0."),
            "methodology_version should start with '1.0.', got {mv}"
        );
        assert!(
            idx["label"].as_str().unwrap_or("").contains("HKMA"),
            "label: {}",
            idx["label"]
        );
        assert_eq!(idx["source"], "hkma");
        assert_eq!(idx["period"], "2026-Q2");
        // One press-only gap → positive score.
        let score = idx["score"].as_f64().unwrap_or(-1.0);
        assert!(score > 0.0, "score should be > 0, got {score}");
        assert!(idx["total_events"].as_u64().unwrap_or(0) > 0);
        // Determinism: signals are populated + auditable.
        assert!(!idx["signals"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn silence_index_empty_when_no_insights() {
        // A state with no insights → zero score, zero events.
        let state = silence_state().await;
        let empty_state = AppState {
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            ..state
        };
        let q = SilenceIndexQuery {
            period: Some("2026-Q2".into()),
            ..Default::default()
        };
        let idx = silence_index(State(empty_state), Query(q)).await.unwrap().0;
        assert_eq!(idx["score"].as_f64().unwrap_or(-1.0), 0.0);
        assert_eq!(idx["total_events"].as_u64().unwrap_or(1), 0);
    }

    #[tokio::test]
    async fn silence_index_unknown_source_is_rejected() {
        // A bad `source` query value must surface as an error (not silently
        // default to HKMA — the caller asked for a specific institution).
        let state = silence_state().await;
        let q = SilenceIndexQuery {
            source: Some("nonsense".into()),
            ..Default::default()
        };
        let res = silence_index(State(state), Query(q)).await;
        assert!(res.is_err(), "unknown source must error");
    }

    // ---- A-010: malformed `period` must 400, not silently match nothing ----
    #[test]
    fn is_valid_silence_period_accepts_documented_shapes() {
        // All-time (handled by caller as empty, but the helper should still
        // gracefully reject it — the caller special-cases "" before calling).
        assert!(!is_valid_silence_period("")); // empty is NOT a valid period shape
                                               // Documented shapes.
        assert!(is_valid_silence_period("2026"));
        assert!(is_valid_silence_period("2026-05"));
        assert!(is_valid_silence_period("2026-Q2"));
        // Boundary quarters + months.
        assert!(is_valid_silence_period("2026-Q1"));
        assert!(is_valid_silence_period("2026-Q4"));
        assert!(is_valid_silence_period("2026-01"));
        assert!(is_valid_silence_period("2026-12"));
    }

    #[test]
    fn is_valid_silence_period_rejects_typos_and_garbage() {
        // The A-010 repros: these previously fell through to `starts_with` and
        // matched nothing (or matched synthetic record_ids for the bare-year
        // case), producing a misleading 0.0 score.
        assert!(!is_valid_silence_period("2026-Q9"), "Q9 out of range");
        assert!(!is_valid_silence_period("2026-Q0"), "Q0 out of range");
        assert!(!is_valid_silence_period("2026-13"), "month 13 out of range");
        assert!(!is_valid_silence_period("2026-00"), "month 0 out of range");
        assert!(!is_valid_silence_period("2026Q2"), "missing dash");
        assert!(!is_valid_silence_period("2026-5"), "single-digit month");
        assert!(!is_valid_silence_period("banana"), "garbage");
        assert!(!is_valid_silence_period("20260"), "5-digit year");
    }

    #[tokio::test]
    async fn silence_index_bad_period_returns_400() {
        let state = silence_state().await;
        let q = SilenceIndexQuery {
            period: Some("2026-Q9".into()),
            ..Default::default()
        };
        let err = silence_index(State(state), Query(q))
            .await
            .expect_err("malformed period must 400 (A-010)");
        assert_eq!(err.0.status_code(), 400);
    }

    // ---- /unprecedentedness (P-103) ---------------------------------------

    /// Build a state seeded with a numeric series long enough to define a band
    /// (≥ MIN_HISTORY_POINTS = 12) and a spike at the end.
    async fn unprecedentedness_state() -> AppState {
        let settings = Settings::default();
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let store = Arc::new(hkgov_store::MemoryStore::new(20, 60));
        let id = DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        store
            .register(
                id.clone(),
                "Daily Interbank Liquidity".into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec!["hibor".into()],
                hkgov_common::Cadence::Daily,
            )
            .await;
        // 12 in-band values (~10) + a spike of 100 at the end.
        let mut recs: Vec<hkgov_common::NormalizedRecord> = Vec::new();
        let vals = [
            9.5_f64, 10.0, 9.8, 10.2, 10.1, 9.9, 10.3, 9.7, 10.1, 9.9, 10.2, 9.8, 100.0,
        ];
        for (i, v) in vals.iter().enumerate() {
            recs.push(hkgov_common::NormalizedRecord {
                source: DataSource::Hkma,
                dataset: "daily-interbank-liquidity".into(),
                record_id: format!("2026-{i:02}"),
                fields: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert(
                        "hibor_overnight".into(),
                        hkgov_common::RecordValue::Float(*v),
                    );
                    m
                },
                fetched_at: chrono::Utc::now(),
            });
        }
        store.put_dataset(&id, recs).await.unwrap();

        AppState {
            registry,
            store,
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            daily_view: crate::daily_view::empty_slot(),
            settings: Arc::new(settings),
        }
    }

    #[tokio::test]
    async fn unprecedentedness_marks_spike_unprecedented() {
        let state = unprecedentedness_state().await;
        let q = UnprecedentednessQuery {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            field: "hibor_overnight".into(),
            value: 100.0,
            k: None,
        };
        let u = unprecedentedness(State(state), Query(q)).await.unwrap().0;
        assert!(u.is_unprecedented(), "100.0 should be unprecedented: {u:?}");
        assert!(u.band.is_some());
        assert!(u.percentile.unwrap() > 90.0);
        assert!(u.one_in_n.unwrap() >= 1);
    }

    #[tokio::test]
    async fn unprecedentedness_in_band_value_not_unprecedented() {
        let state = unprecedentedness_state().await;
        let q = UnprecedentednessQuery {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            field: "hibor_overnight".into(),
            value: 10.0,
            k: None,
        };
        let u = unprecedentedness(State(state), Query(q)).await.unwrap().0;
        assert!(!u.is_unprecedented(), "10.0 should be in-band: {u:?}");
    }

    #[tokio::test]
    async fn unprecedentedness_unknown_source_errors() {
        let state = unprecedentedness_state().await;
        let q = UnprecedentednessQuery {
            source: "not-a-source".into(),
            dataset: "x".into(),
            field: "f".into(),
            value: 1.0,
            k: None,
        };
        assert!(unprecedentedness(State(state), Query(q)).await.is_err());
    }

    #[tokio::test]
    async fn unprecedentedness_is_deterministic() {
        let state = unprecedentedness_state().await;
        let mk = || {
            Query(UnprecedentednessQuery {
                source: "hkma".into(),
                dataset: "daily-interbank-liquidity".into(),
                field: "hibor_overnight".into(),
                value: 10.5,
                k: None,
            })
        };
        let a = unprecedentedness(State(state.clone()), mk())
            .await
            .unwrap()
            .0;
        let b = unprecedentedness(State(state), mk()).await.unwrap().0;
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
        );
    }

    // D-031: when the records cache is cold (refresh in flight / LRU/TTL
    // eviction) for a dataset that the registry reports as having records,
    // /unprecedentedness must return a retryable StoreUnavailable error (→503),
    // NOT silently score the value against empty history (which would produce a
    // misleading "no historical data" band). Before the fix, get_page returned
    // a 502 here; the D-031 fix makes get_page return an empty page and pushes
    // the retryable signal up at the handler boundary.
    #[tokio::test]
    async fn unprecedentedness_cold_cache_returns_store_unavailable() {
        // Same dataset as unprecedentedness_state, but the records cache is
        // evicted (1s TTL + wait + touch) while the registry still reports
        // record_count > 0.
        let settings = Settings::default();
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let store = Arc::new(hkgov_store::MemoryStore::new(20, 1));
        let id = DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        store
            .register(
                id.clone(),
                "Daily Interbank Liquidity".into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec!["hibor".into()],
                hkgov_common::Cadence::Daily,
            )
            .await;
        // Seed + persist the count, then let the TTL lapse.
        store
            .put_dataset(
                &id,
                vec![hkgov_common::NormalizedRecord {
                    source: DataSource::Hkma,
                    dataset: "daily-interbank-liquidity".into(),
                    record_id: "2026-01".into(),
                    fields: {
                        let mut m = std::collections::BTreeMap::new();
                        m.insert(
                            "hibor_overnight".into(),
                            hkgov_common::RecordValue::Float(1.0),
                        );
                        m
                    },
                    fetched_at: chrono::Utc::now(),
                }],
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let _ = store.meta(&id).await; // touch → trigger lazy eviction

        let state = AppState {
            registry,
            store,
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            daily_view: crate::daily_view::empty_slot(),
            settings: Arc::new(settings),
        };
        let q = Query(UnprecedentednessQuery {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            field: "hibor_overnight".into(),
            value: 2.0,
            k: None,
        });
        let err = unprecedentedness(State(state), q).await.expect_err(
            "cold cache must surface StoreUnavailable (D-031), not score against empty history",
        );
        assert!(
            matches!(err.0, hkgov_common::Error::StoreUnavailable(_)),
            "expected StoreUnavailable, got {:?}",
            err.0
        );
    }

    // ---- /insights/{id}/cite (P-101) --------------------------------------
    //
    // The citation moat: a permalink + citation strings + a reproducibility
    // manifest whose SHA-256 detects upstream data drift. These tests guard the
    // HTTP surface; the rendering + hash math is unit-tested in
    // crates/agent/src/cite.rs.

    /// Build a state with one stored insight + its evidence records, so the cite
    /// route can look it up and compute the manifest hash.
    async fn cite_state() -> AppState {
        let settings = Settings::default();
        let registry = Arc::new(
            hkgov_connectors::registry::Registry::build(&settings).expect("registry builds"),
        );
        let store = Arc::new(hkgov_store::MemoryStore::new(10, 60));
        let insights = Arc::new(hkgov_agent::InsightStore::new());

        // Seed the dataset the insight points at, with its evidence records.
        let id = DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        store
            .register(
                id.clone(),
                "Daily Interbank Liquidity".into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec!["hibor".into()],
                hkgov_common::Cadence::Daily,
            )
            .await;
        let mk_rec = |rid: &str, v: f64| hkgov_common::NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            record_id: rid.into(),
            fields: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "hibor_overnight".into(),
                    hkgov_common::RecordValue::Float(v),
                );
                m
            },
            fetched_at: chrono::Utc::now(),
        };
        store
            .put_dataset(
                &id,
                vec![mk_rec("2026-04-01", 1.0), mk_rec("2026-04-15", 2.0)],
            )
            .await
            .unwrap();

        // Store the insight that cites those records.
        let insight = hkgov_agent::Insight {
            id: "series_jump:hkma:daily-interbank-liquidity:test1".into(),
            kind: "series_jump".into(),
            severity: hkgov_agent::InsightSeverity::Warning,
            title: "hibor_overnight moved +100%".into(),
            summary: "s".into(),
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            evidence: vec![
                hkgov_agent::insight::EvidenceRef {
                    record_id: "2026-04-01".into(),
                    field: "hibor_overnight".into(),
                    value: json!(1.0),
                    context: Some("previous period".into()),
                },
                hkgov_agent::insight::EvidenceRef {
                    record_id: "2026-04-15".into(),
                    field: "hibor_overnight".into(),
                    value: json!(2.0),
                    context: Some("current period".into()),
                },
            ],
            confidence: 0.8,
            generated_at: chrono::Utc::now(),
            producer: "test".into(),
            experimental: false,
            first_seen: None,
            version: 1,
            evolution: None,
        };
        insights.upsert(insight).await;

        AppState {
            registry,
            store,
            insights,
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            daily_view: crate::daily_view::empty_slot(),
            settings: Arc::new(settings),
        }
    }

    #[tokio::test]
    async fn cite_returns_bundle_with_manifest() {
        let state = cite_state().await;
        let q = CiteQuery {
            format: None,
            base_url: Some("https://example.com".into()),
        };
        let resp = cite_insight(
            State(state),
            Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
            Query(q),
        )
        .await
        .unwrap();
        let body = body_string(resp).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(
            v["insight_id"],
            "series_jump:hkma:daily-interbank-liquidity:test1"
        );
        assert!(v["permalink"]
            .as_str()
            .unwrap()
            .starts_with("https://example.com/cite/"));
        assert_eq!(v["manifest"]["detector"], "series_jump");
        assert!(
            v["manifest"]["data_sha256"].as_str().unwrap().len() == 64,
            "sha256 hex is 64 chars"
        );
        assert_eq!(v["cite_version"], "1.0");
    }

    #[tokio::test]
    async fn cite_renders_format_as_text() {
        let state = cite_state().await;
        let q = CiteQuery {
            format: Some("bibtex".into()),
            base_url: Some("https://example.com".into()),
        };
        let resp = cite_insight(
            State(state),
            Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
            Query(q),
        )
        .await
        .unwrap();
        let body = body_string(resp).await;
        assert!(body.starts_with("@misc{"), "bibtex body: {body}");
        assert!(body.contains("howpublished"));
    }

    #[tokio::test]
    async fn cite_unknown_insight_404s() {
        let state = cite_state().await;
        let q = CiteQuery::default();
        let result = cite_insight(State(state), Path("does-not-exist".into()), Query(q)).await;
        assert!(result.is_err(), "unknown insight should error");
        let err = result.unwrap_err();
        assert_eq!(err.0.status_code(), 404);
    }

    #[tokio::test]
    async fn cite_bad_format_400s() {
        let state = cite_state().await;
        let q = CiteQuery {
            format: Some("not-a-format".into()),
            base_url: None,
        };
        let result = cite_insight(
            State(state),
            Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
            Query(q),
        )
        .await;
        assert!(result.is_err(), "bad format should error");
        let err = result.unwrap_err();
        assert_eq!(err.0.status_code(), 400);
    }

    #[tokio::test]
    async fn cite_manifest_is_deterministic() {
        let state = cite_state().await;
        let mk = || {
            (
                State(state.clone()),
                Query(CiteQuery {
                    format: None,
                    base_url: Some("https://x".into()),
                }),
            )
        };
        let (s, q) = mk();
        let a = body_string(
            cite_insight(
                s,
                Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
                q,
            )
            .await
            .unwrap(),
        )
        .await;
        let (s, q) = mk();
        let b = body_string(
            cite_insight(
                s,
                Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
                q,
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(a, b, "same insight + records → byte-identical citation");
    }

    // ---- A-003: the manifest must refuse to hash a PARTIAL evidence set ----
    //
    // `get_by_ids` returns whatever subset of the requested ids it found in the
    // cache. If a real (non-synthetic) evidence record_id is missing — cold
    // cache, LRU/TTL eviction, upstream deletion — hashing the subset would
    // produce a stable-but-wrong data_sha256. Two reviewers would then agree
    // on a hash that doesn't reflect the real data, so a genuine upstream
    // revision would never trip the drift detector. The route must surface a
    // retryable StoreUnavailable (503) instead. Synthetic ids ("series",
    // "threshold", "joined_history") carry derived values in evidence.value
    // and are allowed to be absent.
    #[tokio::test]
    async fn cite_partial_evidence_set_returns_store_unavailable() {
        let state = cite_state().await;
        // Replace the stored insight's evidence with one real id that is NOT
        // in the seeded dataset + one synthetic id. The real id ("9999-12-31")
        // is absent → the manifest must refuse rather than hash the partial set.
        let mut insight = state
            .insights
            .get("series_jump:hkma:daily-interbank-liquidity:test1")
            .await
            .expect("seeded insight present");
        insight.evidence = vec![
            hkgov_agent::insight::EvidenceRef {
                record_id: "9999-12-31".into(), // real-shaped id, absent from cache
                field: "hibor_overnight".into(),
                value: json!(9.0),
                context: Some("missing row".into()),
            },
            hkgov_agent::insight::EvidenceRef {
                record_id: "series".into(), // synthetic — allowed to be absent
                field: "median".into(),
                value: json!(1.5),
                context: Some("series median (MAD baseline)".into()),
            },
        ];
        // NB: `upsert`'s evolution diff does not currently include `evidence`
        // (a separate, out-of-scope gap), so bump the title too to force the
        // evolved version (with the new evidence) to actually persist.
        insight.title = "A-003 partial-set probe".into();
        state.insights.upsert(insight).await;

        let q = CiteQuery {
            format: None,
            base_url: Some("https://x".into()),
        };
        let err = cite_insight(
            State(state),
            Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
            Query(q),
        )
        .await
        .expect_err("partial evidence set must error, not hash (A-003)");
        assert!(
            matches!(err.0, hkgov_common::Error::StoreUnavailable(_)),
            "expected StoreUnavailable for partial set, got {:?}",
            err.0
        );
        assert_eq!(err.0.status_code(), 503);
    }

    #[tokio::test]
    async fn cite_synthetic_only_evidence_set_succeeds() {
        // A-003 complement: an insight whose evidence is entirely synthetic
        // (e.g. a derived-only finding) must still produce a manifest — the
        // synthetic ids are expected-absent by design.
        let state = cite_state().await;
        let mut insight = state
            .insights
            .get("series_jump:hkma:daily-interbank-liquidity:test1")
            .await
            .expect("seeded insight present");
        insight.evidence = vec![hkgov_agent::insight::EvidenceRef {
            record_id: "threshold".into(), // synthetic — allowed absent
            field: "line".into(),
            value: json!(2.5),
            context: Some("watch line".into()),
        }];
        // Force the evolved version to persist (see note in the partial-set
        // test above: upsert's diff doesn't include evidence).
        insight.title = "A-003 synthetic-only probe".into();
        state.insights.upsert(insight).await;

        let q = CiteQuery {
            format: None,
            base_url: Some("https://x".into()),
        };
        let resp = cite_insight(
            State(state),
            Path("series_jump:hkma:daily-interbank-liquidity:test1".into()),
            Query(q),
        )
        .await
        .expect("synthetic-only evidence must produce a manifest");
        let body = body_string(resp).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON: {body}");
        assert_eq!(v["manifest"]["data_sha256"].as_str().unwrap().len(), 64);
    }

    // =========================================================================
    // Phase 5 — security regression + bypass tests for the V-004 / V-005 / V-010
    // fixes. These re-simulate the Phase 3 payloads against the hardened code
    // and assert the attack now fails (and that legitimate use still works).
    // =========================================================================

    use hkgov_agent::Signal;

    /// Mint a real session for an email and return the bearer token. Mirrors
    /// the production flow: request-token → redeem → session_token.
    async fn session_for(state: &AppState, email: &str) -> String {
        let t = state.users.issue_token(email).await;
        let s = state.users.redeem_token(&t.token).await.expect("redeem");
        s.session_token
    }

    /// A valid `Authorization: Bearer` header value for `email`.
    fn auth_header(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        h
    }

    /// Build a minimal compiled scan target (the only fields `signal_id`
    /// hashes over that we care about here).
    fn sample_target() -> hkgov_common::ScanTarget {
        hkgov_common::ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "series_jump".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(25.0),
            ..Default::default()
        }
    }

    // ---- V-005: the magic-link token must NOT be returned by default -------
    //
    // Phase 3 payload: POST /auth/request-token {email} → the token came back
    // in the body, so anyone reading the response could impersonate the owner.
    // Fix: the token field is omitted unless `dev_return_auth_token` is set.

    #[tokio::test]
    async fn v005_token_omitted_from_response_by_default() {
        let state = test_state().await;
        let req = RequestTokenRequest {
            email: "eve@example.com".into(),
        };
        let resp = request_auth_token(State(state), Json(req)).await.unwrap().0;
        // Default config ⇒ no token in the body (skip_serializing_if = None).
        assert!(
            resp.token.is_none(),
            "V-005: token must not be in the response body by default"
        );
        assert!(resp.expires_at > chrono::Utc::now());
    }

    #[tokio::test]
    async fn v005_token_returned_only_in_dev_mode() {
        let mut state = test_state().await;
        let mut settings = (*state.settings).clone();
        settings.api.dev_return_auth_token = true;
        state.settings = Arc::new(settings);
        let req = RequestTokenRequest {
            email: "eve@example.com".into(),
        };
        let resp = request_auth_token(State(state), Json(req)).await.unwrap().0;
        assert!(resp.token.is_some(), "dev mode returns the token for CI");
    }

    // ---- V-004: signals are scoped to the authenticated caller -------------
    //
    // Phase 3 payload A: GET /v1/signals?owner= → returned every user's signals.
    // Fix: list_signals derives the owner from the session and `list_owned`
    // never treats an empty principal as "all".

    #[tokio::test]
    async fn v004_list_without_session_is_rejected() {
        let state = test_state().await;
        let empty = HeaderMap::new();
        let q = ListSignalsQuery { limit: 100 };
        let res = list_signals(State(state), empty, Query(q)).await;
        assert!(res.is_err(), "V-004: no session ⇒ rejected, not a dump");
    }

    #[tokio::test]
    async fn v004_list_returns_only_callers_own_signals() {
        let state = test_state().await;
        // Plant two signals owned by two different principals directly in the
        // store (bypassing create so we don't need two sessions up front).
        let alice = hkgov_agent::user_id_for("alice@example.com");
        let bob = hkgov_agent::user_id_for("bob@example.com");
        state
            .signals
            .create(hkgov_agent::Signal {
                id: "sig:alice:1".into(),
                owner: alice.clone(),
                question: "alice's secret".into(),
                compiled: sample_target(),
                channels: vec![],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .await;
        state
            .signals
            .create(hkgov_agent::Signal {
                id: "sig:bob:1".into(),
                owner: bob.clone(),
                question: "bob's secret".into(),
                compiled: sample_target(),
                channels: vec![],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .await;

        // Alice lists → sees only her own.
        let token = session_for(&state, "alice@example.com").await;
        let got = list_signals(
            State(state),
            auth_header(&token),
            Query(ListSignalsQuery { limit: 100 }),
        )
        .await
        .expect("alice authenticated");
        assert_eq!(got.0.len(), 1, "alice sees only her signal");
        assert_eq!(got.0[0].owner, alice);
        assert!(got.0.iter().all(|s| s.question != "bob's secret"));
    }

    #[tokio::test]
    async fn v004_delete_other_users_signal_fails() {
        // Phase 3 payload B: DELETE /v1/signals/{victim-id} destroyed any
        // signal by id. Fix: delete_owned checks the caller owns it.
        let state = test_state().await;
        state
            .signals
            .create(hkgov_agent::Signal {
                id: "sig:bob:victim".into(),
                owner: hkgov_agent::user_id_for("bob@example.com"),
                question: "bob's".into(),
                compiled: sample_target(),
                channels: vec![],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .await;

        // Alice (a different user) tries to delete bob's signal.
        let alice_token = session_for(&state, "alice@example.com").await;
        let res = delete_signal(
            State(state.clone()),
            auth_header(&alice_token),
            Path("sig:bob:victim".into()),
        )
        .await
        .expect("handler ok");
        let deleted = res.0["deleted"].as_bool().unwrap();
        assert!(!deleted, "V-004: cross-tenant delete must fail");
        // And the record still exists.
        assert!(
            state.signals.get("sig:bob:victim").await.is_some(),
            "victim's signal survives the cross-tenant delete attempt"
        );
    }

    #[tokio::test]
    async fn v004_owner_can_delete_own_signal() {
        // Regression: the fix must not break legitimate self-delete.
        let state = test_state().await;
        let token = session_for(&state, "alice@example.com").await;
        let owner = hkgov_agent::user_id_for("alice@example.com");
        state
            .signals
            .create(hkgov_agent::Signal {
                id: "sig:alice:mine".into(),
                owner,
                question: "mine".into(),
                compiled: sample_target(),
                channels: vec![],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .await;
        let res = delete_signal(
            State(state.clone()),
            auth_header(&token),
            Path("sig:alice:mine".into()),
        )
        .await
        .expect("handler ok");
        assert!(
            res.0["deleted"].as_bool().unwrap(),
            "owner can delete their own signal"
        );
    }

    // ---- V-010: mass-assignment cannot rewrite immutable fields ------------
    //
    // Phase 3 payload: PATCH /v1/signals/{id} with {owner:"attacker"} took over
    // the signal. Fix: the body is a SignalPatch (allow-list of mutable
    // fields); owner/id/created_at are absent from the struct by construction.

    #[tokio::test]
    async fn v010_patch_cannot_steal_ownership() {
        let state = test_state().await;
        let victim = hkgov_agent::user_id_for("victim@example.com");
        state
            .signals
            .create(Signal {
                id: "sig:victim:1".into(),
                owner: victim.clone(),
                question: "original".into(),
                compiled: sample_target(),
                channels: vec![],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .await;

        // Attacker has their OWN session but tries to PATCH the victim's signal.
        // The request can only carry a SignalPatch (no `owner` field exists on
        // the struct), AND the ownership gate rejects the mutation anyway.
        let attacker_token = session_for(&state, "attacker@example.com").await;
        let res = update_signal(
            State(state.clone()),
            auth_header(&attacker_token),
            Path("sig:victim:1".into()),
            Json(hkgov_agent::SignalPatch {
                question: Some("hijacked".into()),
                enabled: Some(false),
                ..Default::default()
            }),
        )
        .await;
        assert!(res.is_err(), "V-010: cross-tenant patch must fail");
        // The victim's signal is unchanged.
        let after = state.signals.get("sig:victim:1").await.unwrap();
        assert_eq!(after.owner, victim, "ownership not rewritten");
        assert_eq!(after.question, "original", "content not mutated");
        assert!(after.enabled, "enabled not flipped");
    }

    #[tokio::test]
    async fn v010_owner_can_patch_own_signal_mutable_fields() {
        // Regression: the patch allow-list must still let an owner edit the
        // mutable fields (question/enabled) of their own signal.
        let state = test_state().await;
        let token = session_for(&state, "alice@example.com").await;
        let owner = hkgov_agent::user_id_for("alice@example.com");
        state
            .signals
            .create(Signal {
                id: "sig:alice:1".into(),
                owner: owner.clone(),
                question: "old".into(),
                compiled: sample_target(),
                channels: vec![],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: None,
            })
            .await;
        let updated = update_signal(
            State(state),
            auth_header(&token),
            Path("sig:alice:1".into()),
            Json(hkgov_agent::SignalPatch {
                question: Some("new".into()),
                enabled: Some(false),
                ..Default::default()
            }),
        )
        .await
        .expect("owner can patch");
        assert_eq!(updated.0.question, "new");
        assert!(!updated.0.enabled);
        // Immutable fields preserved.
        assert_eq!(updated.0.owner, owner);
    }

    // ---- V-004: investigations get the same ownership treatment ------------

    #[tokio::test]
    async fn v004_investigation_list_scoped_to_owner() {
        let state = test_state().await;
        // Two investigations, two owners.
        for (owner, id) in [
            ("alice@example.com", "inv:alice:1"),
            ("bob@example.com", "inv:bob:1"),
        ] {
            state
                .investigations
                .create(hkgov_agent::Investigation {
                    id: id.into(),
                    seed_insight_id: "seed".into(),
                    seed_source: hkgov_common::DataSource::Hkma,
                    seed_dataset: "x".into(),
                    seed_title: "t".into(),
                    title: "t".into(),
                    owner: hkgov_agent::user_id_for(owner),
                    steps: vec![],
                    notes: vec![],
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
                .await;
        }
        let token = session_for(&state, "alice@example.com").await;
        let got = list_investigations(
            State(state),
            auth_header(&token),
            Query(ListInvestigationsQuery { limit: 100 }),
        )
        .await
        .expect("alice authenticated");
        assert_eq!(got.0.len(), 1, "alice sees only her own investigation");
        assert_eq!(got.0[0].id, "inv:alice:1");
    }
}
