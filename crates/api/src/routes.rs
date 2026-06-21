//! Route definitions and the tower middleware stack.
//!
//! Surface (v3):
//!   GET /health                      — liveness
//!   GET /health/sources              — per-source circuit breaker state
//!   GET /sources                     — list ingested datasets
//!   GET /datasets/{source}/{dataset} — dataset metadata
//!   GET /datasets/{source}/{dataset}/records?offset=&limit=
//!                                    — paginated records from cache
//!   GET /insights?limit=             — AI-agent generated insights

use crate::auth::{guard, make_guard};
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::middleware::from_fn;
use axum::routing::get;
use axum::{Json, Router};
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
        .route("/insights", get(list_insights));

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

fn parse_source(s: &str) -> Result<DataSource, ApiError> {
    DataSource::parse(s).ok_or_else(|| ApiError(hkgov_common::Error::UnknownSource(s.to_string())))
}
