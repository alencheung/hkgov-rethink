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

/// The mutable overlay applied by [`SignalStore::update_owned`]. V-010 fix:
/// `PATCH /v1/signals/{id}` used to accept the full `Signal` body and persist
/// it verbatim, so a caller could rewrite `owner` / `created_at` / `id` and
/// hijack another user's signal. This struct is an **explicit allow-list** of
/// the fields a client may change (`question`, `compiled`, `channels`,
/// `enabled`); every field is `Option` (omitted = leave unchanged) and the
/// immutable fields (`owner`, `id`, `created_at`) are simply absent — there is
/// no way for a request body to set them, by construction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignalPatch {
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub compiled: Option<hkgov_common::ScanTarget>,
    #[serde(default)]
    pub channels: Option<Vec<SignalChannel>>,
    #[serde(default)]
    pub enabled: Option<bool>,
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
    /// D-032: whether the underlying records were resident in the cache when
    /// the preview ran. `false` means the detector saw zero records (cache
    /// cold / refresh in flight / LRU eviction), so `count == 0` here does NOT
    /// mean the signal would never fire — it means "we couldn't evaluate it
    /// right now; retry shortly". The dashboard shows a "data temporarily
    /// unavailable" notice instead of a misleading "0 findings".
    #[serde(default = "default_data_available")]
    pub data_available: bool,
}

fn default_data_available() -> bool {
    true
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

    /// Like [`list`](Self::list), but **never** treats an empty owner as "all".
    /// V-004 fix: the bare `list("", …)` returned every user's signals because
    /// an empty owner matched the `owner.is_empty()` bypass. The authenticated
    /// surface must scope strictly to the caller, so callers pass a resolved
    /// principal here and receive only their own records (empty principal →
    /// empty result, not a dump).
    pub async fn list_owned(&self, owner: &str, limit: usize) -> Vec<Signal> {
        if owner.is_empty() {
            return Vec::new();
        }
        let r = self.inner.read().await;
        r.values()
            .filter(|s| s.owner == owner)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Fetch a signal, but only if `owner` owns it. V-004 fix: the bare `get`
    /// returned any signal by id with no ownership check, enabling cross-tenant
    /// reads/deletes. `None` for unknown OR not-owned — both look the same to
    /// the caller (no existence oracle for another tenant's ids). The empty-owner
    /// bypass is dead backcompat code now that identity is wired and
    /// `principal_id` always returns non-empty — removing it closes a latent
    /// authz hole.
    pub async fn get_owned(&self, id: &str, owner: &str) -> Option<Signal> {
        self.inner
            .read()
            .await
            .get(id)
            .filter(|s| s.owner == owner)
            .cloned()
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

    /// Update a signal owned by `owner`. V-010 fix: [`update`](Self::update)
    /// replaced the stored record wholesale with the caller's body — so a
    /// caller could rewrite `owner`, `created_at`, or `enabled` and hijack the
    /// signal. This variant (a) refuses to mutate a signal the caller doesn't
    /// own, and (b) preserves the immutable fields (`owner`, `id`,
    /// `created_at`) from the stored record, applying only the mutable
    /// overlay (`question`, `compiled`, `channels`, `enabled`).
    pub async fn update_owned(&self, id: &str, owner: &str, patch: SignalPatch) -> Option<Signal> {
        let mut w = self.inner.write().await;
        let existing = w.get_mut(id)?;
        // Ownership gate: a caller who doesn't own the record gets `None`,
        // identical to "not found" (no cross-tenant existence leak). The
        // empty-owner bypass is dead backcompat code now that identity is wired —
        // removing it closes a latent authz hole.
        if existing.owner != owner {
            return None;
        }
        // Apply only the allow-listed mutable fields. Immutable fields
        // (owner/id/created_at) are never taken from the request body.
        if let Some(question) = patch.question {
            existing.question = question;
        }
        if let Some(compiled) = patch.compiled {
            existing.compiled = compiled;
        }
        if let Some(channels) = patch.channels {
            existing.channels = channels;
        }
        if let Some(enabled) = patch.enabled {
            existing.enabled = enabled;
        }
        existing.updated_at = Some(Utc::now());
        Some(existing.clone())
    }

    pub async fn delete(&self, id: &str) -> bool {
        self.inner.write().await.remove(id).is_some()
    }

    /// Delete a signal owned by `owner`. V-004 fix: the bare `delete` removed
    /// any id with no ownership check, so an attacker who learned another
    /// user's signal id (id format is enumerable) could destroy it. This
    /// variant refuses unless the caller owns the record. The empty-owner bypass
    /// is dead backcompat code now that identity is wired — removing it closes a
    /// latent authz hole.
    pub async fn delete_owned(&self, id: &str, owner: &str) -> bool {
        let mut w = self.inner.write().await;
        match w.get(id) {
            Some(s) if s.owner == owner => {
                w.remove(id);
                true
            }
            _ => false,
        }
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }

    // ---- file-based persistence -----

    /// Capture a serializable snapshot for the file-based persistence layer.
    pub async fn snapshot(&self) -> SignalStoreSnapshot {
        SignalStoreSnapshot {
            signals: self.inner.read().await.values().cloned().collect(),
        }
    }

    /// Restore from a snapshot (loaded on boot).
    pub async fn restore(&self, snap: SignalStoreSnapshot) {
        let mut w = self.inner.write().await;
        for s in snap.signals {
            w.insert(s.id.clone(), s);
        }
    }
}

/// Serializable snapshot of [`SignalStore`] state for file-based persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalStoreSnapshot {
    pub signals: Vec<Signal>,
}

/// Stable lowercase slug for a [`Cadence`], mirroring its serde rename. Used
/// only by [`signal_id`] so the identity hash uses the same canonical string a
/// client sends, without requiring `Cadence` to derive `Hash`.
fn cadence_slug(c: hkgov_common::Cadence) -> &'static str {
    use hkgov_common::Cadence;
    match c {
        Cadence::Daily => "daily",
        Cadence::Weekly => "weekly",
        Cadence::Monthly => "monthly",
        Cadence::Quarterly => "quarterly",
        Cadence::Biannual => "biannual",
        Cadence::Annual => "annual",
        Cadence::Unknown => "unknown",
    }
}

/// Stable snake_case slug for a [`Comparison`], mirroring its serde rename.
fn comparison_slug(c: hkgov_common::Comparison) -> &'static str {
    use hkgov_common::Comparison;
    match c {
        Comparison::PeriodOverPeriod => "period_over_period",
        Comparison::YearOverYear => "year_over_year",
    }
}

/// Compile a stable signal id from its owner + scan target. Two identical
/// signals (same owner, same compiled target) share an id → dedup at create.
///
/// D-023: the identity set previously omitted `cadence`, `comparison`,
/// `field_b`, `companion`, `companion_field`, and `join_field`. That made two
/// semantically distinct signals collide — e.g. a `series_jump` with
/// `cadence=Daily` and one with `cadence=Quarterly` got the same id, so the
/// later create silently overwrote the earlier (the cadence changes the fire
/// set, which is exactly what D-006 fixed at the detection level). Every field
/// that affects detection is now part of the id.
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
    // D-023: the detection-affecting fields that were missing. `Cadence` and
    // `Comparison` are Eq but not Hash (no Hash derive on the enums), and
    // `CompanionRef` is a struct in another crate — so hash stable string
    // forms instead of the values. The serde renames (lowercase /
    // snake_case) give a canonical, version-stable representation, mirrored
    // here so the id matches what a client would send over the wire.
    cadence_slug(compiled.cadence).hash(&mut h);
    comparison_slug(compiled.comparison).hash(&mut h);
    compiled.field_b.hash(&mut h);
    if let Some(c) = &compiled.companion {
        h.write_u8(1);
        c.source.hash(&mut h);
        c.dataset.hash(&mut h);
    } else {
        // Distinguish Some from None when companion is unset.
        h.write_u8(0);
    }
    compiled.companion_field.hash(&mut h);
    compiled.join_field.hash(&mut h);
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
    use hkgov_store::DatasetId;

    let source = DataSource::parse(&compiled.source).unwrap_or(DataSource::Hkma);
    let id = DatasetId::new(source, &compiled.dataset);

    // D-020: paginate through ALL records, not just the first 500-row page. The
    // scheduler's `collect_all_records` exists for exactly this reason — a single
    // `get_page(id, 0, 500)` silently truncated any dataset larger than 500 rows,
    // so a jump on row 600 was invisible to preview but visible in production.
    // This mirrors the scheduler's loader so preview sees the whole feed.
    let (all_records, data_available) = collect_all_records_for_preview(store, &id).await;

    // Window-filter the records to the last `window_days` by record_id date.
    // HKGOV record_ids are ISO-date-ish (e.g. "2026-05-18", "2026-05").
    let cutoff = Utc::now() - chrono::Duration::days(window_days);
    let windowed: Vec<hkgov_common::NormalizedRecord> = all_records
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
        data_available,
    }
}

/// Paginate through ALL records for a dataset for preview, mirroring the
/// scheduler's `collect_all_records`. The store's `get_page` caps at 500 rows,
/// so a single page silently truncated any larger dataset (D-020). This pages
/// until a short/empty batch so the detector sees the whole feed.
///
/// D-032: returns `(records, data_available)`. `data_available` is false when
/// the dataset's registry entry reports records but the cache returned none
/// (cold cache / refresh in flight / LRU eviction) — so the caller can
/// distinguish "0 findings because nothing fired" from "0 findings because we
/// had no data to evaluate."
async fn collect_all_records_for_preview(
    store: &Arc<hkgov_store::MemoryStore>,
    id: &hkgov_store::DatasetId,
) -> (Vec<hkgov_common::NormalizedRecord>, bool) {
    use hkgov_store::RecordStore;
    let mut all = Vec::new();
    let mut offset = 0usize;
    loop {
        match store.get_page(id, offset, 500).await {
            Ok(page) => {
                if page.records.is_empty() {
                    break;
                }
                let len = page.records.len();
                all.extend(page.records);
                if len < 500 {
                    break;
                }
                offset += len;
            }
            // A-009: a mid-pagination error must be observable, not silently
            // indistinguishable from "no more pages". The preview path already
            // has a `data_available` flag below; a partial result from an error
            // is a transient condition, so log it and let the existing
            // cold-cache check (via meta()) report data_available=false when
            // the count is suspiciously low.
            Err(e) => {
                tracing::warn!(
                    source = %id.source,
                    dataset = %id.dataset,
                    offset = offset,
                    error = %e,
                    "preview: get_page errored mid-pagination; collected {} records so far (A-009)",
                    all.len()
                );
                break;
            }
        }
    }
    // D-032: if we collected nothing, check whether the dataset *should* have
    // records (registry persisted count > 0). If so, the cache is cold and the
    // empty result is not meaningful — flag it so preview can tell the user.
    let data_available = if all.is_empty() {
        match store.meta(id).await {
            Ok(Some(m)) => m.record_count == 0, // genuinely empty dataset is "available"
            _ => true, // unknown dataset: assume available to avoid false alarms
        }
    } else {
        true
    };
    (all, data_available)
}

/// Is the record's date (parsed from its record_id) after `cutoff`? Lenient:
/// records whose record_id isn't a parseable date are kept (better to over-
/// include in a preview than silently drop data).
fn record_after(rec: &hkgov_common::NormalizedRecord, cutoff: DateTime<Utc>) -> bool {
    // Try YYYY-MM-DD then YYYY-MM then YYYY prefixes off the record_id.
    let s = &rec.record_id;
    if s.len() >= 10 {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d") {
            // Midnight (0,0,0) is always a valid time of day, so this never returns None.
            return d
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always a valid time")
                .and_utc()
                > cutoff;
        }
    }
    if s.len() >= 7 {
        // YYYY-MM: treat as the first of the month.
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&format!("{}-01", &s[..7]), "%Y-%m-%d") {
            return d
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always a valid time")
                .and_utc()
                > cutoff;
        }
    }
    true // unparseable → keep
}

/// The detector dispatch for preview. **Must mirror the scheduler's
/// `run_one_target` match exactly** — the determinism invariant (the module's
/// raison d'être) is "preview IS what will fire". D-006 was precisely that this
/// had drifted: preview called the unscaled `detect_series_jumps` while the
/// scheduler called the cadence-scaled `detect_series_jumps_cadenced`, so a
/// quarterly/monthly signal preview lied about its fire set.
///
/// The rule: every self-contained detector arm here calls the *same* function
/// the scheduler does, with the *same* threshold-defaulting and the *same*
/// cadence/comparison arguments. Companion detectors (`cross_source_gap`,
/// `proxy_divergence`, `benchmark_deviation`) aren't previewable here because
/// they need a second dataset loaded — they return empty (documented).
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
        "series_jump" => {
            // D-006 fix: mirror the scheduler. The scheduler routes a YoY-
            // comparison `series_jump` to `detect_year_over_year` and otherwise
            // uses the cadence-scaled `detect_series_jumps_cadenced`. Previewing
            // the unscaled `detect_series_jumps` made quarterly/monthly signals
            // report a different fire set than production.
            if matches!(target.comparison, hkgov_common::Comparison::YearOverYear) {
                let ppy = target.cadence.periods_per_year().round() as usize;
                detect_year_over_year(
                    source,
                    &target.dataset,
                    records,
                    field,
                    if threshold > 0.0 {
                        threshold
                    } else {
                        DEFAULT_PCT_THRESHOLD
                    },
                    ppy.max(1),
                )
            } else {
                let t = if threshold > 0.0 {
                    threshold
                } else {
                    crate::analysis::DEFAULT_SERIES_JUMP_WATCH_PCT
                };
                detect_series_jumps_cadenced(
                    source,
                    &target.dataset,
                    records,
                    field,
                    t,
                    target.cadence,
                )
            }
        }
        "year_over_year" => {
            // D-006 fix (second half): this arm was missing entirely, so YoY
            // signals returned an empty preview regardless of data.
            let ppy = target.cadence.periods_per_year().round() as usize;
            detect_year_over_year(
                source,
                &target.dataset,
                records,
                field,
                if threshold > 0.0 {
                    threshold
                } else {
                    DEFAULT_PCT_THRESHOLD
                },
                ppy.max(1),
            )
        }
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
        // D-019: `correlation` is a single-dataset detector (both fields live on
        // the same records) and so IS previewable — yet it was absent from this
        // dispatch, so a correlation signal previewed as 0 findings even when the
        // scheduler would fire. The scheduler arm (scheduler.rs:300) and the tool
        // arm (tools.rs:586) both handle it; preview is the third dispatch site
        // and must agree. The `field_b` requirement mirrors the scheduler's guard.
        "correlation" => {
            let Some(field_b) = target.field_b.as_deref() else {
                return Vec::new();
            };
            detect_correlation(
                source,
                &target.dataset,
                records,
                field,
                field_b,
                if threshold > 0.0 {
                    threshold
                } else {
                    DEFAULT_CORRELATION_R
                },
            )
        }
        "trend_break" => {
            // Mirror the scheduler: `threshold` is the min-run-length here
            // (> 0 overrides DEFAULT_TREND_BREAK_MIN_RUN). Single-dataset
            // detector, so previewable without a companion.
            let min_run = if threshold > 0.0 {
                threshold as usize
            } else {
                DEFAULT_TREND_BREAK_MIN_RUN
            };
            detect_trend_break(source, &target.dataset, records, field, min_run)
        }
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

    #[tokio::test]
    async fn preview_trend_break_fires_on_reversal() {
        // Guards the trend_break preview wiring: before the arm was added, a
        // trend_break signal previewed empty even though it would fire in
        // production. The same series (rising 3 periods, then a reversal) must
        // preview >= 1 finding.
        let store = Arc::new(hkgov_store::MemoryStore::new(100, 60));
        let id = hkgov_store::DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        let recs = vec![
            rec("2026-06-20", "hibor_overnight", 1.0),
            rec("2026-06-21", "hibor_overnight", 1.5), // rising
            rec("2026-06-22", "hibor_overnight", 2.0), // rising
            rec("2026-06-23", "hibor_overnight", 2.5), // rising (3-period run)
            rec("2026-06-24", "hibor_overnight", 2.0), // reversal → break
        ];
        store.put_dataset(&id, recs).await.unwrap();
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "trend_break".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(3.0),
            direction: None,
            cadence: Cadence::Daily,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        };
        let preview = preview_signal(&store, &target, 90).await;
        assert!(
            preview.count >= 1,
            "trend_break preview must fire on a 3-period rise then reversal"
        );
        assert!(!preview.fired_on.is_empty());
    }

    // ---- D-006 regression: preview MUST equal production detection ---------
    //
    // The signal module's whole contract is "preview IS what will fire"
    // (see the module doc). D-006 broke this for `series_jump` on non-Unknown
    // cadences: preview called the unscaled detector, production called the
    // cadence-scaled one. This test asserts the fix by calling BOTH the preview
    // dispatch AND the cadenced detector on identical inputs and requiring
    // identical findings. A regression here means the two paths drifted again.

    /// Build records for a `+pct%` jump between two consecutive periods.
    fn jump_records(from: f64, pct: f64) -> Vec<NormalizedRecord> {
        let to = from * (1.0 + pct / 100.0);
        vec![rec("2026-Q1", "v", from), rec("2026-Q2", "v", to)]
    }

    #[test]
    fn d006_quarterly_series_jump_preview_matches_production() {
        // +35% jump, base threshold 25%, QUARTERLY cadence.
        // - Cadenced (production) effective threshold = 25 * sqrt(12/4) = 43.3%,
        //   so 35% does NOT fire → production must report 0 findings.
        // - Before D-006 fix, preview used the unscaled 25% and reported 1.
        let records = jump_records(100.0, 35.0);
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(25.0),
            cadence: Cadence::Quarterly,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        };
        // Production path (what the scheduler runs).
        let prod = crate::analysis::detect_series_jumps_cadenced(
            DataSource::Hkma,
            "x",
            &records,
            "v",
            25.0,
            Cadence::Quarterly,
        );
        // Preview path (what preview_signal runs).
        let prev = run_detector_preview(DataSource::Hkma, &target, &records);
        assert_eq!(
            prod.len(),
            prev.len(),
            "D-006: preview ({} findings) must equal production ({}) for quarterly series_jump",
            prev.len(),
            prod.len()
        );
        assert!(
            prev.is_empty(),
            "D-006: 35% jump under a 43.3% quarterly threshold must NOT fire in preview"
        );
    }

    #[test]
    fn d006_monthly_series_jump_preview_matches_production() {
        // +30% jump, base 25%, MONTHLY → scale sqrt(12/12)=1.0 → eff 25% → fires.
        // (Monthly is the no-op scaling case, but it still must match exactly.)
        let records = jump_records(100.0, 30.0);
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(25.0),
            cadence: Cadence::Monthly,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        };
        let prod = crate::analysis::detect_series_jumps_cadenced(
            DataSource::Hkma,
            "x",
            &records,
            "v",
            25.0,
            Cadence::Monthly,
        );
        let prev = run_detector_preview(DataSource::Hkma, &target, &records);
        assert_eq!(prod.len(), prev.len());
        assert_eq!(prev.len(), 1, "30% > 25% monthly threshold → fires");
    }

    #[test]
    fn d006_unknown_cadence_preview_matches_production() {
        // Unknown cadence: scaling is a no-op (factor 1.0), so preview and prod
        // agree trivially. Guards that the fix didn't break the default path.
        let records = jump_records(100.0, 30.0);
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(25.0),
            cadence: Cadence::Unknown,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        };
        let prod = crate::analysis::detect_series_jumps_cadenced(
            DataSource::Hkma,
            "x",
            &records,
            "v",
            25.0,
            Cadence::Unknown,
        );
        let prev = run_detector_preview(DataSource::Hkma, &target, &records);
        assert_eq!(prod.len(), prev.len());
        assert_eq!(prev.len(), 1);
    }

    #[test]
    fn d006_yoy_series_jump_preview_runs() {
        // The YoY-comparison `series_jump` arm was missing from preview entirely
        // before D-006. With enough periods it must now delegate to
        // detect_year_over_year and surface a finding, not silently return empty.
        //
        // detect_year_over_year needs series.len() >= periods_per_year + MIN_YOY_SAMPLES.
        // For QUARTERLY (ppy=4, MIN_YOY_SAMPLES=4) that's >= 8 records. We build
        // 8 quarters where Q4 of year 2 is +50% over Q4 of year 1 (idx 7 vs idx 3).
        let mut records = Vec::new();
        let baseline = [100.0, 102.0, 101.0, 100.0]; // year 1, quarters 1-4
        let year2 = [103.0, 101.0, 102.0, 150.0]; // year 2: Q4 jumps +50% vs year1 Q4
        for (q, v) in baseline.iter().enumerate() {
            records.push(rec(&format!("2025-Q{}", q + 1), "v", *v));
        }
        for (q, v) in year2.iter().enumerate() {
            records.push(rec(&format!("2026-Q{}", q + 1), "v", *v));
        }
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(15.0),
            cadence: Cadence::Quarterly,
            comparison: Comparison::YearOverYear,
            ..Default::default()
        };
        let prev = run_detector_preview(DataSource::Hkma, &target, &records);
        // Cross-check against the production detector directly.
        let prod = crate::analysis::detect_year_over_year(
            DataSource::Hkma,
            "x",
            &records,
            "v",
            15.0,
            4, // quarterly
        );
        assert_eq!(
            prev.len(),
            prod.len(),
            "D-006: YoY series_jump preview must match production (prev={}, prod={})",
            prev.len(),
            prod.len()
        );
        assert!(
            !prev.is_empty(),
            "D-006: +50% YoY jump must surface a finding in preview"
        );
        assert_eq!(prev[0].kind, "year_over_year");
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

    // ---- D-019: correlation must be previewable (was missing from dispatch) --
    //
    // `correlation` is a single-dataset detector (both fields on the same
    // records), so it is fully previewable — yet the preview dispatch had no
    // `correlation` arm and fell through to the empty catch-all. A correlation
    // signal previewed as 0 findings even when the scheduler would fire. This
    // is the same defect class as the historical D-006/D-013 (a detector wired
    // into the scheduler but silently empty in preview).

    /// Build a record with TWO numeric fields (needed for correlation).
    fn rec2(id: &str, a: &str, av: f64, b: &str, bv: f64) -> NormalizedRecord {
        let mut f = BTreeMap::new();
        f.insert(a.into(), RecordValue::Float(av));
        f.insert(b.into(), RecordValue::Float(bv));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields: f,
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn d019_correlation_preview_fires_when_decoupled() {
        // Two fields that DECOUPLE: a rises monotonically while b is random →
        // |r| is low → correlation detector fires (it flags LOW correlation).
        let records: Vec<NormalizedRecord> = (1..20)
            .map(|i| {
                rec2(
                    &format!("2026-{:02}-01", i),
                    "price",
                    i as f64, // rises monotonically
                    "volume",
                    ((i * 7) % 5) as f64, // pseudo-random, uncorrelated
                )
            })
            .collect();
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "correlation".into(),
            field: Some("price".into()),
            field_b: Some("volume".into()),
            threshold: Some(0.3),
            cadence: Cadence::Monthly,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        };
        let prev = run_detector_preview(DataSource::Hkma, &target, &records);
        // Cross-check against the production detector directly.
        let prod = crate::analysis::detect_correlation(
            DataSource::Hkma,
            "x",
            &records,
            "price",
            "volume",
            0.3,
        );
        assert_eq!(
            prev.len(),
            prod.len(),
            "D-019: correlation preview ({}) must equal production ({})",
            prev.len(),
            prod.len()
        );
        assert!(
            !prev.is_empty(),
            "D-019: decoupled series must surface a correlation finding in preview"
        );
        assert_eq!(prev[0].kind, "correlation");
    }

    #[test]
    fn d019_correlation_preview_missing_field_b_is_empty() {
        // No field_b → the arm returns empty (mirrors the scheduler's guard).
        let records = vec![rec2("2026-01-01", "price", 1.0, "volume", 2.0)];
        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "correlation".into(),
            field: Some("price".into()),
            field_b: None,
            ..Default::default()
        };
        let prev = run_detector_preview(DataSource::Hkma, &target, &records);
        assert!(prev.is_empty(), "correlation without field_b must be empty");
    }

    // ---- D-020: preview must paginate, not truncate at 500 rows -------------
    //
    // Before D-020, preview called a single get_page(id, 0, 500), so a dataset
    // with >500 rows was scored on its first 500 records only. A jump on row
    // 600 was invisible to preview but visible in production. The scheduler
    // paginates via collect_all_records; preview now does the same.

    #[tokio::test]
    async fn d020_preview_sees_records_beyond_first_page() {
        let store = Arc::new(hkgov_store::MemoryStore::new(2000, 3600));
        let id = hkgov_store::DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        // Build 600 recent records: a flat baseline, then a +1000% jump on the
        // very last record (row 600 — beyond the old 500-row truncation).
        let today = chrono::Local::now().date_naive();
        let mut recs = Vec::new();
        for i in 0..599 {
            let d = today - chrono::Duration::days(599 - i);
            recs.push(rec(
                &d.format("%Y-%m-%d").to_string(),
                "hibor_overnight",
                1.0,
            ));
        }
        // The 600th record: a massive jump that a series_jump MUST catch.
        recs.push(rec(
            &today.format("%Y-%m-%d").to_string(),
            "hibor_overnight",
            11.0,
        ));
        store.put_dataset(&id, recs).await.unwrap();

        let target = ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "series_jump".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(100.0), // 1000% jump >> 100% threshold
            cadence: Cadence::Daily,
            comparison: Comparison::PeriodOverPeriod,
            ..Default::default()
        };
        let preview = preview_signal(&store, &target, 3650).await;
        assert!(
            preview.count >= 1,
            "D-020: preview must see the jump on record 600 (beyond the old 500-row truncation); got {} findings",
            preview.count
        );
    }

    // ---- D-023: signal_id must distinguish detection-affecting fields --------
    //
    // Before D-023, signal_id hashed only (owner, source, dataset, detector,
    // field, threshold, direction) — omitting cadence, comparison, field_b,
    // companion, companion_field, join_field. Two semantically distinct signals
    // collided and one silently overwrote the other on create. The most acute
    // case: series_jump where cadence changes the fire set.

    #[test]
    fn d023_different_cadence_yields_different_id() {
        let base = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(25.0),
            ..Default::default()
        };
        let daily = ScanTarget {
            cadence: Cadence::Daily,
            ..base.clone()
        };
        let quarterly = ScanTarget {
            cadence: Cadence::Quarterly,
            ..base
        };
        assert_ne!(
            signal_id("alice", &daily),
            signal_id("alice", &quarterly),
            "D-023: different cadence must not collide (cadence changes the fire set)"
        );
    }

    #[test]
    fn d023_different_comparison_yields_different_id() {
        let base = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(25.0),
            cadence: Cadence::Quarterly,
            ..Default::default()
        };
        let pop = ScanTarget {
            comparison: Comparison::PeriodOverPeriod,
            ..base.clone()
        };
        let yoy = ScanTarget {
            comparison: Comparison::YearOverYear,
            ..base
        };
        assert_ne!(
            signal_id("alice", &pop),
            signal_id("alice", &yoy),
            "D-023: different comparison must not collide"
        );
    }

    #[test]
    fn d023_different_field_b_yields_different_id() {
        let base = ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "correlation".into(),
            field: Some("a".into()),
            threshold: Some(0.3),
            ..Default::default()
        };
        let with_b1 = ScanTarget {
            field_b: Some("b1".into()),
            ..base.clone()
        };
        let with_b2 = ScanTarget {
            field_b: Some("b2".into()),
            ..base
        };
        assert_ne!(
            signal_id("alice", &with_b1),
            signal_id("alice", &with_b2),
            "D-023: different field_b must not collide"
        );
    }

    #[test]
    fn d023_different_companion_yields_different_id() {
        let mk = |comp_src: &str| ScanTarget {
            source: "hkma".into(),
            dataset: "x".into(),
            detector: "cross_source_gap".into(),
            field: Some("date".into()),
            companion: Some(hkgov_common::CompanionRef {
                source: comp_src.into(),
                dataset: "y".into(),
            }),
            ..Default::default()
        };
        assert_ne!(
            signal_id("alice", &mk("press")),
            signal_id("alice", &mk("datagovhk")),
            "D-023: different companion source must not collide"
        );
    }

    #[test]
    fn d023_identical_targets_still_dedup() {
        // Regression guard: adding fields to the id must not break the dedup
        // property for truly identical targets.
        let t = hibor_target("above", 2.5);
        assert_eq!(
            signal_id("alice", &t),
            signal_id("alice", &t),
            "identical targets must still share an id"
        );
    }
}
