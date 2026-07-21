//! Property / NT Metropolis intelligence handlers (M5).
//!
//! Exposes the canonical property projection (M5) over the live store: the
//! cross-portal composite, portal health, and divergence findings. This is the
//! surface an NT-Metropolis planner queries — "what are portals reporting for
//! this region/month, and do they agree?"
//!
//! Routes:
//!   GET /v1/property/composite?region=&month=   — cross-portal median
//!   GET /v1/property/portals                    — portal health + dataset coverage
//!   GET /v1/property/divergence?threshold=      — recent cross-portal divergence

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::Json;
use hkgov_common::DataSource;
use hkgov_connectors::property_canon::{
    build_composite, detect_portal_divergence, project, CanonicalListing, JoinKey,
    DEFAULT_PORTAL_DIVERGENCE_PCT,
};
use hkgov_store::{DatasetId, RecordStore};
use serde::{Deserialize, Serialize};

/// The property-portal sources the canonical projection covers. RVD and
/// LandRegistry are time-series (don't project to listings); the 4 commercial
/// portals do.
const PROJECTABLE_PORTALS: &[DataSource] = &[
    DataSource::Hkp,
    DataSource::Midland,
    DataSource::ChungSen,
    DataSource::AaProperty,
];

#[derive(Deserialize, Default)]
pub struct CompositeQuery {
    /// "hk", "kln", "nt". Omit for all regions.
    #[serde(default)]
    pub region: Option<String>,
    /// "YYYY-MM". Omit for all months.
    #[serde(default)]
    pub month: Option<String>,
}

/// `GET /v1/property/composite?region=&month=` — the cross-portal median
/// per-net-sqft price for a region/month bucket, with per-portal contribution.
/// The composite is what a planner reads; the per-portal breakdown is the
/// transparency layer (which portals agreed, which diverged).
pub async fn property_composite(
    State(state): State<AppState>,
    Query(q): Query<CompositeQuery>,
) -> Result<Json<hkgov_connectors::property_canon::PortalComposite>, ApiError> {
    let mut portal_listings: Vec<(DataSource, Vec<CanonicalListing>)> = Vec::new();
    for &source in PROJECTABLE_PORTALS {
        let listings = collect_projected(&state, source).await;
        if !listings.is_empty() {
            portal_listings.push((source, listings));
        }
    }
    let portal_refs: Vec<(DataSource, &[CanonicalListing])> = portal_listings
        .iter()
        .map(|(s, v)| (*s, v.as_slice()))
        .collect();
    let composite = build_composite(
        q.region.as_deref(),
        q.month.as_deref(),
        &portal_refs,
    );
    Ok(Json(composite))
}

/// `GET /v1/property/portals` — health + dataset coverage for each property
/// portal. Mirrors the `/health/sources` pattern but scoped to the property
/// domain. Shows which portals are active (warmed) and how many listings each
/// contributes — the transparency view behind the composite.
pub async fn property_portals(
    State(state): State<AppState>,
) -> Result<Json<Vec<PortalStatus>>, ApiError> {
    let mut out = Vec::new();
    for &source in PROJECTABLE_PORTALS {
        let datasets = state.store.list(Some(source)).await?;
        let total_records: usize = datasets.iter().map(|d| d.record_count).sum();
        let projected = collect_projected(&state, source).await;
        out.push(PortalStatus {
            source,
            dataset_count: datasets.len(),
            total_records,
            projectable_listings: projected.len(),
            datasets: datasets
                .iter()
                .map(|d| format!("{}/{}", d.source, d.dataset))
                .collect(),
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Serialize)]
pub struct PortalStatus {
    pub source: DataSource,
    pub dataset_count: usize,
    pub total_records: usize,
    pub projectable_listings: usize,
    pub datasets: Vec<String>,
}

#[derive(Deserialize, Default)]
pub struct DivergenceQuery {
    /// Divergence threshold (%) above which to report. Defaults to
    /// DEFAULT_PORTAL_DIVERGENCE_PCT (10%).
    #[serde(default)]
    pub threshold: Option<f64>,
    /// Region to scope to (optional).
    #[serde(default)]
    pub region: Option<String>,
}

/// `GET /v1/property/divergence?threshold=&region=` — recent cross-portal
/// divergence findings. Compares every pair of projectable portals'
/// per-net-sqft medians by (region, month) bucket. The "are the portals
/// telling the same story?" signal — directly analogous to cross_source_gap
/// but for numeric agreement.
pub async fn property_divergence(
    State(state): State<AppState>,
    Query(q): Query<DivergenceQuery>,
) -> Result<Json<Vec<hkgov_connectors::property_canon::PortalDivergenceFinding>>, ApiError> {
    let threshold = q.threshold.unwrap_or(DEFAULT_PORTAL_DIVERGENCE_PCT);
    // Collect each portal's projected listings.
    let mut by_portal: Vec<(DataSource, Vec<CanonicalListing>)> = Vec::new();
    for &source in PROJECTABLE_PORTALS {
        let mut listings = collect_projected(&state, source).await;
        // Optional region scope.
        if let Some(r) = &q.region {
            listings.retain(|l| l.region.as_deref() == Some(r.as_str()));
        }
        if !listings.is_empty() {
            by_portal.push((source, listings));
        }
    }
    // Compare every pair.
    let mut all_findings = Vec::new();
    for i in 0..by_portal.len() {
        for j in (i + 1)..by_portal.len() {
            let (src_a, list_a) = &by_portal[i];
            let (src_b, list_b) = &by_portal[j];
            all_findings.extend(detect_portal_divergence(
                *src_a,
                list_a,
                *src_b,
                list_b,
                JoinKey::RegionAndMonth,
                threshold,
            ));
        }
    }
    Ok(Json(all_findings))
}

/// Collect all records for a property-portal source and project them onto the
/// canonical vocabulary. Lists every dataset the source exposes (each portal
/// has 1-4 datasets); projects each record.
async fn collect_projected(
    state: &AppState,
    source: DataSource,
) -> Vec<CanonicalListing> {
    let mut out = Vec::new();
    let datasets = match state.store.list(Some(source)).await {
        Ok(d) => d,
        Err(_) => return out,
    };
    for meta in datasets {
        let id = DatasetId::new(source, meta.dataset.clone());
        // Page through all records (the composite wants the full picture).
        let mut offset = 0usize;
        loop {
            let page = match state.store.get_page(&id, offset, 500).await {
                Ok(p) => p,
                Err(_) => break,
            };
            if page.records.is_empty() {
                break;
            }
            let len = page.records.len();
            for rec in page.records {
                if let Some(listing) = project(&rec, source) {
                    out.push(listing);
                }
            }
            if len < 500 {
                break;
            }
            offset += len;
        }
    }
    out
}
