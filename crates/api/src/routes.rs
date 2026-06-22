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
        .route("/brief", get(get_brief))
        .route("/alerts", get(list_alerts))
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
            "GET /v1/brief",
            "GET /v1/alerts",
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
}

async fn list_insights(
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> Json<Vec<hkgov_agent::Insight>> {
    Json(state.insights.list(q.limit).await)
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

// ---- GET /alerts — proactive dispatch log ---------------------------------

async fn list_alerts(
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> Json<Vec<hkgov_agent::AlertLogEntry>> {
    Json(state.alert_log.recent(q.limit))
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
}
