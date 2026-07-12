//! Investigation case-file handlers (P-105).
//!
//! Extracted from the main routes module for navigability.

use super::signals::MAX_LIST_LIMIT;
use super::*;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct CreateInvestigationRequest {
    /// The Insight.id this case is launched from (the seed).
    seed_insight_id: String,
    /// Snapshot fields (so the case is intelligible if the seed rotates).
    seed_source: String,
    seed_dataset: String,
    seed_title: String,
    /// Optional human-authored title; defaults to the seed title.
    #[serde(default)]
    title: Option<String>,
}

pub(super) async fn create_investigation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateInvestigationRequest>,
) -> Result<Json<hkgov_agent::Investigation>, ApiError> {
    // V-004: owner from the session, not the body.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
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
        owner,
        steps: Vec::new(),
        notes: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    Ok(Json(state.investigations.create(inv).await))
}

#[derive(Deserialize, Default)]
pub(super) struct ListInvestigationsQuery {
    #[serde(default = "default_limit")]
    pub(super) limit: usize,
}

pub(super) async fn list_investigations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListInvestigationsQuery>,
) -> Result<Json<Vec<hkgov_agent::Investigation>>, ApiError> {
    // V-004: scope to the authenticated caller only.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    let limit = q.limit.clamp(1, MAX_LIST_LIMIT);
    Ok(Json(state.investigations.list_owned(&owner, limit).await))
}

pub(super) async fn get_investigation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Option<hkgov_agent::Investigation>>, ApiError> {
    // V-004: ownership-gated read.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    Ok(Json(state.investigations.get_owned(&id, &owner).await))
}

pub(super) async fn delete_investigation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // V-004: ownership-gated delete.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    let ok = state.investigations.delete_owned(&id, &owner).await;
    Ok(Json(serde_json::json!({ "deleted": ok })))
}

#[derive(Deserialize)]
pub(super) struct AppendStepRequest {
    kind: String,
    prompt: String,
    #[serde(default)]
    answer: Option<hkgov_agent::Answer>,
    #[serde(default)]
    trace: Vec<hkgov_agent::TraceStep>,
    #[serde(default)]
    annotation: Option<String>,
}

pub(super) async fn append_investigation_step(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<AppendStepRequest>,
) -> Result<Json<hkgov_agent::Investigation>, ApiError> {
    // V-004: ownership-gated mutation.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
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
    match state
        .investigations
        .append_step_owned(&id, &owner, step)
        .await
    {
        Some(inv) => Ok(Json(inv)),
        None => Err(ApiError(hkgov_common::Error::NotFound(
            "investigation not found".into(),
        ))),
    }
}

#[derive(Deserialize)]
pub(super) struct AddNoteRequest {
    body: String,
}

pub(super) async fn add_investigation_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<AddNoteRequest>,
) -> Result<Json<hkgov_agent::Investigation>, ApiError> {
    // V-004: ownership-gated mutation.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    match state
        .investigations
        .add_note_owned(&id, &owner, req.body)
        .await
    {
        Some(inv) => Ok(Json(inv)),
        None => Err(ApiError(hkgov_common::Error::NotFound(
            "investigation not found".into(),
        ))),
    }
}
