//! Daily-view snapshot layer (performance optimization).
//!
//! The dashboard's hero numbers (Silence Index, Transparency Index, property
//! composite/divergence/portals, the brief) were recomputed on every request
//! — walking the entire `InsightStore` + paging every portal dataset each
//! time, with zero memoization. A cold container restart paid the full ~30s
//! HKMA warmup before any number could render.
//!
//! This module precomputes those hero numbers into a JSON snapshot that the
//! serving routes read on every request (a single `RwLock` read + clone). The
//! snapshot is regenerated on the tail of each agent pass — same cadence the
//! insights already refresh on. A graceful restart now loads yesterday's
//! snapshot from disk in <100ms instead of waiting for warmup.
//!
//! ## Storage choice
//!
//! The composite/transparency/report types in the agent crate carry
//! `&'static str` methodology/publisher fields that can't round-trip through
//! `Deserialize`, and the agent crate doesn't depend on the connectors crate
//! (where the property types live). To avoid threading `Deserialize` through a
//! dozen existing types AND avoid coupling crates, each field is stored as a
//! pre-serialized `serde_json::Value`. On serve we clone the `Value` straight
//! into `Json(...)` — no re-parse, no extra allocation beyond the clone.
//!
//! ## Safety net
//!
//! Every route that reads this snapshot falls back to its original live
//! computation when the snapshot is missing, stale, or the field is absent.
//! The snapshot is purely an optimization; correctness is unchanged.

use crate::state::AppState;
use chrono::{DateTime, Datelike, Utc};
use hkgov_connectors::property_canon::{
    build_composite, detect_portal_divergence, project, CanonicalListing, JoinKey,
    DEFAULT_PORTAL_DIVERGENCE_PCT,
};
use hkgov_store::{DatasetId, RecordStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Schema version of the on-disk snapshot. Bump on any breaking shape change;
/// [`load_from_file`] rejects mismatched versions as a no-op (never blocks boot).
pub const DAILY_VIEW_VERSION: u32 = 1;

/// How long a snapshot is considered "fresh" for the hero read endpoints.
/// Tuned to match the agent cadence (~6h) so we serve cache hits for a full
/// cycle; property numbers refresh faster so they get a shorter budget.
const FRESH_SECS_HERO: i64 = 6 * 3600;
const FRESH_SECS_PROPERTY: i64 = 3600;

/// The property-portal sources the canonical projection covers. Mirrors
/// `crate::routes::property::PROJECTABLE_PORTALS` (can't import: it's private).
const PROJECTABLE_PORTALS: &[hkgov_common::DataSource] = &[
    hkgov_common::DataSource::Hkp,
    hkgov_common::DataSource::Midland,
    hkgov_common::DataSource::ChungSen,
    hkgov_common::DataSource::AaProperty,
];

/// The precomputed daily-view snapshot. Each field is the JSON serialization
/// of the response the corresponding route returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyViewSnapshot {
    pub version: u32,
    pub generated_at: DateTime<Utc>,
    /// Silence Index for each (source, period) bucket the dashboard polls.
    /// Keyed `"{source}:{period}"`. Covers HKMA current + prior quarter (the
    /// dashboard's two-call delta pattern).
    pub silence_index: HashMap<String, serde_json::Value>,
    /// `CompositeTransparencyIndex` for the default source set + period.
    pub transparency_index: Option<serde_json::Value>,
    /// The quarterly report as structured JSON (`TransparencyReport`).
    pub transparency_report_json: Option<serde_json::Value>,
    /// The quarterly report rendered as Markdown text.
    pub transparency_report_markdown: Option<String>,
    /// `GET /v1/property/composite` (all-region/all-month view).
    pub property_composite: Option<serde_json::Value>,
    /// `GET /v1/property/portals`.
    pub property_portals: Option<serde_json::Value>,
    /// `GET /v1/property/divergence` at the default threshold.
    pub property_divergence: Option<serde_json::Value>,
    /// `GET /v1/brief?limit=50`.
    pub brief: Option<serde_json::Value>,
}

impl DailyViewSnapshot {
    /// True when `generated_at` is within `max_age_secs` of `now`.
    pub fn fresh(&self, max_age_secs: i64, now: DateTime<Utc>) -> bool {
        let d = now.signed_duration_since(self.generated_at).num_seconds();
        d >= 0 && d <= max_age_secs
    }

    /// Fetch the Silence Index for a (source, period) bucket, if present.
    pub fn silence(
        &self,
        source: hkgov_common::DataSource,
        period: &str,
    ) -> Option<&serde_json::Value> {
        self.silence_index
            .get(&format!("{}:{}", source.as_str(), period))
    }
}

/// Shared slot the routes read from + the materializer writes to.
pub type DailyViewSlot = Arc<RwLock<Option<DailyViewSnapshot>>>;

/// Construct an empty slot (used by test fixtures + before the first
/// materialize lands). Marked `allow(dead_code)` because it's only referenced
/// from `#[cfg(test)]` modules — without the gate, release builds flag it.
#[allow(dead_code)]
pub fn empty_slot() -> DailyViewSlot {
    Arc::new(RwLock::new(None))
}

/// The on-disk filename (relative to the persist dir).
pub const FILENAME: &str = "daily_view.json";

/// Atomically write the snapshot to `<dir>/daily_view.json`. Mirrors the
/// `hkgov_agent::persist::snapshot_to_file` write-temp-then-rename pattern.
pub async fn save_to_file(dir: &Path, snap: &DailyViewSnapshot) -> anyhow::Result<()> {
    let path = dir.join(FILENAME);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(snap)
        .map_err(|e| anyhow::anyhow!("serialize daily_view: {e}"))?;
    tokio::fs::write(&tmp, json.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("write daily_view tmp {tmp:?}: {e}"))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| anyhow::anyhow!("rename daily_view {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}

/// Restore the snapshot from `<dir>/daily_view.json`. Returns `None` on a
/// missing file, a corrupt JSON, or a version mismatch — a stale or unreadable
/// snapshot never blocks boot; the routes fall back to live compute.
pub async fn load_from_file(dir: &Path) -> Option<DailyViewSnapshot> {
    let path = dir.join(FILENAME);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => return None,
    };
    let snap: DailyViewSnapshot = match serde_json::from_slice(&bytes) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "daily_view.json corrupt — ignoring");
            return None;
        }
    };
    if snap.version != DAILY_VIEW_VERSION {
        tracing::info!(
            on_disk_version = snap.version,
            expected = DAILY_VIEW_VERSION,
            "daily_view.json schema mismatch — ignoring (will regenerate on next pass)"
        );
        return None;
    }
    Some(snap)
}

/// The default source list for the composite transparency snapshot. Matches
/// the route's behavior when `?sources=` is omitted: every source the
/// connector registry declares.
fn default_transparency_sources(state: &AppState) -> Vec<hkgov_common::DataSource> {
    state.registry.sources()
}

// Settings currently unused here — keep the import so future per-route
// shaping (publisher/base_url/top_n) lands without churn.
#[allow(dead_code)]
fn _settings_type_anchor(_s: &hkgov_common::Settings) {}

/// The default silence-index buckets the dashboard polls. The overview page
/// fetches the HKMA score for the current quarter and the prior quarter (to
/// render the period-over-period delta arrow), so we precompute both.
fn default_silence_buckets(now: DateTime<Utc>) -> Vec<(hkgov_common::DataSource, String)> {
    use hkgov_common::DataSource;
    let year = now.format("%Y").to_string();
    let q = (now.month0() / 3) + 1; // 1..=4
    let cur = format!("{year}-Q{q}");
    let prev = if q == 1 {
        format!("{}-Q4", now.year() - 1)
    } else {
        format!("{year}-Q{}", q - 1)
    };
    // Snapshot HKMA (the flagship source) + any other source the dashboard
    // might poll. Cheap to over-include; each entry is one rollup over the
    // already-loaded insight slice.
    let sources = [
        DataSource::Hkma,
        DataSource::Rvd,
        DataSource::LandRegistry,
        DataSource::Immigration,
        DataSource::LandsD,
    ];
    let mut out = Vec::new();
    for s in sources {
        out.push((s, cur.clone()));
        out.push((s, prev.clone()));
    }
    out
}

/// Collect all records for a property-portal source and project them onto the
/// canonical vocabulary. Mirrors `routes::property::collect_projected` (which
/// is private). Paged reads keep the moka lock granular.
async fn collect_projected(
    state: &AppState,
    source: hkgov_common::DataSource,
) -> Vec<CanonicalListing> {
    let mut out = Vec::new();
    let datasets = match state.store.list(Some(source)).await {
        Ok(d) => d,
        Err(_) => return out,
    };
    for meta in datasets {
        let id = DatasetId::new(source, meta.dataset.clone());
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

/// Per-portal status row — mirrors `routes::property::PortalStatus`. Kept here
/// so the snapshot materializer doesn't depend on a private route struct.
#[derive(Serialize)]
struct PortalStatus {
    source: hkgov_common::DataSource,
    dataset_count: usize,
    total_records: usize,
    projectable_listings: usize,
    datasets: Vec<String>,
}

/// Build the snapshot: run every hero computation once against the live
/// `AppState` and serialize the results. Called on the tail of each agent pass
/// and on demand by tests.
///
/// `now` is injected for deterministic timestamps.
pub async fn materialize(state: &AppState, now: DateTime<Utc>) -> DailyViewSnapshot {
    // ---- Silence Index buckets (current + prior quarter for HKMA + peers) ----
    let mut silence_map: HashMap<String, serde_json::Value> = HashMap::new();
    for (source, period) in default_silence_buckets(now) {
        let idx = hkgov_agent::build_silence_index(&state.insights, source, &period, now).await;
        silence_map.insert(
            format!("{}:{}", source.as_str(), period),
            serde_json::to_value(&idx).unwrap_or(serde_json::Value::Null),
        );
    }

    // ---- Composite Transparency Index (default source set, current quarter) ----
    let sources = default_transparency_sources(state);
    let cur_q = format!("{}-Q{}", now.format("%Y"), (now.month0() / 3) + 1);
    let composite =
        hkgov_agent::build_composite_index(&state.insights, &sources, &cur_q, now).await;
    let transparency_index = serde_json::to_value(&composite).ok();

    // ---- Quarterly report (JSON + Markdown) ----
    // Mirrors the route's defaults (`routes::mod::transparency_report_route`):
    // HKMA source, default publisher, localhost base URL, top-10 insights.
    // The snapshot is regenerated each agent pass so the on-disk artifact
    // tracks the latest report rendering without a per-request rebuild.
    let opts = hkgov_agent::ReportOptions::new(hkgov_common::DataSource::Hkma, cur_q.clone())
        .base_url("http://localhost:8080".to_string())
        .publisher(hkgov_agent::DEFAULT_PUBLISHER.to_string())
        .top_n(10);
    let report = hkgov_agent::build_report(&state.insights, &state.provenance, &opts, now).await;
    let transparency_report_markdown = Some(hkgov_agent::render_markdown(&report));
    let transparency_report_json = serde_json::to_value(&report).ok();

    // ---- Property composite / portals / divergence ----
    let mut portal_listings: Vec<(hkgov_common::DataSource, Vec<CanonicalListing>)> = Vec::new();
    let mut portal_status_rows: Vec<PortalStatus> = Vec::new();
    for &source in PROJECTABLE_PORTALS {
        let datasets = state.store.list(Some(source)).await.unwrap_or_default();
        let total_records: usize = datasets.iter().map(|d| d.record_count).sum();
        let listings = collect_projected(state, source).await;
        portal_status_rows.push(PortalStatus {
            source,
            dataset_count: datasets.len(),
            total_records,
            projectable_listings: listings.len(),
            datasets: datasets
                .iter()
                .map(|d| format!("{}/{}", d.source, d.dataset))
                .collect(),
        });
        if !listings.is_empty() {
            portal_listings.push((source, listings));
        }
    }
    let portal_refs: Vec<(hkgov_common::DataSource, &[CanonicalListing])> = portal_listings
        .iter()
        .map(|(s, v)| (*s, v.as_slice()))
        .collect();
    let composite_view = build_composite(None, None, &portal_refs);
    let property_composite = serde_json::to_value(&composite_view).ok();
    let property_portals = serde_json::to_value(&portal_status_rows).ok();

    let mut all_findings = Vec::new();
    for i in 0..portal_listings.len() {
        for j in (i + 1)..portal_listings.len() {
            let (src_a, list_a) = &portal_listings[i];
            let (src_b, list_b) = &portal_listings[j];
            all_findings.extend(detect_portal_divergence(
                *src_a,
                list_a,
                *src_b,
                list_b,
                JoinKey::RegionAndMonth,
                DEFAULT_PORTAL_DIVERGENCE_PCT,
            ));
        }
    }
    let property_divergence = serde_json::to_value(&all_findings).ok();

    // ---- Brief ----
    let brief = hkgov_agent::build_brief(&state.insights, 50, now).await;
    let brief_json = serde_json::to_value(&brief).ok();

    DailyViewSnapshot {
        version: DAILY_VIEW_VERSION,
        generated_at: now,
        silence_index: silence_map,
        transparency_index,
        transparency_report_json,
        transparency_report_markdown,
        property_composite,
        property_portals,
        property_divergence,
        brief: brief_json,
    }
}

/// Read the silence index snapshot for `(source, period)`. Returns the value
/// if the snapshot is fresh, else `None` (caller falls back to live compute).
pub async fn read_silence(
    slot: &DailyViewSlot,
    source: hkgov_common::DataSource,
    period: &str,
    now: DateTime<Utc>,
) -> Option<serde_json::Value> {
    let guard = slot.read().await;
    let snap = guard.as_ref()?;
    if !snap.fresh(FRESH_SECS_HERO, now) {
        return None;
    }
    snap.silence(source, period).cloned()
}

/// Read any of the top-level hero fields (transparency, property, brief) from
/// the snapshot if fresh, else `None`.
pub async fn read_hero(
    slot: &DailyViewSlot,
    field: HeroField,
    now: DateTime<Utc>,
) -> Option<serde_json::Value> {
    let guard = slot.read().await;
    let snap = guard.as_ref()?;
    let max_age = match field {
        HeroField::PropertyComposite
        | HeroField::PropertyPortals
        | HeroField::PropertyDivergence => FRESH_SECS_PROPERTY,
        _ => FRESH_SECS_HERO,
    };
    if !snap.fresh(max_age, now) {
        return None;
    }
    let v = match field {
        HeroField::TransparencyIndex => &snap.transparency_index,
        HeroField::TransparencyReportJson => &snap.transparency_report_json,
        HeroField::PropertyComposite => &snap.property_composite,
        HeroField::PropertyPortals => &snap.property_portals,
        HeroField::PropertyDivergence => &snap.property_divergence,
        HeroField::Brief => &snap.brief,
    };
    v.clone()
}

/// Read the pre-rendered Markdown report (text, not JSON) if fresh.
pub async fn read_report_markdown(slot: &DailyViewSlot, now: DateTime<Utc>) -> Option<String> {
    let guard = slot.read().await;
    let snap = guard.as_ref()?;
    if !snap.fresh(FRESH_SECS_HERO, now) {
        return None;
    }
    snap.transparency_report_markdown.clone()
}

/// Which top-level hero field a route wants to read from the snapshot.
#[derive(Copy, Clone, Debug)]
pub enum HeroField {
    TransparencyIndex,
    TransparencyReportJson,
    PropertyComposite,
    PropertyPortals,
    PropertyDivergence,
    Brief,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use hkgov_common::Settings;
    use std::sync::Arc;

    fn blank_state() -> AppState {
        // Minimal state: every field initialized empty. The materializer
        // should produce a valid (zero-score) snapshot even with no data.
        let settings = Settings::default();
        let store = Arc::new(hkgov_store::MemoryStore::new(10, 60));
        let registry = hkgov_connectors::registry::Registry::build(&settings).unwrap();
        AppState {
            registry: Arc::new(registry),
            store,
            insights: Arc::new(hkgov_agent::InsightStore::new()),
            feedback: Arc::new(hkgov_agent::FeedbackStore::new()),
            signals: Arc::new(hkgov_agent::SignalStore::new()),
            investigations: Arc::new(hkgov_agent::InvestigationStore::new()),
            users: Arc::new(hkgov_agent::UserStore::new()),
            provenance: Arc::new(hkgov_agent::ProvenanceStore::new()),
            llm: Arc::new(hkgov_agent::HeuristicClient::new()),
            alert_log: Arc::new(hkgov_agent::AlertLog::new(200)),
            magic_link_delivery: Arc::new(hkgov_agent::LogMagicLinkDelivery),
            settings: Arc::new(settings),
            daily_view: empty_slot(),
        }
    }

    #[tokio::test]
    async fn materialize_on_empty_state_produces_valid_snapshot() {
        let state = blank_state();
        let now = Utc::now();
        let snap = materialize(&state, now).await;
        assert_eq!(snap.version, DAILY_VIEW_VERSION);
        assert_eq!(snap.generated_at, now);
        // All hero fields are present (even if zero-scored).
        assert!(snap.transparency_index.is_some());
        assert!(snap.transparency_report_json.is_some());
        assert!(snap.transparency_report_markdown.is_some());
        assert!(snap.property_composite.is_some());
        assert!(snap.property_portals.is_some());
        assert!(snap.property_divergence.is_some());
        assert!(snap.brief.is_some());
        // Silence map covers the current + prior quarter for HKMA.
        let hkma_cur = snap
            .silence_index
            .keys()
            .filter(|k| k.starts_with("hkma:"))
            .count();
        assert!(
            hkma_cur >= 2,
            "expected ≥2 HKMA silence entries, got {hkma_cur}"
        );
    }

    #[tokio::test]
    async fn fresh_helper_within_age_budget() {
        let now = Utc::now();
        let snap = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now,
            silence_index: HashMap::new(),
            transparency_index: None,
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: None,
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        assert!(snap.fresh(FRESH_SECS_HERO, now));
        assert!(snap.fresh(FRESH_SECS_HERO, now + chrono::Duration::seconds(100)));
    }

    #[tokio::test]
    async fn fresh_helper_expires_after_age_budget() {
        let now = Utc::now();
        let stale = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now - chrono::Duration::seconds(FRESH_SECS_HERO + 1),
            silence_index: HashMap::new(),
            transparency_index: None,
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: None,
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        assert!(!stale.fresh(FRESH_SECS_HERO, now));
    }

    #[tokio::test]
    async fn snapshot_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!(
            "hkgov-daily-view-rt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let now = Utc::now();
        let snap = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now,
            silence_index: HashMap::new(),
            transparency_index: Some(serde_json::json!({"score": 0.0})),
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: None,
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        save_to_file(&dir, &snap).await.unwrap();
        let loaded = load_from_file(&dir).await.expect("snapshot should load");
        assert_eq!(loaded.version, DAILY_VIEW_VERSION);
        assert_eq!(loaded.generated_at, now);
        assert!(loaded.transparency_index.is_some());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn load_returns_none_when_file_missing() {
        let dir = std::env::temp_dir().join("hkgov-daily-view-missing-nope");
        let loaded = load_from_file(&dir).await;
        assert!(loaded.is_none(), "missing snapshot must not block boot");
    }

    #[tokio::test]
    async fn load_returns_none_on_version_mismatch() {
        let dir = std::env::temp_dir().join(format!(
            "hkgov-daily-view-ver-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // A future-version snapshot — must be rejected, not panic.
        let bad = serde_json::json!({
            "version": 9999,
            "generated_at": Utc::now(),
            "silence_index": {},
            "transparency_index": null,
            "transparency_report_json": null,
            "transparency_report_markdown": null,
            "property_composite": null,
            "property_portals": null,
            "property_divergence": null,
            "brief": null,
        });
        tokio::fs::write(dir.join(FILENAME), bad.to_string())
            .await
            .unwrap();
        let loaded = load_from_file(&dir).await;
        assert!(loaded.is_none(), "version mismatch must be a no-op");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn load_returns_none_on_corrupt_json() {
        let dir = std::env::temp_dir().join(format!(
            "hkgov-daily-view-corrupt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join(FILENAME), "not valid json {{{")
            .await
            .unwrap();
        let loaded = load_from_file(&dir).await;
        assert!(loaded.is_none(), "corrupt JSON must be a no-op");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn read_silence_serves_snapshot_when_fresh() {
        // Prime the slot with a snapshot containing a known silence value.
        let slot = empty_slot();
        let now = Utc::now();
        let mut silence_map = HashMap::new();
        silence_map.insert(
            "hkma:2026-Q2".to_string(),
            serde_json::json!({"score": 42.0, "source": "hkma", "period": "2026-Q2"}),
        );
        let snap = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now,
            silence_index: silence_map,
            transparency_index: None,
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: None,
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        *slot.write().await = Some(snap);
        let v = read_silence(
            &slot,
            hkgov_common::DataSource::Hkma,
            "2026-Q2",
            now + chrono::Duration::seconds(10),
        )
        .await
        .expect("fresh snapshot should serve");
        assert_eq!(v["score"], 42.0);
    }

    #[tokio::test]
    async fn read_silence_falls_back_when_slot_empty() {
        let slot = empty_slot();
        let v = read_silence(&slot, hkgov_common::DataSource::Hkma, "2026-Q2", Utc::now()).await;
        assert!(v.is_none(), "empty slot must signal fallback");
    }

    #[tokio::test]
    async fn read_silence_falls_back_when_stale() {
        let slot = empty_slot();
        let now = Utc::now();
        let stale = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now - chrono::Duration::seconds(FRESH_SECS_HERO + 1),
            silence_index: HashMap::new(),
            transparency_index: None,
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: None,
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        *slot.write().await = Some(stale);
        let v = read_silence(&slot, hkgov_common::DataSource::Hkma, "2026-Q2", now).await;
        assert!(v.is_none(), "stale snapshot must signal fallback");
    }

    #[tokio::test]
    async fn read_hero_serves_property_field_with_property_freshness() {
        // Property fields have a shorter freshness budget than the hero
        // fields. Verify the boundary: a snapshot slightly too old for the
        // property budget is still "fresh" for the hero budget.
        let slot = empty_slot();
        let now = Utc::now();
        // 90 min old: within property budget (1h=3600s) — NO. Set to 30 min
        // so it's fresh for property (1h) AND fresh for hero (6h).
        let age = chrono::Duration::seconds(1800);
        let snap = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now - age,
            silence_index: HashMap::new(),
            transparency_index: Some(serde_json::json!({"score": 0.5})),
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: Some(serde_json::json!({"total_listings": 100})),
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        *slot.write().await = Some(snap);
        let v = read_hero(&slot, HeroField::PropertyComposite, now)
            .await
            .expect("30-min-old property snapshot should be fresh");
        assert_eq!(v["total_listings"], 100);
    }

    #[tokio::test]
    async fn read_hero_rejects_stale_property_field() {
        // A 2h-old property snapshot is past the 1h property budget.
        let slot = empty_slot();
        let now = Utc::now();
        let age = chrono::Duration::seconds(2 * 3600);
        let snap = DailyViewSnapshot {
            version: DAILY_VIEW_VERSION,
            generated_at: now - age,
            silence_index: HashMap::new(),
            transparency_index: Some(serde_json::json!({"score": 0.5})),
            transparency_report_json: None,
            transparency_report_markdown: None,
            property_composite: Some(serde_json::json!({"total_listings": 100})),
            property_portals: None,
            property_divergence: None,
            brief: None,
        };
        *slot.write().await = Some(snap);
        // Property composite is rejected (2h > 1h budget).
        assert!(
            read_hero(&slot, HeroField::PropertyComposite, now)
                .await
                .is_none(),
            "2h-old property snapshot must fall back"
        );
        // But transparency index is still fresh (2h < 6h budget).
        let v = read_hero(&slot, HeroField::TransparencyIndex, now)
            .await
            .expect("2h-old hero snapshot should be fresh");
        assert_eq!(v["score"], 0.5);
    }
}
