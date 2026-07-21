//! Responsible-AI audit handlers (M3).
//!
//! The audit surface for the determinism guarantee: every insight the agent
//! produces carries a ProvenanceRecord attesting how it was made, and these
//! routes expose that trail so a regulator, researcher, or compliance reviewer
//! can verify reproducibility without trusting the system.
//!
//! Routes:
//!   GET /v1/insights/{id}/provenance  — the provenance record for one insight
//!   GET /v1/audit                     — paginated audit log (filterable)
//!   GET /v1/audit/attestation/{id}    — a signed attestation bundle

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use hkgov_agent::{filter_audit, AuditQuery, ProvenanceRecord};
use serde::Deserialize;

/// `GET /v1/insights/{id}/provenance` — the full audit trail for one insight:
/// detector, threshold, evidence hash, producer, deterministic flag. 404 if no
/// provenance was recorded (e.g. an insight from a snapshot taken before M3).
pub async fn insight_provenance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProvenanceRecord>, ApiError> {
    let rec = state
        .provenance
        .get(&id)
        .await
        .ok_or_else(|| hkgov_common::Error::NotFound("provenance".into()))?;
    Ok(Json(rec))
}

#[derive(Deserialize, Default)]
pub struct AuditListQuery {
    /// RFC3339 or epoch seconds; only records produced at/after this time.
    #[serde(default)]
    pub since: Option<String>,
    /// Substring match on producer name (heuristic, or an LLM model id).
    #[serde(default)]
    pub producer: Option<String>,
    /// "true" / "false" / "1" / "0" — filter by the determinism flag.
    #[serde(default)]
    pub deterministic: Option<String>,
    /// Page size, clamped 1..=500 (default 100).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    100
}

const MAX_AUDIT_LIMIT: usize = 500;

/// `GET /v1/audit` — the paginated audit log, newest-first. Filterable by
/// `?since=`, `?producer=`, `?deterministic=true`. This is the surface a
/// regulator queries to answer "which findings are deterministic-reproducible?"
/// vs "which had an LLM in the loop?".
pub async fn list_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditListQuery>,
) -> Result<Json<Vec<ProvenanceRecord>>, ApiError> {
    let limit = q.limit.clamp(1, MAX_AUDIT_LIMIT);
    let since = match q.since.as_deref() {
        Some(s) if !s.trim().is_empty() => Some(parse_since(s)?),
        _ => None,
    };
    let deterministic = match q.deterministic.as_deref() {
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        None => None,
    };
    let query = AuditQuery {
        since,
        producer: q.producer.filter(|s| !s.trim().is_empty()),
        deterministic,
    };
    let all = state.provenance.snapshot().await;
    let mut filtered = filter_audit(all, &query);
    filtered.truncate(limit);
    Ok(Json(filtered))
}

/// `GET /v1/audit/attestation/{id}` — a signed attestation bundle: the insight
/// + its provenance + (when available) the cite reproducibility manifest. The
/// `claim` field is the plain-text attestation a human reviewer reads.
///
/// For deterministic findings the claim asserts byte-reproducibility (re-run
/// the detector, same hash). For LLM-framed findings it honestly states the
/// detection was deterministic but the framing is model-dependent.
pub async fn attestation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<hkgov_agent::Attestation>, ApiError> {
    // The insight must be held to attest it (same accessor the cite route uses).
    let insight = state
        .insights
        .get(&id)
        .await
        .ok_or_else(|| hkgov_common::Error::NotFound("insight".into()))?;

    // The cite manifest is computed from the insight's evidence. When the
    // backing records aren't cached (cold store), the manifest is None and the
    // attestation falls back to the provenance hash alone — still an honest
    // attestation, just without the cite-layer drift anchor.
    let cite_manifest = build_cite_manifest(&state, &insight).await;

    let attestation = hkgov_agent::build_attestation(&insight, &state.provenance, cite_manifest)
        .await
        .ok_or_else(|| hkgov_common::Error::NotFound("provenance".into()))?;
    Ok(Json(attestation))
}

/// Best-effort cite manifest for the attestation. Returns None on any failure
/// (cold cache, missing evidence) — the attestation degrades gracefully to the
/// provenance hash, never false-claims the cite anchor.
async fn build_cite_manifest(
    state: &AppState,
    insight: &hkgov_agent::Insight,
) -> Option<hkgov_agent::ReproducibilityManifest> {
    use hkgov_store::RecordStore;
    // Gather the evidence record_ids that point at real records (skip synthetic
    // markers like "series"/"threshold"/"joined_history").
    let real_ids: Vec<String> = insight
        .evidence
        .iter()
        .filter(|e| !hkgov_agent::cite::is_synthetic_evidence_id(&e.record_id))
        .map(|e| e.record_id.clone())
        .collect();
    if real_ids.is_empty() {
        return None;
    }
    let id = hkgov_store::DatasetId::new(insight.source, insight.dataset.clone());
    let records = state.store.get_by_ids(&id, &real_ids).await.ok()?;
    Some(
        hkgov_agent::build_citation(
            insight,
            &records,
            "http://localhost:8080",
            Some(env!("CARGO_PKG_VERSION")),
        )
        .manifest,
    )
}

/// Parse the `?since=` filter. Accepts RFC3339 or epoch seconds, mirroring the
/// insights `?since=` lifeline filter. Bad value → 400 (not silent fallback).
fn parse_since(s: &str) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    // Try RFC3339 first.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Then epoch seconds.
    if let Ok(secs) = s.parse::<i64>() {
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
            return Ok(dt);
        }
    }
    Err(hkgov_common::Error::BadRequest(
        "since must be RFC3339 or epoch seconds".into(),
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_accepts_rfc3339_and_epoch() {
        assert!(parse_since("2026-07-21T00:00:00Z").is_ok());
        assert!(parse_since("1753000000").is_ok());
    }

    #[test]
    fn parse_since_rejects_garbage() {
        assert!(parse_since("banana").is_err());
    }
}
