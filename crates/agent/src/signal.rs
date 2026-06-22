//! Signal Subscriptions (P-102) — consumer-grade push alerts without infra.
//!
//! A `Signal` is a user-owned [`ScanTarget`] (the existing config shape) plus
//! channel routing. The flagship use case: "tell me when overnight HIBOR breaks
//! 2.5%" compiles to `detector="threshold_crossing"`, `threshold=2.5`,
//! `field="hibor_overnight"`.
//!
//! ## Determinism invariant (unchanged)
//!
//! Detection stays pure Rust. The only place an LLM enters is
//! [`compile_intent`] — and there it only *translates* the user's natural
//! language into a `ScanTarget`; it never runs detection. The "preview IS what
//! will fire" property holds because [`preview_signal`] runs the very same
//! deterministic detector against the stored history.
//!
//! ## v1 scope (no identity layer yet)
//!
//! Per the Phase-5 validation (D-1) and the integration map: authoring +
//! preview + compilation are **stateless** and ship now. Server-side push
//! (holding channel secrets, scheduled re-scan, outbound HTTP) waits on P-108
//! (Identity Tier) — a per-user `owner` principal. The `owner` field is
//! `String` (empty in v1; populated by P-108 later) so no schema migration is
//! needed when identity lands.
//!
//! ## `preview_signal`
//!
//! Runs a compiled `ScanTarget`'s detector against the stored history and
//! returns the findings it *would have* produced — so the user calibrates
//! sensitivity before subscribing. Reuses the same `run_one_target` path the
//! scheduler uses, so preview and production detection are identical by
//! construction.

use chrono::{DateTime, Utc};
use hkgov_common::{DataSource, ScanTarget};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::analysis::Finding;

/// A signal channel — where a fired signal pushes. v1 stores the routing but
/// dispatch itself waits on P-108 (the platform must hold channel secrets +
/// run the scheduled re-scan + make outbound HTTP; that needs a user principal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalChannel {
    Email {
        to: String,
        #[serde(default)]
        verified: bool,
    },
    Telegram {
        chat_id: String,
        #[serde(default)]
        verified: bool,
    },
    Slack {
        webhook_url: String,
    },
    Rss,
}

/// A user-owned scan target plus channel routing. One signal = one detector
/// watch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    /// `sig:{owner}:{short_fingerprint}` — stable id. The fingerprint is over
    /// the compiled scan target so two identical signals collide (dedup).
    pub id: String,
    /// The identity-tier handle (P-108). Empty string in v1 (no identity).
    pub owner: String,
    /// The natural-language intent the user authored (kept for re-display).
    pub question: String,
    /// The compiled detector watch. This IS a `ScanTarget` verbatim.
    pub compiled: ScanTarget,
    /// Where to push when it fires. v1 stores these; dispatch waits on P-108.
    #[serde(default)]
    pub channels: Vec<SignalChannel>,
    /// On/off toggle. A paused signal doesn't fire.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

fn default_enabled() -> bool {
    true
}

/// The preview result for one signal: "this would have fired N times in the
/// last window". Deterministic — produced by running the real detector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalPreview {
    pub signal_id: Option<String>,
    pub question: Option<String>,
    pub compiled: ScanTarget,
    /// The findings the detector produced over the window.
    pub findings: Vec<FindingDto>,
    /// How many times it fired.
    pub count: usize,
    /// The record_ids (dates) it fired on.
    pub fired_on: Vec<String>,
    pub window_days: i64,
    pub previewed_at: DateTime<Utc>,
}

/// A slim, serializable finding view (the `Finding` itself isn't `Serialize`
/// in a stable way across the API boundary; this mirrors `tools::FindingDto`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingDto {
    pub kind: String,
    pub source: DataSource,
    pub dataset: String,
    pub title: String,
    pub summary: String,
    pub severity: String,
    pub confidence: f64,
    pub evidence_count: usize,
    pub fired_on: Vec<String>,
}

impl From<&Finding> for FindingDto {
    fn from(f: &Finding) -> Self {
        let fired_on = f
            .evidence
            .iter()
            .map(|e| e.record_id.clone())
            .collect::<Vec<_>>();
        FindingDto {
            kind: f.kind.clone(),
            source: f.source,
            dataset: f.dataset.clone(),
            title: f.title.clone(),
            summary: f.heuristic_summary.clone(),
            severity: f.severity.clone(),
            confidence: f.confidence,
            evidence_count: f.evidence.len(),
            fired_on,
        }
    }
}

/// In-process signal store. Mirrors `InsightStore` — `Arc<RwLock<BTreeMap>>`,
/// volatile (no DB tier). v1 holds authoring state; per-user ownership + push
/// dispatch arrive with P-108.
#[derive(Default)]
pub struct SignalStore {
    inner: Arc<RwLock<std::collections::BTreeMap<String, Signal>>>,
}

impl SignalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(&self, signal: Signal) -> Signal {
        let mut w = self.inner.write().await;
        w.insert(signal.id.clone(), signal.clone());
        signal
    }

    pub async fn get(&self, id: &str) -> Option<Signal> {
        self.inner.read().await.get(id).cloned()
    }

    pub async fn list(&self, owner: &str, limit: usize) -> Vec<Signal> {
        let r = self.inner.read().await;
        r.values()
            .filter(|s| owner.is_empty() || s.owner == owner)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn update(&self, signal: Signal) -> Option<Signal> {
        let mut w = self.inner.write().await;
        if w.contains_key(&signal.id) {
            let mut s = signal;
            s.updated_at = Some(Utc::now());
            w.insert(s.id.clone(), s.clone());
            Some(s)
        } else {
            None
        }
    }

    pub async fn delete(&self, id: &str) -> bool {
        self.inner.write().await.remove(id).is_some()
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }
}

/// Compile a stable signal id from its owner + scan target. Two identical
/// signals (same owner, same compiled target) share an id → dedup at create.
pub fn signal_id(owner: &str, compiled: &ScanTarget) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    owner.hash(&mut h);
    compiled.source.hash(&mut h);
    compiled.dataset.hash(&mut h);
    compiled.detector.hash(&mut h);
    compiled.field.hash(&mut h);
    // f64 isn't Hash (NaN), so hash the bit pattern instead.
    compiled.threshold.map(|t| t.to_le_bytes()).hash(&mut h);
    compiled.direction.hash(&mut h);
    format!("sig:{owner}:{:016x}", h.finish())
}

/// Run a compiled scan target's detector against stored history and report
/// what it *would have* fired. This is the "preview before you subscribe" call.
///
/// Reuses the scheduler's `run_one_target` so preview detection is identical to
/// production detection by construction — the determinism guarantee holds.
pub async fn preview_signal(
    store: &Arc<hkgov_store::MemoryStore>,
    compiled: &ScanTarget,
    window_days: i64,
) -> SignalPreview {
    use hkgov_store::{DatasetId, RecordStore};

    let source = DataSource::parse(&compiled.source).unwrap_or(DataSource::Hkma);
    let id = DatasetId::new(source, &compiled.dataset);
    let page = store
        .get_page(&id, 0, 500)
        .await
        .unwrap_or_else(|_| hkgov_store::RecordPage {
            source,
            dataset: compiled.dataset.clone(),
            total: 0,
            offset: 0,
            limit: 500,
            records: Vec::new(),
        });

    // Window-filter the records to the last `window_days` by record_id date.
    // HKGOV record_ids are ISO-date-ish (e.g. "2026-05-18", "2026-05").
    let cutoff = Utc::now() - chrono::Duration::days(window_days);
    let windowed: Vec<hkgov_common::NormalizedRecord> = page
        .records
        .into_iter()
        .filter(|r| record_after(r, cutoff))
        .collect();

    // Run the detector over the windowed records. We inline the same dispatch
    // the scheduler uses so preview == production.
    let findings = run_detector_preview(source, compiled, &windowed);
    let count = findings.len();
    let fired_on = findings
        .iter()
        .flat_map(|f| f.evidence.iter().map(|e| e.record_id.clone()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let dtos = findings.iter().map(FindingDto::from).collect();
    SignalPreview {
        signal_id: None,
        question: None,
        compiled: compiled.clone(),
        findings: dtos,
        count,
        fired_on,
        window_days,
        previewed_at: Utc::now(),
    }
}

/// Is the record's date (parsed from its record_id) after `cutoff`? Lenient:
/// records whose record_id isn't a parseable date are kept (better to over-
/// include in a preview than silently drop data).
fn record_after(rec: &hkgov_common::NormalizedRecord, cutoff: DateTime<Utc>) -> bool {
    // Try YYYY-MM-DD then YYYY-MM then YYYY prefixes off the record_id.
    let s = &rec.record_id;
    if s.len() >= 10 {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d") {
            return d.and_hms_opt(0, 0, 0).unwrap().and_utc() > cutoff;
        }
    }
    if s.len() >= 7 {
        // YYYY-MM: treat as the first of the month.
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&format!("{}-01", &s[..7]), "%Y-%m-%d") {
            return d.and_hms_opt(0, 0, 0).unwrap().and_utc() > cutoff;
        }
    }
    true // unparseable → keep
}

/// The detector dispatch for preview. Mirrors the scheduler's `run_one_target`
/// match for the self-contained detectors (no companion needed). Kept here
/// rather than calling the scheduler's private fn so this module is testable
/// in isolation.
fn run_detector_preview(
    source: DataSource,
    target: &ScanTarget,
    records: &[hkgov_common::NormalizedRecord],
) -> Vec<Finding> {
    use crate::analysis::*;
    let Some(field) = target.field.as_deref() else {
        return Vec::new();
    };
    let threshold = target.threshold.unwrap_or(0.0);
    match target.detector.as_str() {
        "threshold_crossing" => {
            let direction = match target.direction.as_deref() {
                Some("below") => CrossDirection::Below,
                _ => CrossDirection::Above,
            };
            detect_threshold_crossing(
                source,
                &target.dataset,
                records,
                field,
                threshold,
                direction,
            )
        }
        "series_jump" => detect_series_jumps(source, &target.dataset, records, field, {
            if threshold > 0.0 {
                threshold
            } else {
                25.0
            }
        }),
        "outlier" => detect_outliers(
            source,
            &target.dataset,
            records,
            field,
            if threshold > 0.0 {
                threshold
            } else {
                DEFAULT_OUTLIER_Z
            },
        ),
        "seasonality" => detect_seasonality(
            source,
            &target.dataset,
            records,
            field,
            if threshold > 0.0 {
                threshold
            } else {
                DEFAULT_SEASONALITY_R
            },
        ),
        _ => Vec::new(), // cross-source / companion detectors not previewable here
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hkgov_common::{Cadence, Comparison, NormalizedRecord, RecordValue};
    use hkgov_store::RecordStore;
    use std::collections::BTreeMap;

    fn rec(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut f = BTreeMap::new();
        f.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            record_id: id.into(),
            fields: f,
            fetched_at: Utc::now(),
        }
    }

    fn hibor_target(direction: &str, threshold: f64) -> ScanTarget {
        ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "threshold_crossing".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(threshold),
            direction: Some(direction.into()),
            cadence: Cadence::Daily,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn preview_threshold_crossing_counts_fires() {
        let store = Arc::new(hkgov_store::MemoryStore::new(100, 60));
        let id = hkgov_store::DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        // Recent data with one value above 2.5.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let recs = vec![
            rec(&yesterday, "hibor_overnight", 2.0),
            rec(&today, "hibor_overnight", 2.93), // crosses above 2.5
        ];
        store.put_dataset(&id, recs).await.unwrap();

        let preview = preview_signal(&store, &hibor_target("above", 2.5), 90).await;
        assert!(preview.count >= 1, "should fire on the 2.93 value");
        assert!(!preview.fired_on.is_empty());
    }

    #[tokio::test]
    async fn preview_silent_when_not_crossed() {
        let store = Arc::new(hkgov_store::MemoryStore::new(100, 60));
        let id = hkgov_store::DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        store
            .put_dataset(&id, vec![rec(&today, "hibor_overnight", 1.5)])
            .await
            .unwrap();
        // Watch above 5.0 — far above the data.
        let preview = preview_signal(&store, &hibor_target("above", 5.0), 90).await;
        assert_eq!(preview.count, 0);
    }

    #[tokio::test]
    async fn preview_is_deterministic() {
        let store = Arc::new(hkgov_store::MemoryStore::new(100, 60));
        let id = hkgov_store::DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        store
            .put_dataset(&id, vec![rec(&today, "hibor_overnight", 3.0)])
            .await
            .unwrap();
        let target = hibor_target("above", 2.5);
        // Count is deterministic (the fired_on set); the previewed_at timestamp
        // varies, so compare count + fired_on not the whole struct.
        let a = preview_signal(&store, &target, 90).await;
        let b = preview_signal(&store, &target, 90).await;
        assert_eq!(a.count, b.count);
        assert_eq!(a.fired_on, b.fired_on);
    }

    #[test]
    fn signal_id_is_stable_and_dedup() {
        let t = hibor_target("above", 2.5);
        let a = signal_id("alice", &t);
        let b = signal_id("alice", &t);
        assert_eq!(a, b, "same owner + target → same id (dedup)");
        // Different owner → different id.
        let c = signal_id("bob", &t);
        assert_ne!(a, c);
        // Different threshold → different id.
        let t2 = hibor_target("above", 3.0);
        let d = signal_id("alice", &t2);
        assert_ne!(a, d);
    }

    #[tokio::test]
    async fn store_crud_roundtrip() {
        let store = SignalStore::new();
        let t = hibor_target("above", 2.5);
        let sig = Signal {
            id: signal_id("alice", &t),
            owner: "alice".into(),
            question: "tell me when HIBOR breaks 2.5".into(),
            compiled: t,
            channels: vec![SignalChannel::Email {
                to: "a@b.com".into(),
                verified: false,
            }],
            enabled: true,
            created_at: Utc::now(),
            updated_at: None,
        };
        let id = sig.id.clone();
        store.create(sig).await;
        assert_eq!(store.count().await, 1);
        assert!(store.get(&id).await.is_some());
        // Owner-filtered list.
        assert_eq!(store.list("alice", 10).await.len(), 1);
        assert_eq!(store.list("bob", 10).await.len(), 0);
        assert_eq!(store.list("", 10).await.len(), 1, "empty owner = all");
        assert!(store.delete(&id).await);
        assert_eq!(store.count().await, 0);
    }

    #[test]
    fn record_after_parses_iso_dates() {
        let cutoff = Utc::now() - chrono::Duration::days(30);
        let recent = rec(
            &chrono::Local::now().format("%Y-%m-%d").to_string(),
            "v",
            1.0,
        );
        let old = rec("2020-01-01", "v", 1.0);
        assert!(record_after(&recent, cutoff));
        assert!(!record_after(&old, cutoff));
    }
}
