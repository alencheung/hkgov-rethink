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
        .route("/datasets/{source}/{dataset}", get(dataset_meta))
        .route("/datasets/{source}/{dataset}/records", get(dataset_records))
        .route("/insights", get(list_insights))
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
    let router = Router::new()
        .route("/", get(root))
        .route("/health", get(health));

    let router = if prefix.is_empty() {
        router.merge(api_routes)
    } else {
        router.nest(&format!("/{prefix}"), api_routes)
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
            "GET /v1/health/sources",
            "GET /v1/sources",
            "GET /v1/datasets/{source}/{dataset}",
            "GET /v1/datasets/{source}/{dataset}/records",
            "GET /v1/insights",
            "GET /v1/alerts",
            "POST /v1/ask",
        ],
    })
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

async fn list_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<hkgov_common::DatasetMeta>>, ApiError> {
    Ok(Json(state.store.list(None).await?))
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
            .register(id.clone(), "Daily Interbank Liquidity".into(), None, 3600)
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
}
