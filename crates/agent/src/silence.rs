//! Silence Index (P-100) — government opacity, quantified.
//!
//! Productizes the project's thesis ("surfaces what the press room leaves
//! unsaid") into a single, public, citable number: *how much did HKGOV not
//! explain this period?*
//!
//! ## Determinism
//!
//! The index is a pure-Rust rollup of existing deterministic findings
//! (`cross_source_gap` + unattributed `series_jump` + missing-data days). No
//! LLM, no API key. Same findings in → same score out → CI-reproducible. The
//! determinism guarantee is the defense against "your opacity score is biased"
//! — critics can reproduce it themselves from the evidence.
//!
//! ## v1 scope (honest)
//!
//! Per the Phase-5 validation (D-5): v1 is explicitly **HKMA-scoped**, because
//! today ~4 of 5 ingested datasets are HKMA and `cross_source_gap` fires richly
//! only there. The methodology is source-pluggable: when data.gov.hk expansion
//! lands, the same code widens to other sources without a methodology bump. The
//! surfaced label is "HKMA Silence Index v1".
//!
//! ## Score construction
//!
//! The score is `0–100` (higher = more opaque) over a period (default = the
//! most-recent complete quarter of available findings). Inputs:
//!
//! | signal | weight | rationale |
//! |---|---|---|
//! | `cross_source_gap` press-only dates (press w/ no data row) | 3 each | the literal "narrative exists, data doesn't" |
//! | `cross_source_gap` data-only dates (data w/ no press) | 1 each | softer — routine data days need no press |
//! | unattributed `series_jump` (big move, no same-day press) | 5 each | a move this big *should* have been explained |
//! | missing-data days (expected cadence, no row) | 2 each | data the press room never published |
//!
//! The raw weighted sum is squashed through `100 · (1 − 1/(1 + raw/40))` so it
//! asymptotes to 100: doubling opacity doesn't double the score, matching how
//! people read a 0–100 gauge. The `40` (half-saturation) is the methodology
//! constant — change it in one place, bump the version.

use crate::insight::{Insight, InsightStore};
use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Methodology version. Bump whenever the weights, the squash constant, or the
/// signal set change — so a v1.x score is never silently compared to a v1.y.
pub const METHODOLOGY_VERSION: &str = "1.0";

/// The source this index currently covers. Per Phase-5 D-5, v1 is HKMA-scoped.
pub const COVERED_SOURCE: DataSource = DataSource::Hkma;

/// Half-saturation constant for the score squash. Lower = more sensitive at the
/// low end; 40 means ~40 weighted signals yields a score of 50.
const HALF_SATURATION: f64 = 40.0;

/// Per-signal weights. Centralized so a methodology bump is a one-line change.
pub mod weights {
    /// A press release with no matching data row — the headline signal.
    pub const PRESS_ONLY_GAP: f64 = 3.0;
    /// A data row with no matching press release — softer (routine data days).
    pub const DATA_ONLY_GAP: f64 = 1.0;
    /// A `series_jump` with no same-day attributing press release.
    pub const UNATTRIBUTED_JUMP: f64 = 5.0;
    /// A missing-data day (expected cadence, no row published).
    pub const MISSING_DATA_DAY: f64 = 2.0;
}

/// One structured signal feeding the index, with its evidence pointers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceSignal {
    pub kind: SilenceSignalKind,
    /// How many times this signal fired in the period.
    pub count: usize,
    /// The weight applied (kept in the struct so the breakdown is auditable).
    pub weight: f64,
    /// The contribution to the raw score (`count × weight`).
    pub contribution: f64,
    /// Evidence: the source insight ids (or missing-date strings) backing this.
    pub evidence_ids: Vec<String>,
}

/// The kind of opacity signal. Serializable for the breakdown table.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SilenceSignalKind {
    /// Press release issued but no matching data row.
    PressOnlyGap,
    /// Data row published but no matching press release.
    DataOnlyGap,
    /// A big series move with no same-day attributing press release.
    UnattributedJump,
    /// A day the data was expected but never published.
    MissingDataDay,
}

impl SilenceSignalKind {
    fn weight(self) -> f64 {
        match self {
            SilenceSignalKind::PressOnlyGap => weights::PRESS_ONLY_GAP,
            SilenceSignalKind::DataOnlyGap => weights::DATA_ONLY_GAP,
            SilenceSignalKind::UnattributedJump => weights::UNATTRIBUTED_JUMP,
            SilenceSignalKind::MissingDataDay => weights::MISSING_DATA_DAY,
        }
    }
}

/// The full Silence Index read for one period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceIndex {
    /// "HKMA Silence Index" — the label the UI renders.
    pub label: String,
    /// Methodology version, e.g. "1.0".
    pub methodology_version: &'static str,
    /// The source this index covers (v1 = HKMA).
    pub source: DataSource,
    /// The period key, e.g. "2026-Q2".
    pub period: String,
    /// The 0–100 score (higher = more opaque).
    pub score: f64,
    /// The raw weighted sum before the squash (auditable).
    pub raw_score: f64,
    /// When the score was computed.
    pub computed_at: DateTime<Utc>,
    /// The structured breakdown feeding the score.
    pub signals: Vec<SilenceSignal>,
    /// Total number of opacity events in the period (sum of `count`s).
    pub total_events: usize,
}

impl SilenceIndex {
    /// The delta vs a prior period's score (positive = more opaque this period).
    pub fn delta(&self, prior: &SilenceIndex) -> f64 {
        self.score - prior.score
    }
}

/// Build the Silence Index for a period from the currently-held insights.
///
/// The period is a string key (e.g. "2026-Q2"); insights whose `record_id`
/// evidence falls within the period are counted. `now` is injected for
/// deterministic timestamps.
///
/// This is the pure, testable core — it does no IO beyond reading the insight
/// store. The HTTP route is a thin wrapper.
pub async fn build_index(
    store: &Arc<InsightStore>,
    period: &str,
    now: DateTime<Utc>,
) -> SilenceIndex {
    let insights = store.list(500).await;
    build_index_from_insights(&insights, period, now)
}

/// Pure core: build the index from an in-memory slice of insights. This is
/// what the tests exercise directly (no async, no store).
pub fn build_index_from_insights(
    insights: &[Insight],
    period: &str,
    now: DateTime<Utc>,
) -> SilenceIndex {
    // Partition insights in-period by kind.
    let mut press_only: Vec<&Insight> = Vec::new();
    let mut data_only: Vec<&Insight> = Vec::new();
    let mut unattributed_jumps: Vec<&Insight> = Vec::new();

    for i in insights {
        if i.source != COVERED_SOURCE {
            continue;
        }
        if !insight_in_period(i, period) {
            continue;
        }
        match i.kind.as_str() {
            "cross_source_gap" => {
                // Distinguish press-only vs data-only by the evidence context.
                if evidence_says_press_only(&i.evidence) {
                    press_only.push(i);
                } else {
                    data_only.push(i);
                }
            }
            "series_jump" if !has_same_day_press(i, insights) => {
                unattributed_jumps.push(i);
            }
            _ => {}
        }
    }

    // Missing-data days are derived from cross_source_gap evidence that names
    // missing dates; approximate via the data-only gap count for v1 (each
    // data-only gap finding represents a cluster of missing days, capped).
    let missing_days = missing_data_day_count(&data_only);

    let signals = vec![
        make_signal(
            SilenceSignalKind::PressOnlyGap,
            press_only.len(),
            |i| i.id.clone(),
            &press_only,
        ),
        make_signal(
            SilenceSignalKind::DataOnlyGap,
            data_only.len(),
            |i| i.id.clone(),
            &data_only,
        ),
        make_signal(
            SilenceSignalKind::UnattributedJump,
            unattributed_jumps.len(),
            |i| i.id.clone(),
            &unattributed_jumps,
        ),
        make_signal(
            SilenceSignalKind::MissingDataDay,
            missing_days,
            |_i: &Insight| String::new(),
            &[],
        ),
    ];

    let raw_score: f64 = signals.iter().map(|s| s.contribution).sum();
    let score = squash(raw_score);
    let total_events: usize = signals.iter().map(|s| s.count).sum();

    SilenceIndex {
        label: format!("{} Silence Index", COVERED_SOURCE.as_str().to_uppercase()),
        methodology_version: METHODOLOGY_VERSION,
        source: COVERED_SOURCE,
        period: period.into(),
        score,
        raw_score,
        computed_at: now,
        signals,
        total_events,
    }
}

/// Build a signal row from a count + the backing insights.
fn make_signal<F>(
    kind: SilenceSignalKind,
    count: usize,
    id_fn: F,
    backing: &[&Insight],
) -> SilenceSignal
where
    F: Fn(&Insight) -> String,
{
    let weight = kind.weight();
    let contribution = count as f64 * weight;
    let evidence_ids = backing
        .iter()
        .map(|i| id_fn(i))
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

/// Squash a raw weighted sum to a 0–100 score. `100 · (1 − 1/(1 + raw/k))`
/// asymptotes to 100; `raw == k` yields exactly 50.
fn squash(raw: f64) -> f64 {
    if raw <= 0.0 {
        return 0.0;
    }
    let s = 100.0 * (1.0 - 1.0 / (1.0 + raw / HALF_SATURATION));
    s.clamp(0.0, 100.0)
}

/// Does this insight's evidence fall in the given period? Uses the record_ids
/// (dates) in the evidence. For a quarterly period "2026-Q2", matches any
/// record_id starting with "2026-04".."2026-06" or containing the quarter tag.
fn insight_in_period(insight: &Insight, period: &str) -> bool {
    if period.is_empty() {
        return true; // "" = all periods (used for whole-history views).
    }
    // Parse "YYYY-Qn" → month range; fall back to a prefix match on other keys.
    if let Some((year, q)) = parse_quarter(period) {
        let (start_month, end_month) = quarter_month_range(q);
        return insight
            .evidence
            .iter()
            .any(|e| date_in_quarter(&e.record_id, year, start_month, end_month));
    }
    // Non-quarter period key: substring match on record_ids.
    insight
        .evidence
        .iter()
        .any(|e| e.record_id.starts_with(period))
}

/// Parse "YYYY-Qn" into (year, quarter 1..=4).
fn parse_quarter(s: &str) -> Option<(i32, u8)> {
    let (y, q) = s.split_once('-')?;
    let year: i32 = y.parse().ok()?;
    let q = q.strip_prefix('Q')?.parse::<u8>().ok()?;
    if (1..=4).contains(&q) {
        Some((year, q))
    } else {
        None
    }
}

/// Month range (inclusive, 1-indexed start, exclusive end) for a quarter.
fn quarter_month_range(q: u8) -> (u8, u8) {
    let start = (q - 1) * 3 + 1;
    (start, start + 3)
}

/// Does a date-ish record_id (e.g. "2026-05-18" or "2026-05") fall in the month
/// range? Matches on a YYYY-MM prefix.
fn date_in_quarter(record_id: &str, year: i32, start_month: u8, end_month: u8) -> bool {
    // Expect a leading "YYYY-MM".
    if record_id.len() < 7 {
        return false;
    }
    let y_str = &record_id[..4];
    let m_str = &record_id[5..7];
    let Ok(y) = y_str.parse::<i32>() else {
        return false;
    };
    let Ok(m) = m_str.parse::<u8>() else {
        return false;
    };
    y == year && m >= start_month && m < end_month
}

/// Cross-source-gap evidence conventionally carries a "press release date
/// without matching data" context. Treat that as press-only; anything else as
/// data-only.
fn evidence_says_press_only(evidence: &[crate::insight::EvidenceRef]) -> bool {
    evidence
        .iter()
        .any(|e| e.context.as_deref().unwrap_or("").contains("press"))
}

/// Did the same source issue a press release on the same day as a series_jump's
/// current period? If so, the jump is *attributed* and shouldn't count toward
/// opacity. Heuristic for v1: any cross_source_gap insight in-period whose
/// evidence contains the jump's current record_id counts as "press existed".
fn has_same_day_press(jump: &Insight, all: &[Insight]) -> bool {
    let jump_dates: Vec<&str> = jump
        .evidence
        .iter()
        .filter(|e| e.context.as_deref().unwrap_or("").contains("current"))
        .map(|e| e.record_id.as_str())
        .collect();
    if jump_dates.is_empty() {
        return false;
    }
    all.iter().any(|other| {
        other.kind == "cross_source_gap"
            && other
                .evidence
                .iter()
                .any(|e| jump_dates.contains(&e.record_id.as_str()))
    })
}

/// Approximate missing-data days from data-only gap findings for v1. Each
/// data-only gap finding represents a cluster; cap the contribution so a single
/// noisy finding can't dominate.
fn missing_data_day_count(data_only: &[&Insight]) -> usize {
    // Each data-only gap finding represents up to N missing press-coverage days.
    // Sum the evidence counts, capped at 30 per finding to avoid a runaway.
    data_only.iter().map(|i| i.evidence.len().min(30)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{EvidenceRef, InsightSeverity};
    use chrono::Utc;
    use hkgov_common::DataSource;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn gap_insight(id: &str, dates: &[&str], press_only: bool) -> Insight {
        let ctx = if press_only {
            "press release date without matching data"
        } else {
            "data row without matching press"
        };
        Insight {
            id: id.into(),
            kind: "cross_source_gap".into(),
            severity: InsightSeverity::Info,
            title: "gap".into(),
            summary: "s".into(),
            source: DataSource::Hkma,
            dataset: "x".into(),
            evidence: dates
                .iter()
                .map(|d| EvidenceRef {
                    record_id: d.to_string(),
                    field: "date".into(),
                    value: json!(d),
                    context: Some(ctx.into()),
                })
                .collect(),
            confidence: 0.6,
            generated_at: Utc::now(),
            producer: "test".into(),
            experimental: false,
        }
    }

    fn jump_insight(id: &str, prev_date: &str, curr_date: &str, attributed: bool) -> Insight {
        // If attributed, we ALSO create a same-day cross_source_gap that covers
        // curr_date — but that's done by the caller via the insights slice.
        // Here we just build the jump with the standard evidence context.
        let _ = attributed;
        Insight {
            id: id.into(),
            kind: "series_jump".into(),
            severity: InsightSeverity::Warning,
            title: "jump".into(),
            summary: "s".into(),
            source: DataSource::Hkma,
            dataset: "x".into(),
            evidence: vec![
                EvidenceRef {
                    record_id: prev_date.into(),
                    field: "rate".into(),
                    value: json!(1.0),
                    context: Some("previous period".into()),
                },
                EvidenceRef {
                    record_id: curr_date.into(),
                    field: "rate".into(),
                    value: json!(2.0),
                    context: Some("current period".into()),
                },
            ],
            confidence: 0.7,
            generated_at: Utc::now(),
            producer: "test".into(),
            experimental: false,
        }
    }

    #[test]
    fn empty_insights_yield_zero_score() {
        let idx = build_index_from_insights(&[], "2026-Q2", Utc::now());
        assert_eq!(idx.score, 0.0);
        assert_eq!(idx.total_events, 0);
        assert_eq!(idx.methodology_version, "1.0");
        assert!(idx.label.contains("HKMA"));
    }

    #[test]
    fn press_only_gaps_drive_score_up() {
        // 5 press-only gaps in 2026-Q2.
        let ins = vec![gap_insight("g1", &["2026-05-10"], true)];
        let idx = build_index_from_insights(&ins, "2026-Q2", Utc::now());
        let press_signal = idx
            .signals
            .iter()
            .find(|s| s.kind == SilenceSignalKind::PressOnlyGap)
            .unwrap();
        assert_eq!(press_signal.count, 1);
        assert_eq!(idx.raw_score, weights::PRESS_ONLY_GAP);
        assert!(idx.score > 0.0);
    }

    #[test]
    fn unattributed_jump_counts_toward_opacity() {
        // A series_jump in-period with no same-day press → counts.
        let ins = vec![jump_insight("j1", "2026-04-01", "2026-04-15", false)];
        let idx = build_index_from_insights(&ins, "2026-Q2", Utc::now());
        let jump_signal = idx
            .signals
            .iter()
            .find(|s| s.kind == SilenceSignalKind::UnattributedJump)
            .unwrap();
        assert_eq!(jump_signal.count, 1);
    }

    #[test]
    fn attributed_jump_excluded_from_opacity() {
        // A series_jump whose current date ALSO appears in a cross_source_gap
        // (i.e. press existed that day) → not unattributed → excluded.
        let jump = jump_insight("j1", "2026-04-01", "2026-04-15", true);
        let gap = gap_insight("g1", &["2026-04-15"], true);
        let ins = vec![jump, gap];
        let idx = build_index_from_insights(&ins, "2026-Q2", Utc::now());
        let jump_signal = idx
            .signals
            .iter()
            .find(|s| s.kind == SilenceSignalKind::UnattributedJump)
            .unwrap();
        assert_eq!(jump_signal.count, 0, "attributed jump should not count");
    }

    #[test]
    fn non_hkma_insights_excluded() {
        // A press-source gap should NOT count toward the HKMA index.
        let mut ins = gap_insight("g1", &["2026-05-10"], true);
        ins.source = DataSource::Press;
        let idx = build_index_from_insights(&[ins], "2026-Q2", Utc::now());
        assert_eq!(idx.score, 0.0);
    }

    #[test]
    fn out_of_period_insights_excluded() {
        // A gap in 2026-Q1 should not count for the 2026-Q2 index.
        let ins = vec![gap_insight("g1", &["2026-02-10"], true)];
        let idx = build_index_from_insights(&ins, "2026-Q2", Utc::now());
        assert_eq!(idx.score, 0.0);
    }

    #[test]
    fn squash_is_monotonic_and_bounded() {
        assert_eq!(squash(0.0), 0.0);
        let s1 = squash(10.0);
        let s2 = squash(20.0);
        let s3 = squash(HALF_SATURATION);
        assert!(s1 < s2);
        // At the half-saturation point, score should be ~50.
        assert!((s3 - 50.0).abs() < 0.01, "expected ~50 at k, got {s3}");
        // Asymptote to 100.
        assert!(squash(1_000_000.0) < 100.0);
        assert!(squash(1_000_000.0) > 99.0);
    }

    #[test]
    fn delta_is_score_difference() {
        let a = SilenceIndex {
            label: "x".into(),
            methodology_version: METHODOLOGY_VERSION,
            source: COVERED_SOURCE,
            period: "2026-Q1".into(),
            score: 40.0,
            raw_score: 20.0,
            computed_at: Utc::now(),
            signals: vec![],
            total_events: 0,
        };
        let b = SilenceIndex {
            score: 52.0,
            period: "2026-Q2".into(),
            ..a.clone_for_test()
        };
        assert_eq!(b.delta(&a), 12.0);
    }

    #[test]
    fn determinism_same_inputs_same_output() {
        let ins = vec![
            gap_insight("g1", &["2026-05-10"], true),
            gap_insight("g2", &["2026-05-11"], true),
            jump_insight("j1", "2026-04-01", "2026-04-15", false),
        ];
        let now = Utc::now();
        let a = build_index_from_insights(&ins, "2026-Q2", now);
        let b = build_index_from_insights(&ins, "2026-Q2", now);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn parse_quarter_valid_and_invalid() {
        assert_eq!(parse_quarter("2026-Q2"), Some((2026, 2)));
        assert_eq!(parse_quarter("2026-Q5"), None);
        assert_eq!(parse_quarter("nope"), None);
    }

    #[test]
    fn date_in_quarter_matches() {
        assert!(date_in_quarter("2026-05-18", 2026, 4, 7));
        assert!(date_in_quarter("2026-04", 2026, 4, 7));
        assert!(!date_in_quarter("2026-03-31", 2026, 4, 7));
        assert!(!date_in_quarter("2026-07-01", 2026, 4, 7));
    }

    // ---- test helper: clone a SilenceIndex cheaply for the delta test ----
    impl SilenceIndex {
        fn clone_for_test(&self) -> Self {
            SilenceIndex {
                label: self.label.clone(),
                methodology_version: self.methodology_version,
                source: self.source,
                period: self.period.clone(),
                score: self.score,
                raw_score: self.raw_score,
                computed_at: self.computed_at,
                signals: self.signals.clone(),
                total_events: self.total_events,
            }
        }
    }

    // Quiet the BTreeMap import lint in case of feature-gated cfg.
    #[test]
    fn _btreemap_unused_guard() {
        let _m: BTreeMap<&str, i32> = BTreeMap::new();
    }
}
