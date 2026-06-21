//! Route definitions and the tower middleware stack.
//!
//! v1 surface:
//!   GET /health                      — liveness
//!   GET /sources                     — list ingested datasets
//!   GET /datasets/:source/:dataset   — dataset metadata
//!   GET /datasets/:source/:dataset/records?offset=&limit=
//!                                    — paginated records from cache

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
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

    Router::new()
        .route("/health", get(health))
        .route("/sources", get(list_sources))
        .route("/datasets/:source/:dataset", get(dataset_meta))
        .route("/datasets/:source/:dataset/records", get(dataset_records))
        .route("/", get(root))
        .with_state(state)
        // The middleware stack is the heart of our path to 100k concurrency:
        // timeouts prevent slowloris-style pile-up, gzip shrinks payloads, and
        // trace gives us per-request latency for capacity planning.
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

async fn root() -> Json<Root> {
    Json(Root {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        endpoints: &[
            "GET /health",
            "GET /sources",
            "GET /datasets/:source/:dataset",
            "GET /datasets/:source/:dataset/records",
        ],
    })
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
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

fn parse_source(s: &str) -> Result<DataSource, ApiError> {
    DataSource::parse(s).ok_or_else(|| ApiError(hkgov_common::Error::UnknownSource(s.to_string())))
}
