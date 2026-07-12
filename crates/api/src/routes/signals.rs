//! Signal subscription handlers (P-102).
//!
//! Extracted from the main routes module for navigability. The handlers are
//! `pub(super)` so the parent's `router()` can wire them.

use super::*;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct CreateSignalRequest {
    /// The natural-language intent (kept for re-display).
    #[serde(default)]
    question: Option<String>,
    /// The compiled scan target. The caller compiles intent→target client-side
    /// for now (a future `compile_intent` LLM step can move this server-side).
    compiled: hkgov_common::ScanTarget,
    /// Where to push when it fires. v1 stores these; dispatch waits on P-108.
    #[serde(default)]
    channels: Vec<hkgov_agent::SignalChannel>,
}

pub(super) async fn create_signal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateSignalRequest>,
) -> Result<Json<hkgov_agent::Signal>, ApiError> {
    // V-004: owner comes from the authenticated session, NOT the request body.
    // The request no longer even carries an `owner` field — there is no way
    // for a caller to claim another user's identity.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
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
    Ok(Json(state.signals.create(signal).await))
}

#[derive(Deserialize, Default)]
pub(super) struct ListSignalsQuery {
    #[serde(default = "default_limit")]
    pub(super) limit: usize,
}

/// Upper bound on per-user list endpoints (signals/investigations).
pub(super) const MAX_LIST_LIMIT: usize = 100;

pub(super) async fn list_signals(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListSignalsQuery>,
) -> Result<Json<Vec<hkgov_agent::Signal>>, ApiError> {
    // V-004: scope to the authenticated caller only. The old `?owner=` filter
    // let anyone list every user's signals (empty owner = all). `list_owned`
    // returns ONLY the caller's records.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    let limit = q.limit.clamp(1, MAX_LIST_LIMIT);
    Ok(Json(state.signals.list_owned(&owner, limit).await))
}

pub(super) async fn get_signal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Option<hkgov_agent::Signal>>, ApiError> {
    // V-004: ownership-gated read.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    Ok(Json(state.signals.get_owned(&id, &owner).await))
}

pub(super) async fn delete_signal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // V-004: ownership-gated delete. A caller can no longer destroy another
    // user's signal by guessing/enumerating its id.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    let ok = state.signals.delete_owned(&id, &owner).await;
    Ok(Json(serde_json::json!({ "deleted": ok })))
}

pub(super) async fn update_signal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(patch): Json<hkgov_agent::SignalPatch>,
) -> Result<Json<hkgov_agent::Signal>, ApiError> {
    // V-010: the body is now a SignalPatch (an explicit allow-list of mutable
    // fields: question/compiled/channels/enabled). The immutable fields —
    // owner, id, created_at — are absent from the struct, so they can never be
    // rewritten by a request body. V-004: the update is ownership-gated.
    let owner = require_principal(principal_id(&state.users, &headers).await)?;
    match state.signals.update_owned(&id, &owner, patch).await {
        Some(s) => Ok(Json(s)),
        None => Err(ApiError(hkgov_common::Error::NotFound(
            "signal not found".into(),
        ))),
    }
}

#[derive(Deserialize)]
pub(super) struct PreviewSignalRequest {
    /// The compiled scan target to preview.
    compiled: hkgov_common::ScanTarget,
    /// Window in days (default 90).
    #[serde(default = "default_preview_window")]
    window_days: i64,
}

pub(super) fn default_preview_window() -> i64 {
    90
}

pub(super) async fn preview_signal_route(
    State(state): State<AppState>,
    Json(req): Json<PreviewSignalRequest>,
) -> Json<hkgov_agent::SignalPreview> {
    let preview = hkgov_agent::preview_signal(&state.store, &req.compiled, req.window_days).await;
    Json(preview)
}
