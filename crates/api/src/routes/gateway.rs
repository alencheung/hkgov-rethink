//! Open Data Gateway handlers (M1).
//!
//! The dataset-registration + lineage surface: lets an operator register an
//! external dataset into the gateway at runtime and trace every served record
//! back to its upstream source. This is the foundation of the Digital Policy
//! Office "single public data gateway" direction — the platform can serve as
//! the cross-departmental data layer rather than only the static connector
//! catalog.
//!
//! Routes:
//!   POST /v1/datasets                          — register an external dataset
//!   GET  /v1/datasets/{source}/{dataset}/lineage — the dataset's lineage
//!   GET  /v1/lineage                           — all lineage (the provenance index)

use super::*;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use hkgov_common::{Cadence, Category};
use hkgov_store::UpstreamFormat;
use serde::{Deserialize, Serialize};

/// Request body for `POST /v1/datasets`. Registers a dataset's static metadata
/// + seeds its lineage (upstream URL + format) so the gateway can serve it
///   before the first ingest refresh lands records.
#[derive(Deserialize)]
pub(super) struct RegisterDatasetRequest {
    pub source: String,
    pub dataset: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub cadence: Option<String>,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_secs: u64,
    /// The upstream URL the gateway will (or does) fetch from.
    pub upstream_url: String,
    /// Wire format hint for the lineage record. One of: hkma_json, json_array,
    /// json_api, csv, html_next_data, html_table, feed, unknown. Defaults to
    /// unknown.
    #[serde(default)]
    pub upstream_format: Option<String>,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
}

fn default_refresh_interval() -> u64 {
    3600
}

fn default_schema_version() -> String {
    "1".to_string()
}

/// Response for `POST /v1/datasets`: the registered metadata + the seeded
/// lineage (the lineage carries no records yet, so `content_sha256` is the
/// empty-input hash and `record_count_at_fetch` is 0 until the first refresh).
#[derive(Serialize)]
pub(super) struct RegisterDatasetResponse {
    #[serde(flatten)]
    pub meta: hkgov_common::DatasetMeta,
    pub lineage: hkgov_store::DatasetLineage,
}

pub(super) async fn register_dataset(
    State(state): State<AppState>,
    Json(req): Json<RegisterDatasetRequest>,
) -> Result<(StatusCode, Json<RegisterDatasetResponse>), ApiError> {
    // Validate the source against the known enum so a typo doesn't register a
    // phantom dataset that can never be fetched.
    let source = parse_source(&req.source)?;

    // Cadence + category default to Unknown / Other (the same defaults the
    // model uses), parsed leniently so an unknown string doesn't 400 — it just
    // falls back to the default (matches the connector-catalog behavior).
    let cadence = req
        .cadence
        .as_deref()
        .and_then(Cadence::parse)
        .unwrap_or_default();
    let category = req
        .category
        .as_deref()
        .and_then(Category::parse)
        .unwrap_or_default();
    let upstream_format = parse_upstream_format(req.upstream_format.as_deref());

    let id = DatasetId::new(source, req.dataset.clone());

    // Register static metadata (idempotent; preserves any prior record count /
    // last_refreshed_at per the D-029 invariant).
    state
        .store
        .register(
            id.clone(),
            req.title.clone(),
            req.description.clone(),
            req.refresh_interval_secs,
            category,
            req.tags.clone(),
            cadence,
        )
        .await;

    // Seed lineage. No records yet → empty-input hash + zero count. The first
    // connector refresh (or a manual `put_dataset`) will overwrite this with
    // the real content hash.
    let lineage = hkgov_store::lineage_from(
        &id,
        &req.upstream_url,
        upstream_format,
        &req.schema_version,
        &[],
        chrono::Utc::now(),
    );
    state.store.record_lineage(lineage.clone()).await;

    let meta = state
        .store
        .meta(&id)
        .await?
        .ok_or_else(|| hkgov_common::Error::Internal("dataset did not register".into()))?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterDatasetResponse { meta, lineage }),
    ))
}

/// `GET /v1/datasets/{source}/{dataset}/lineage` — the dataset's provenance
/// record (upstream URL, wire format, content hash, fetch timestamp). 404 if
/// the dataset was registered but never fetched (no lineage recorded).
pub(super) async fn dataset_lineage(
    State(state): State<AppState>,
    Path((source, dataset)): Path<(String, String)>,
) -> Result<Json<hkgov_store::DatasetLineage>, ApiError> {
    let source = parse_source(&source)?;
    let id = DatasetId::new(source, dataset);
    let lineage = state
        .store
        .lineage(&id)
        .await?
        .ok_or_else(|| hkgov_common::Error::NotFound("lineage".into()))?;
    Ok(Json(lineage))
}

#[derive(Deserialize, Default)]
pub(super) struct LineageListQuery {
    /// Optional source filter (case-insensitive, same as `/v1/sources`).
    #[serde(default)]
    pub source: Option<String>,
}

/// `GET /v1/lineage` — every dataset's lineage record, the gateway's provenance
/// index. Optionally filtered by `?source=`. Sorted by (source, dataset).
pub(super) async fn list_lineage(
    State(state): State<AppState>,
    Query(q): Query<LineageListQuery>,
) -> Result<Json<Vec<hkgov_store::DatasetLineage>>, ApiError> {
    let source = match q.source.as_deref() {
        Some(s) if !s.trim().is_empty() => Some(parse_source(s)?),
        _ => None,
    };
    let lineage = state.store.lineage_sidecar().list(source).await;
    Ok(Json(lineage))
}

/// Parse the upstream-format hint from the request body. Unknown / missing →
/// `Unknown` (the default); a recognized string maps to the enum. Never errors
/// — an unrecognized format is a documentation lapse, not a 400-worthy fault.
fn parse_upstream_format(s: Option<&str>) -> UpstreamFormat {
    match s.map(|x| x.to_ascii_lowercase()).as_deref() {
        Some("hkma_json") => UpstreamFormat::HkmaJson,
        Some("json_array") => UpstreamFormat::JsonArray,
        Some("json_api") => UpstreamFormat::JsonApi,
        Some("csv") => UpstreamFormat::Csv,
        Some("html_next_data") => UpstreamFormat::HtmlNextData,
        Some("html_table") => UpstreamFormat::HtmlTable,
        Some("feed") => UpstreamFormat::Feed,
        _ => UpstreamFormat::Unknown,
    }
}
