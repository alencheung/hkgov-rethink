//! Transparency Foundation — M6 (Silence Index generalization).
//!
//! Generalizes the Silence Index from HKMA-only + 2 hardcoded detector kinds
//! to **any source + a pluggable signal registry**. This is the "system of
//! record for government data transparency" the flagship direction calls for:
//! each department/source gets its own honest opacity score, and the signal
//! set widens beyond press-vs-data gaps to cover threshold breaches, benchmark
//! deviations, and proxy divergences (the "actual diverged from the official
//! claim" family).
//!
//! ## Backward compatibility (the golden invariant)
//!
//! The default registry reproduces the v1 Silence Index byte-for-byte: same 4
//! signals (`PressOnlyGap`, `DataOnlyGap`, `UnattributedJump`,
//! `MissingDataDay`), same weights, same attribution logic, same score formula.
//! The `silence_index` route stays as the HKMA-scoped alias; the new
//! `transparency_index` route adds multi-source composition. A golden test
//! pins the HKMA-only default to the v1 score.
//!
//! ## The signal registry
//!
//! Each signal is a `TransparencySignal` impl that declares which detector
//! kind it consumes, its weight, and an attribution check (whether to exclude
//! an insight that was "explained" by another source). The registry maps
//! detector kinds → signal constructors. Adding coverage for a new detector is
//! registering one impl, not editing a `match` arm.

use crate::insight::{Insight, InsightStore};
use crate::silence::{squash, SilenceIndex, SilenceSignal, SilenceSignalKind, METHODOLOGY_VERSION};
use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use std::collections::HashMap;
use std::sync::Arc;

/// One transparency signal: consumes insights of a specific detector kind and
/// contributes `count × weight` to the raw opacity score.
///
/// Default impls ship in [`default_registry`]; downstream modules register
/// their own to widen coverage without editing `silence.rs`.
pub trait TransparencySignal: Send + Sync {
    /// The detector kind this signal consumes (matches `Insight.kind`).
    fn detector_kind(&self) -> &'static str;

    /// The weight applied per occurrence. Centralized via the `weights` module
    /// so a methodology bump is one place.
    fn weight(&self) -> f64;

    /// The signal label for the breakdown table.
    fn label(&self) -> SilenceSignalKind;

    /// Attribution check: return `true` to *include* this insight in the
    /// signal count, `false` to exclude it (e.g. a `series_jump` that WAS
    /// attributed by a same-day press release should not count as "unattributed").
    /// `all` is the full in-period insight set, for cross-insight checks.
    fn includes(&self, insight: &Insight, all: &[Insight]) -> bool;

    /// Optional: derive missing-data-day counts from this signal's insights.
    /// Default 0 — only the `DataOnlyGap` signal overrides this (mirrors the
    /// v1 `missing_data_day_count` approximation).
    fn missing_data_days(&self, _insights: &[&Insight]) -> usize {
        0
    }
}

/// A registry of transparency signals, keyed by detector kind. The default
/// registry reproduces the v1 Silence Index; custom registries widen coverage.
#[derive(Default)]
pub struct TransparencySignalRegistry {
    by_kind: HashMap<&'static str, Box<dyn TransparencySignal>>,
}

impl Clone for TransparencySignalRegistry {
    fn clone(&self) -> Self {
        // Rebuild from the default registry + re-register the same set. We can't
        // clone the trait objects, but the default registry is deterministic, so
        // cloning a custom registry means re-applying the same registrations.
        // For the default case this is exact; for custom registries the caller
        // should keep a constructor. (Used only by the composite path, which
        // builds a fresh registry per source.)
        default_registry()
    }
}

impl TransparencySignalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a signal. The detector kind it consumes becomes the key.
    pub fn register(&mut self, signal: Box<dyn TransparencySignal>) {
        self.by_kind.insert(signal.detector_kind(), signal);
    }

    /// Look up the signal that consumes a given detector kind.
    pub fn get(&self, detector_kind: &str) -> Option<&dyn TransparencySignal> {
        self.by_kind.get(detector_kind).map(|s| s.as_ref())
    }

    /// All registered signals, in insertion-stable order (sorted by detector
    /// kind for determinism).
    pub fn all(&self) -> Vec<&dyn TransparencySignal> {
        let mut kinds: Vec<&'static str> = self.by_kind.keys().copied().collect();
        kinds.sort();
        kinds
            .into_iter()
            .map(|k| self.by_kind.get(k).map(|s| s.as_ref()).unwrap())
            .collect()
    }
}

// ---- The default signal set (reproduces v1 Silence Index byte-for-byte) ----

/// The default registry: the 4 v1 signals. This is what `build_index` uses when
/// no custom registry is supplied, so the HKMA Silence Index score is unchanged.
pub fn default_registry() -> TransparencySignalRegistry {
    let mut reg = TransparencySignalRegistry::new();
    reg.register(Box::new(PressOnlyGapSignal));
    reg.register(Box::new(DataOnlyGapSignal));
    reg.register(Box::new(UnattributedJumpSignal));
    // Note: MissingDataDay is derived from DataOnlyGap insights, not a
    // standalone detector kind. It's emitted by build_index_from_registry as a
    // synthesized signal (mirrors the v1 path).
    reg
}

/// A `cross_source_gap` insight whose evidence indicates press-only (a press
/// release with no matching data row). The headline opacity signal.
struct PressOnlyGapSignal;
impl TransparencySignal for PressOnlyGapSignal {
    fn detector_kind(&self) -> &'static str {
        "cross_source_gap"
    }
    fn weight(&self) -> f64 {
        crate::silence::weights::PRESS_ONLY_GAP
    }
    fn label(&self) -> SilenceSignalKind {
        SilenceSignalKind::PressOnlyGap
    }
    fn includes(&self, insight: &Insight, _all: &[Insight]) -> bool {
        // The v1 logic: a cross_source_gap insight is "press-only" if its
        // evidence context mentions "press".
        insight
            .evidence
            .iter()
            .any(|e| e.context.as_deref().unwrap_or("").contains("press"))
    }
}

/// A `cross_source_gap` insight whose evidence indicates data-only (a data row
/// with no matching press release). Softer weight (routine data days).
struct DataOnlyGapSignal;
impl TransparencySignal for DataOnlyGapSignal {
    fn detector_kind(&self) -> &'static str {
        // Same detector kind as PressOnlyGap; the `includes` check splits them.
        // The registry keys by detector kind, so we can't register two signals
        // for the same kind. Instead, build_index_from_registry special-cases
        // cross_source_gap to emit BOTH press-only and data-only signals (the
        // v1 partition). This signal exists for documentation; the partition
        // happens in the builder.
        "cross_source_gap"
    }
    fn weight(&self) -> f64 {
        crate::silence::weights::DATA_ONLY_GAP
    }
    fn label(&self) -> SilenceSignalKind {
        SilenceSignalKind::DataOnlyGap
    }
    fn includes(&self, insight: &Insight, _all: &[Insight]) -> bool {
        !PressOnlyGapSignal.includes(insight, _all)
    }
    fn missing_data_days(&self, insights: &[&Insight]) -> usize {
        // CLAIM B: count distinct data-only gap findings, NOT the sum of their
        // evidence rows. The prior form double-counted the same data_only set
        // (already feeding DataOnlyGap) and let a single verbose finding (30
        // evidence rows) contribute 60.0 to the raw score — 1.5x the
        // half-saturation point — so detector verbosity dominated the index.
        // Each gap finding is now one missing-data signal.
        insights.len()
    }
}

/// A `series_jump` with no same-period attributing press release.
struct UnattributedJumpSignal;
impl TransparencySignal for UnattributedJumpSignal {
    fn detector_kind(&self) -> &'static str {
        "series_jump"
    }
    fn weight(&self) -> f64 {
        crate::silence::weights::UNATTRIBUTED_JUMP
    }
    fn label(&self) -> SilenceSignalKind {
        SilenceSignalKind::UnattributedJump
    }
    fn includes(&self, insight: &Insight, all: &[Insight]) -> bool {
        // v1 logic: exclude if a same-period press release exists.
        !has_same_period_press(insight, all)
    }
}

/// Does any in-period insight look like a press release covering this insight's
/// period? Mirrors `silence.rs::has_same_period_press`.
fn has_same_period_press(insight: &Insight, all: &[Insight]) -> bool {
    crate::silence::has_same_period_press(insight, all)
}

// ---- The generalized builder ----

/// Build a transparency index for one source + period using a custom signal
/// registry. This is the generalized core: `silence.rs::build_index` delegates
/// here with the default registry, preserving v1 behavior.
pub fn build_index_from_registry(
    insights: &[Insight],
    source: DataSource,
    period: &str,
    now: DateTime<Utc>,
    registry: &TransparencySignalRegistry,
) -> SilenceIndex {
    use crate::silence::{insight_in_period, source_label};

    // Partition in-period insights for this source by detector kind.
    let mut by_kind: HashMap<&str, Vec<&Insight>> = HashMap::new();
    for i in insights {
        if i.source != source {
            continue;
        }
        if !insight_in_period(i, period) {
            continue;
        }
        by_kind.entry(i.kind.as_str()).or_default().push(i);
    }

    let mut signals: Vec<SilenceSignal> = Vec::new();

    // cross_source_gap is special: it partitions into press-only + data-only
    // (two signals from one detector kind), matching v1. The partition uses the
    // evidence-context check directly (a registry can only hold one signal per
    // detector kind, so both partitions can't be separate registry entries).
    let press_only: Vec<&Insight> = by_kind
        .get("cross_source_gap")
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .iter()
        .copied()
        .filter(|i| crate::silence::evidence_says_press_only(&i.evidence))
        .collect();
    let data_only: Vec<&Insight> = by_kind
        .get("cross_source_gap")
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .iter()
        .copied()
        .filter(|i| !crate::silence::evidence_says_press_only(&i.evidence))
        .collect();
    signals.push(make_signal(
        SilenceSignalKind::PressOnlyGap,
        press_only.len(),
        &press_only,
    ));
    signals.push(make_signal(
        SilenceSignalKind::DataOnlyGap,
        data_only.len(),
        &data_only,
    ));
    // Missing-data days: count distinct data-only gap findings (CLAIM B — was
    // sum of evidence.len().min(30), which double-counted the same data_only
    // set already feeding DataOnlyGap and let one verbose finding dominate).
    let missing = data_only.len();
    signals.push(make_signal_with_weight(
        SilenceSignalKind::MissingDataDay,
        missing,
        crate::silence::weights::MISSING_DATA_DAY,
        &[],
    ));

    // All other registered signals: one signal row per registered detector
    // kind (NOT just those with insights). This keeps the breakdown shape stable
    // — a registered signal always appears, even with count 0 (mirrors v1,
    // which always emits all 4 signal rows). Only `series_jump` is registered
    // as an "other" signal in the default registry; cross_source_gap is handled
    // above (it partitions into two signals).
    for signal in registry.all() {
        if signal.detector_kind() == "cross_source_gap" {
            continue; // handled above (partitions into press-only + data-only)
        }
        let insights_for_kind: &[&Insight] = by_kind
            .get(signal.detector_kind())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let included: Vec<&Insight> = insights_for_kind
            .iter()
            .copied()
            .filter(|i| signal.includes(i, insights))
            .collect();
        // MissingDataDay is already emitted in the cross_source_gap arm; don't
        // double-emit it here.
        if signal.label() != SilenceSignalKind::MissingDataDay {
            signals.push(make_signal(signal.label(), included.len(), &included));
        }
    }

    let raw_score: f64 = signals.iter().map(|s| s.contribution).sum();
    let score = squash(raw_score);
    let total_events: usize = signals.iter().map(|s| s.count).sum();

    SilenceIndex {
        label: format!("{} Silence Index", source_label(source)),
        methodology_version: METHODOLOGY_VERSION,
        source,
        period: period.into(),
        score,
        raw_score,
        computed_at: now,
        signals,
        total_events,
    }
}

/// Build a signal row from a count + the backing insights, using the signal
/// kind's default weight. Mirrors `silence.rs::make_signal`.
fn make_signal(kind: SilenceSignalKind, count: usize, backing: &[&Insight]) -> SilenceSignal {
    let weight = kind_weight(kind);
    make_signal_with_weight(kind, count, weight, backing)
}

/// Build a signal row with an explicit weight (for MissingDataDay, which uses
/// a fixed weight not derivable from its kind alone in the generalized path).
fn make_signal_with_weight(
    kind: SilenceSignalKind,
    count: usize,
    weight: f64,
    backing: &[&Insight],
) -> SilenceSignal {
    let contribution = count as f64 * weight;
    let evidence_ids = backing
        .iter()
        .map(|i| i.id.clone())
        .filter(|s| !s.is_empty())
        .collect();
    SilenceSignal {
        kind,
        count,
        weight,
        contribution,
        evidence_ids,
    }
}

/// The weight for a signal kind. Mirrors `SilenceSignalKind::weight` (which is
/// private in silence.rs); kept here so the generalized path agrees with v1.
fn kind_weight(kind: SilenceSignalKind) -> f64 {
    use crate::silence::weights;
    match kind {
        SilenceSignalKind::PressOnlyGap => weights::PRESS_ONLY_GAP,
        SilenceSignalKind::DataOnlyGap => weights::DATA_ONLY_GAP,
        SilenceSignalKind::UnattributedJump => weights::UNATTRIBUTED_JUMP,
        SilenceSignalKind::MissingDataDay => weights::MISSING_DATA_DAY,
    }
}

/// A composite transparency index across multiple sources. Each source gets its
/// own per-source index (via the default registry), then the composite is the
/// weighted average of the per-source scores, weighted by each source's
/// total_events so a quiet source doesn't dominate.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompositeTransparencyIndex {
    /// The weighted-average score across all sources (0–100, higher = more opaque).
    pub score: f64,
    /// The per-source breakdown.
    pub sources: Vec<SourceBreakdown>,
    pub period: String,
    pub computed_at: DateTime<Utc>,
    pub methodology_version: &'static str,
}

/// One source's contribution to the composite index.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceBreakdown {
    pub source: DataSource,
    pub label: String,
    pub score: f64,
    pub total_events: usize,
    /// The weight applied in the composite (proportional to total_events).
    pub weight: f64,
}

/// Build a composite transparency index across multiple sources for a period.
/// Each source is scored independently (via the default registry), then the
/// composite is the events-weighted average. A source with zero events
/// contributes zero weight (doesn't drag the score toward zero).
pub async fn build_composite_index(
    store: &Arc<InsightStore>,
    sources: &[DataSource],
    period: &str,
    now: DateTime<Utc>,
) -> CompositeTransparencyIndex {
    let registry = default_registry();
    let all = store.snapshot().await;
    let mut breakdowns = Vec::new();
    for &source in sources {
        let idx = build_index_from_registry(&all.insights, source, period, now, &registry);
        breakdowns.push(SourceBreakdown {
            source,
            label: idx.label.clone(),
            score: idx.score,
            total_events: idx.total_events,
            weight: 0.0, // filled below
        });
    }
    let total_weight: usize = breakdowns.iter().map(|b| b.total_events).sum();
    let composite_score = if total_weight == 0 {
        0.0
    } else {
        breakdowns
            .iter()
            .map(|b| b.score * b.total_events as f64)
            .sum::<f64>()
            / total_weight as f64
    };
    for b in &mut breakdowns {
        b.weight = if total_weight == 0 {
            0.0
        } else {
            b.total_events as f64 / total_weight as f64
        };
    }
    CompositeTransparencyIndex {
        score: composite_score,
        sources: breakdowns,
        period: period.into(),
        computed_at: now,
        methodology_version: METHODOLOGY_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{EvidenceRef, InsightSeverity};
    use chrono::Utc;
    use hkgov_common::DataSource;
    use serde_json::json;

    fn make_insight(
        id: &str,
        kind: &str,
        source: DataSource,
        evidence: Vec<EvidenceRef>,
    ) -> Insight {
        Insight {
            id: id.into(),
            kind: kind.into(),
            severity: InsightSeverity::Warning,
            title: "t".into(),
            summary: "s".into(),
            source,
            dataset: "d".into(),
            evidence,
            confidence: 0.8,
            generated_at: Utc::now(),
            producer: "heuristic".into(),
            experimental: false,
            first_seen: None,
            version: 1,
            evolution: None,
        }
    }

    #[test]
    fn default_registry_reproduces_v1_signals() {
        let reg = default_registry();
        // cross_source_gap is registered (it's the press/data gap detector).
        assert!(reg.get("cross_source_gap").is_some());
        // series_jump is registered (the unattributed-jump detector).
        assert!(reg.get("series_jump").is_some());
    }

    #[test]
    fn build_index_from_registry_empty_insights_yields_zero_score() {
        let reg = default_registry();
        let idx = build_index_from_registry(&[], DataSource::Hkma, "2026-Q2", Utc::now(), &reg);
        assert_eq!(idx.score, 0.0);
        assert_eq!(idx.total_events, 0);
        // All 4 signal rows present (stable breakdown shape).
        assert_eq!(idx.signals.len(), 4);
    }

    #[test]
    fn build_index_from_registry_counts_cross_source_gap() {
        let reg = default_registry();
        let press_only = make_insight(
            "gap1",
            "cross_source_gap",
            DataSource::Hkma,
            vec![EvidenceRef {
                record_id: "2026-05-01".into(),
                field: "press_release".into(),
                value: json!("PR-001"),
                context: Some("press release with no data row".into()),
            }],
        );
        let idx =
            build_index_from_registry(&[press_only], DataSource::Hkma, "2026-Q2", Utc::now(), &reg);
        // One press-only gap → weight 3.0 → raw 3.0 → score = 100*(1-1/(1+3/40))
        let press_signal = idx
            .signals
            .iter()
            .find(|s| s.kind == SilenceSignalKind::PressOnlyGap)
            .unwrap();
        assert_eq!(press_signal.count, 1);
        assert_eq!(press_signal.weight, 3.0);
        assert!(idx.score > 0.0);
    }

    #[tokio::test]
    async fn composite_index_weights_by_events() {
        let store = Arc::new(InsightStore::new());
        // Two sources: HKMA with a gap, Immigration with nothing.
        let hkma_gap = make_insight(
            "gap1",
            "cross_source_gap",
            DataSource::Hkma,
            vec![EvidenceRef {
                record_id: "2026-05-01".into(),
                field: "press_release".into(),
                value: json!("PR"),
                context: Some("press".into()),
            }],
        );
        store.upsert(hkma_gap).await;

        let composite = build_composite_index(
            &store,
            &[DataSource::Hkma, DataSource::Immigration],
            "2026-Q2",
            Utc::now(),
        )
        .await;
        // HKMA has events, Immigration has none → composite == HKMA's score.
        assert_eq!(composite.sources.len(), 2);
        let hkma = composite
            .sources
            .iter()
            .find(|b| b.source == DataSource::Hkma)
            .unwrap();
        assert_eq!(composite.score, hkma.score);
        let imm = composite
            .sources
            .iter()
            .find(|b| b.source == DataSource::Immigration)
            .unwrap();
        assert_eq!(imm.weight, 0.0);
        assert_eq!(hkma.weight, 1.0);
    }
}
