//! Route definitions and the tower middleware stack.
//!
//! Surface (v6):
//!   GET  /health                      — liveness
//!   GET  /health/sources              — per-source circuit breaker state
//!   GET  /sources                     — list ingested datasets
//!   GET  /datasets/{source}/{dataset} — dataset metadata
//!   GET  /datasets/{source}/{dataset}/records?offset=&limit=
//!                                    — paginated records from cache
//!   GET  /insights?limit=             — AI-agent generated insights
//!   GET  /alerts?limit=               — proactive alert dispatch log
//!   POST /ask                         — natural-language Q&A over the data

use crate::auth::{guard, make_guard};
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::middleware::from_fn;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hkgov_agent::{heuristic_answer, run_agent_loop, Answer, HeuristicClient, LlmClient, ToolBelt};
use hkgov_common::DataSource;
use hkgov_store::{DatasetId, RecordStore};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub fn router(state: AppState) -> Router {
    let timeout = Duration::from_millis(state.settings.api.request_timeout_ms);
    let api_key = make_guard(state.settings.api.api_key.clone());
    let prefix = state.settings.api.api_prefix.trim_matches('/').to_string();

    // The versioned API routes. State is applied here so the nested router is
    // fully resolved before it's mounted under the prefix.
    let mut api_routes = Router::new()
        .route("/health", get(health))
        .route("/health/sources", get(health_sources))
        .route("/sources", get(list_sources))
        .route("/categories", get(list_categories))
        .route("/datasets/{source}/{dataset}", get(dataset_meta))
        .route("/datasets/{source}/{dataset}/records", get(dataset_records))
        .route("/insights", get(list_insights))
        .route(
            "/insights/{id}/feedback",
            post(submit_feedback).get(get_feedback),
        )
        .route("/insights/{id}/cite", get(cite_insight))
        .route("/insights/{id}/history", get(insight_history))
        .route("/brief", get(get_brief))
        .route("/alerts", get(list_alerts))
        .route("/silence-index", get(silence_index))
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
        .route("/ask", post(ask));

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
    let router = Router::new()
        .route("/", get(root))
        .route("/dashboard", get(dashboard))
        .route("/dashboard/", get(dashboard));
    let router = if prefix.is_empty() {
        // api_routes already carries `/health`; merge brings it to root.
        router.merge(api_routes)
    } else {
        // Nested: root `/health` for probes, api_routes under /{prefix}.
        router
            .route("/health", get(health))
            .nest(&format!("/{prefix}"), api_routes)
    };

    router
        .with_state(state)
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
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
            "GET /v1/health/sources",
            "GET /v1/sources",
            "GET /v1/categories",
            "GET /v1/datasets/{source}/{dataset}",
            "GET /v1/datasets/{source}/{dataset}/records",
            "GET /v1/insights",
            "POST /v1/insights/{id}/feedback",
            "GET /v1/insights/{id}/cite",
            "GET /v1/insights/{id}/history",
            "GET /v1/brief",
            "GET /v1/alerts",
            "GET /v1/silence-index",
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
async fn dashboard(State(_): State<AppState>) -> axum::response::Response {
    const HTML: &str = include_str!("../../../dashboard/index.html");
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        axum::response::Html(HTML),
    )
        .into_response()
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health(State(_): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
struct SourceHealth {
    source: String,
    circuit: &'static str,
}

async fn health_sources(State(state): State<AppState>) -> Json<Vec<SourceHealth>> {
    let states = state.registry.breaker_states();
    Json(
        states
            .into_iter()
            .map(|(s, circuit)| SourceHealth {
                source: s.as_str().to_string(),
                circuit,
            })
            .collect(),
    )
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

async fn dataset_records(
    State(state): State<AppState>,
    Path((source, dataset)): Path<(String, String)>,
    Query(q): Query<PageQuery>,
) -> Result<Json<hkgov_store::RecordPage>, ApiError> {
    let source = parse_source(&source)?;
    let id = DatasetId::new(source, dataset);
    Ok(Json(state.store.get_page(&id, q.offset, q.limit).await?))
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

async fn list_insights(
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> Result<Json<Vec<hkgov_agent::Insight>>, ApiError> {
    let lang = hkgov_agent::Language::parse(q.lang.as_deref());
    // D-007: a present-but-unparseable `since` is a client error, not a silent
    // fallback to the full list. Previously a typo like `?since=banana`
    // returned every insight as if "everything is new since banana" —
    // misleading and a potential surprise leak surface. Now it 400s with a
    // message naming the bad value and the accepted formats.
    let mut insights = if let Some(s) = q.since.as_deref().filter(|s| !s.is_empty()) {
        match parse_since(s) {
            Ok(ts) => state.insights.list_since(q.limit, ts).await,
            Err(()) => {
                return Err(ApiError(hkgov_common::Error::BadRequest(format!(
                    "invalid `since` value: {s:?} (expected RFC 3339 datetime or epoch seconds)"
                ))));
            }
        }
    } else {
        state.insights.list(q.limit).await
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
    let brief = hkgov_agent::build_brief(&state.insights, q.limit, chrono::Utc::now()).await;
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
    let dataset_id = DatasetId::new(insight.source, insight.dataset.clone());
    let page = state.store.get_page(&dataset_id, 0, 500).await?;
    let records = page.records;
    let base_url = q
        .base_url
        .unwrap_or_else(|| "http://localhost:8080".to_string());
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
    Json(state.alert_log.recent(q.limit))
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
    /// Period key like "2026-Q2". Empty/omitted = the latest complete quarter
    /// derivable from the held insights (falls back to "" = all history).
    #[serde(default)]
    period: Option<String>,
}

async fn silence_index(
    State(state): State<AppState>,
    Query(q): Query<SilenceIndexQuery>,
) -> Json<hkgov_agent::SilenceIndex> {
    let period = q.period.unwrap_or_default();
    let idx = hkgov_agent::build_silence_index(&state.insights, &period, chrono::Utc::now()).await;
    Json(idx)
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
    let k = q.k.unwrap_or(hkgov_agent::DEFAULT_BAND_K);
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

// ---- Signals (P-102) — authoring + preview ---------------------------------
//
// A signal is a user-owned ScanTarget plus channel routing. v1 ships authoring
// + preview (stateless). Server-side push (holding channel secrets, scheduled
// re-scan, outbound HTTP) waits on P-108 (identity). The `owner` field is the
// pseudo-identity `X-Reader-Id` header (client-generated UUID) until real auth
// lands — matches the current shared-key trust model.
//
// ⚠️ D-009 (known risk, waived for v1 by design): `owner` is a *filter*, not
// an *ACL*. Any caller holding the shared API key can list (`?owner=` empty →
// all owners), read, update, or delete any other user's signals and
// investigations. This is the documented "shared-key trust model" — every keyed
// caller is mutually trusting. Before multi-tenant deployment, the owner must
// become an enforced principal: derive `owner` from the authenticated session
// (not the request body) and reject cross-owner mutations. Tracked as a risk,
// not fixed here, because the v1 auth model intentionally has a single trust
// domain.

#[derive(Deserialize)]
struct CreateSignalRequest {
    /// The natural-language intent (kept for re-display).
    #[serde(default)]
    question: Option<String>,
    /// The compiled scan target. The caller compiles intent→target client-side
    /// for now (a future `compile_intent` LLM step can move this server-side).
    compiled: hkgov_common::ScanTarget,
    /// Where to push when it fires. v1 stores these; dispatch waits on P-108.
    #[serde(default)]
    channels: Vec<hkgov_agent::SignalChannel>,
    /// Pseudo-identity: a client-generated UUID. Real identity arrives with P-108.
    #[serde(default)]
    owner: Option<String>,
}

async fn create_signal(
    State(state): State<AppState>,
    Json(req): Json<CreateSignalRequest>,
) -> Json<hkgov_agent::Signal> {
    let owner = req.owner.unwrap_or_default();
    let id = hkgov_agent::signal_id(&owner, &req.compiled);
    let signal = hkgov_agent::Signal {
        id,
        owner,
        question: req.question.unwrap_or_default(),
        compiled: req.compiled,
        channels: req.channels,
        enabled: true,
        created_at: chrono::Utc::now(),
        updated_at: None,
    };
    Json(state.signals.create(signal).await)
}

#[derive(Deserialize, Default)]
struct ListSignalsQuery {
    /// Filter to one owner. Empty/omitted = all (the trust model is one shared key).
    #[serde(default)]
    owner: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

async fn list_signals(
    State(state): State<AppState>,
    Query(q): Query<ListSignalsQuery>,
) -> Json<Vec<hkgov_agent::Signal>> {
    Json(
        state
            .signals
            .list(&q.owner.unwrap_or_default(), q.limit)
            .await,
    )
}

async fn get_signal(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Option<hkgov_agent::Signal>> {
    Json(state.signals.get(&id).await)
}

async fn delete_signal(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let ok = state.signals.delete(&id).await;
    Json(serde_json::json!({ "deleted": ok }))
}

async fn update_signal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut signal): Json<hkgov_agent::Signal>,
) -> Result<Json<hkgov_agent::Signal>, ApiError> {
    signal.id = id;
    match state.signals.update(signal).await {
        Some(s) => Ok(Json(s)),
        None => Err(ApiError(hkgov_common::Error::NotFound(
            "signal not found".into(),
        ))),
    }
}

#[derive(Deserialize)]
struct PreviewSignalRequest {
    /// The compiled scan target to preview.
    compiled: hkgov_common::ScanTarget,
    /// Window in days (default 90).
    #[serde(default = "default_preview_window")]
    window_days: i64,
}

fn default_preview_window() -> i64 {
    90
}

async fn preview_signal_route(
    State(state): State<AppState>,
    Json(req): Json<PreviewSignalRequest>,
) -> Json<hkgov_agent::SignalPreview> {
    let preview = hkgov_agent::preview_signal(&state.store, &req.compiled, req.window_days).await;
    Json(preview)
}

// ---- Investigations (P-105) — saved, resumable case files ------------------
//
// From any insight, a user launches a multi-step investigation. v1 stores the
// case file in-process (volatile, no DB tier). The `owner` field is the pseudo-
// identity until P-108; share/resume work via the case-file id over the shared
// API key.

#[derive(Deserialize)]
struct CreateInvestigationRequest {
    /// The Insight.id this case is launched from (the seed).
    seed_insight_id: String,
    /// Snapshot fields (so the case is intelligible if the seed rotates).
    seed_source: String,
    seed_dataset: String,
    seed_title: String,
    /// Optional human-authored title; defaults to the seed title.
    #[serde(default)]
    title: Option<String>,
    /// Pseudo-identity (P-108 lands real identity later).
    #[serde(default)]
    owner: Option<String>,
}

async fn create_investigation(
    State(state): State<AppState>,
    Json(req): Json<CreateInvestigationRequest>,
) -> Result<Json<hkgov_agent::Investigation>, ApiError> {
    let source = parse_source(&req.seed_source)?;
    let now = chrono::Utc::now();
    let id = hkgov_agent::investigation_id(&req.seed_insight_id, now);
    let inv = hkgov_agent::Investigation {
        id,
        seed_insight_id: req.seed_insight_id,
        seed_source: source,
        seed_dataset: req.seed_dataset,
        seed_title: req.seed_title.clone(),
        title: req.title.unwrap_or(req.seed_title),
        owner: req.owner.unwrap_or_default(),
        steps: Vec::new(),
        notes: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    Ok(Json(state.investigations.create(inv).await))
}

#[derive(Deserialize, Default)]
struct ListInvestigationsQuery {
    #[serde(default)]
    owner: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

async fn list_investigations(
    State(state): State<AppState>,
    Query(q): Query<ListInvestigationsQuery>,
) -> Json<Vec<hkgov_agent::Investigation>> {
    Json(
        state
            .investigations
            .list(&q.owner.unwrap_or_default(), q.limit)
            .await,
    )
}

async fn get_investigation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Option<hkgov_agent::Investigation>> {
    Json(state.investigations.get(&id).await)
}

async fn delete_investigation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let ok = state.investigations.delete(&id).await;
    Json(serde_json::json!({ "deleted": ok }))
}

#[derive(Deserialize)]
struct AppendStepRequest {
    kind: String,
    prompt: String,
    #[serde(default)]
    answer: Option<hkgov_agent::Answer>,
    #[serde(default)]
    trace: Vec<hkgov_agent::TraceStep>,
    #[serde(default)]
    annotation: Option<String>,
}

async fn append_investigation_step(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AppendStepRequest>,
) -> Result<Json<hkgov_agent::Investigation>, ApiError> {
    let kind = match req.kind.as_str() {
        "chip" => hkgov_agent::StepKind::Chip,
        "qa" => hkgov_agent::StepKind::Qa,
        "finding_promotion" => hkgov_agent::StepKind::FindingPromotion,
        other => {
            return Err(ApiError(hkgov_common::Error::BadRequest(format!(
                "unknown step kind: {other} (try chip|qa|finding_promotion)"
            ))))
        }
    };
    let step = hkgov_agent::InvestigationStep {
        id: String::new(), // assigned by append_step
        kind,
        prompt: req.prompt,
        answer: req.answer,
        trace: req.trace,
        executed_at: chrono::Utc::now(),
        annotation: req.annotation,
    };
    match state.investigations.append_step(&id, step).await {
        Some(inv) => Ok(Json(inv)),
        None => Err(ApiError(hkgov_common::Error::NotFound(
            "investigation not found".into(),
        ))),
    }
}

#[derive(Deserialize)]
struct AddNoteRequest {
    body: String,
}

async fn add_investigation_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddNoteRequest>,
) -> Result<Json<hkgov_agent::Investigation>, ApiError> {
    match state.investigations.add_note(&id, req.body).await {
        Some(inv) => Ok(Json(inv)),
        None => Err(ApiError(hkgov_common::Error::NotFound(
            "investigation not found".into(),
        ))),
    }
}

// ---- Auth (P-108) — email + magic-link identity ----------------------------
//
// The cheapest identity that unblocks the per-user features (signals,
// investigations, read-state). A user POSTs their email → gets a one-time token
// (returned directly in dev/CI; emailed in production) → redeems it for a
// session handle → uses `Authorization: Bearer {session}` on subsequent calls.
// The `User.id` is the principal the other features key on as `owner`.

#[derive(Deserialize)]
struct RequestTokenRequest {
    email: String,
}

#[derive(Serialize)]
struct TokenResponse {
    /// The one-time token. In dev/CI this is returned directly; in production
    /// it's emailed and this field is omitted.
    token: String,
    /// When the token expires (RFC 3339). The client should re-request after.
    expires_at: chrono::DateTime<chrono::Utc>,
}

async fn request_auth_token(
    State(state): State<AppState>,
    Json(req): Json<RequestTokenRequest>,
) -> Json<TokenResponse> {
    let t = state.users.issue_token(req.email.trim()).await;
    Json(TokenResponse {
        token: t.token,
        expires_at: t.expires_at,
    })
}

#[derive(Deserialize)]
struct RedeemRequest {
    token: String,
}

#[derive(Serialize)]
struct RedeemResponse {
    session_token: String,
    user: hkgov_agent::User,
}

async fn redeem_auth_token(
    State(state): State<AppState>,
    Json(req): Json<RedeemRequest>,
) -> Result<Json<RedeemResponse>, ApiError> {
    let session = state.users.redeem_token(&req.token).await.ok_or_else(|| {
        ApiError(hkgov_common::Error::BadRequest(
            "token invalid, expired, or already used".into(),
        ))
    })?;
    let user = state.users.get(&session.user_id).await.ok_or_else(|| {
        ApiError(hkgov_common::Error::Internal(
            "session minted for unknown user".into(),
        ))
    })?;
    Ok(Json(RedeemResponse {
        session_token: session.session_token,
        user,
    }))
}

/// Resolve the `Authorization: Bearer {session}` header to the current user.
/// Returns 404 when no session is present (not 401, to match the existing
/// auth model — the API-key gate already handles 401).
async fn auth_me(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Json<Option<hkgov_agent::User>> {
    let session = bearer_token(&headers);
    let user = match session {
        Some(s) => state.users.lookup_session(&s).await,
        None => None,
    };
    Json(user)
}

/// Extract the `Bearer {token}` value from an Authorization header, if present.
fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

// ---- POST /ask — natural-language Q&A -------------------------------------

#[derive(Deserialize)]
struct AskRequest {
    question: String,
}

/// Answer a natural-language question about the data.
///
/// Rich mode (LLM configured): drives [`run_agent_loop`], letting the model
/// call store/detector tools and reason to an answer.
/// Heuristic mode (default, no key): [`heuristic_answer`] matches keywords
/// against ingested datasets — useful but shallow.
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

#[cfg(test)]
mod tests {
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
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
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
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
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
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            settings: Arc::new(settings),
        }
    }

    #[tokio::test]
    async fn silence_index_returns_versioned_hkma_scoped_score() {
        let state = silence_state().await;
        let q = SilenceIndexQuery {
            period: Some("2026-Q2".into()),
        };
        let idx = silence_index(State(state), Query(q)).await.0;
        assert_eq!(idx.methodology_version, "1.0");
        assert!(idx.label.contains("HKMA"), "label: {}", idx.label);
        assert_eq!(idx.source, DataSource::Hkma);
        assert_eq!(idx.period, "2026-Q2");
        // One press-only gap → positive score.
        assert!(idx.score > 0.0, "score should be > 0, got {}", idx.score);
        assert!(idx.total_events > 0);
        // Determinism: signals are populated + auditable.
        assert!(!idx.signals.is_empty());
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
        };
        let idx = silence_index(State(empty_state), Query(q)).await.0;
        assert_eq!(idx.score, 0.0);
        assert_eq!(idx.total_events, 0);
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
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
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
            llm: Arc::new(HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
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
}
