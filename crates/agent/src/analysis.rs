//! Provider-independent analysis: turns raw records into structured `Finding`s.
//!
//! v3 detectors: [`detect_series_jumps`] (consecutive moves) and
//! [`detect_cross_source_gaps`] (press date vs data date).
//!
//! v6 added [`detect_outliers`], [`detect_seasonality`], [`detect_correlation`].
//!
//! v7 added cadence-aware detectors that account for non-daily update frequency:
//! - [`detect_series_jumps_cadenced`] — threshold scaled to the series' cadence.
//! - [`detect_year_over_year`] — same-period-last-year comparison (removes
//!   seasonality; the right choice for quarterly/annual series).
//! - [`detect_proxy_divergence`] — the hero detector for "non-obvious lies":
//!   flags when two proxies that *should* measure the same underlying fact
//!   diverge in value or decouple over time.
//! - [`detect_benchmark_deviation`] — actual vs. assumed/projected/peer value.
//!
//! Each detector emits `Finding`s; the LLM client (heuristic or HTTP) only
//! *frames* them into natural language. Detection stays deterministic.

use crate::insight::{EvidenceRef, Insight, InsightSeverity};
use chrono::Utc;
use hkgov_common::{Cadence, DataSource, NormalizedRecord, RecordValue};
use serde::{Deserialize, Serialize};

/// A raw, structured finding before an LLM frames it. Serializable so the HTTP
/// LLM client can ship it verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: String,
    pub source: DataSource,
    pub dataset: String,
    pub title: String,
    pub heuristic_summary: String,
    pub severity: String,
    pub confidence: f64,
    pub evidence: Vec<EvidenceRef>,
}

impl Finding {
    pub fn heuristic_summary(&self) -> String {
        self.heuristic_summary.clone()
    }

    pub fn severity_enum(&self) -> InsightSeverity {
        match self.severity.as_str() {
            "critical" => InsightSeverity::Critical,
            "warning" => InsightSeverity::Warning,
            _ => InsightSeverity::Info,
        }
    }

    /// Promote a framed finding into a stored Insight.
    pub fn into_insight(self, summary: String, confidence: f64, producer: &str) -> Insight {
        self.into_insight_experimental(summary, confidence, producer, false)
    }

    /// Like [`into_insight`] but carries the experimental flag through to the
    /// insight, so the UI can badge it. Use when the producing detector is
    /// marked experimental in `[[agent.scan]]`.
    pub fn into_insight_experimental(
        self,
        summary: String,
        confidence: f64,
        producer: &str,
        experimental: bool,
    ) -> Insight {
        let severity = self.severity_enum();
        // Include dataset in the id so two detectors scanning different datasets
        // can't collide (the v3 id omitted it, which was fine only while exactly
        // two datasets were ever scanned).
        let id = format!(
            "{}:{}:{}:{}",
            self.kind,
            self.source,
            self.dataset,
            fingerprint(&self.evidence)
        );
        Insight {
            id,
            kind: self.kind,
            severity,
            title: self.title,
            summary,
            source: self.source,
            dataset: self.dataset,
            evidence: self.evidence,
            confidence: confidence.clamp(0.0, 1.0),
            generated_at: Utc::now(),
            producer: producer.to_string(),
            experimental,
            // P-104 Lifeline: the store overrides these on upsert
            // (first_seen is set to now, version to 1, evolution to None).
            first_seen: None,
            version: 1,
            evolution: None,
        }
    }
}

fn fingerprint(ev: &[EvidenceRef]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for e in ev {
        e.record_id.hash(&mut h);
        e.field.hash(&mut h);
        e.value.to_string().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Detect large period-over-period moves in numeric series. Returns one finding
/// per (field) that jumped by more than `pct_threshold` percent.
///
/// Records are expected to be sorted descending by period; the comparator is
/// the field named by `series_field`.
pub fn detect_series_jumps(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    series_field: &str,
    pct_threshold: f64,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    // Sort ascending by period/date so consecutive pairs are chronological.
    let mut sorted: Vec<&NormalizedRecord> = records.iter().collect();
    sorted.sort_by_key(|r| r.record_id.clone());

    for window in sorted.windows(2) {
        let (prev, curr) = (window[0], window[1]);
        let Some(RecordValue::Float(curr_v)) = curr.fields.get(series_field) else {
            continue;
        };
        let Some(RecordValue::Float(prev_v)) = prev.fields.get(series_field) else {
            continue;
        };
        if prev_v.abs() < f64::EPSILON {
            continue;
        }
        let pct = ((curr_v - prev_v) / prev_v.abs()) * 100.0;
        if pct.abs() >= pct_threshold {
            let confidence = ((pct.abs() / pct_threshold).min(5.0) / 5.0).clamp(0.5, 1.0);
            let severity = if pct.abs() >= pct_threshold * 3.0 {
                "critical"
            } else {
                "warning"
            };
            findings.push(Finding {
                kind: "series_jump".into(),
                source,
                dataset: dataset.into(),
                title: format!(
                    "{series_field} moved {pct:+.1}% in {} ({} → {})",
                    curr.record_id, prev.record_id, curr.record_id
                ),
                heuristic_summary: format!(
                    "The {series_field} series changed by {pct:+.1}% between {prev} and {curr_id} \
                     ({prev_v:.2} → {curr_v:.2}). This exceeds the {pct_threshold:.0}% watch \
                     threshold.",
                    prev = prev.record_id,
                    curr_id = curr.record_id,
                ),
                severity: severity.into(),
                confidence,
                evidence: vec![
                    EvidenceRef {
                        record_id: prev.record_id.clone(),
                        field: series_field.into(),
                        value: serde_json::json!(*prev_v),
                        context: Some("previous period".into()),
                    },
                    EvidenceRef {
                        record_id: curr.record_id.clone(),
                        field: series_field.into(),
                        value: serde_json::json!(*curr_v),
                        context: Some("current period".into()),
                    },
                ],
            });
        }
    }
    findings
}

/// Detect dates where a press release exists but no matching data row does (or
/// the reverse). Surfaces "the narrative and the data don't line up" gaps.
///
/// `source`/`dataset` describe the press side (what the dates come from). The
/// default scan target points these at `press` / `hkma-press-releases`.
pub fn detect_cross_source_gaps(
    source: DataSource,
    dataset: &str,
    press_dates: &[String],
    data_dates: &[String],
) -> Vec<Finding> {
    let press_set: std::collections::HashSet<&String> = press_dates.iter().collect();
    let data_set: std::collections::HashSet<&String> = data_dates.iter().collect();
    let mut findings = Vec::new();

    // Press release with no data for that date.
    let press_only: Vec<&&String> = press_set
        .iter()
        .filter(|d| !data_set.contains(*d))
        .collect();
    if !press_only.is_empty() {
        findings.push(Finding {
            kind: "cross_source_gap".into(),
            source,
            dataset: dataset.into(),
            title: format!(
                "{} press date(s) with no matching {} data row",
                press_only.len(),
                source
            ),
            heuristic_summary: format!(
                "On {} date(s) a {} press release was issued but no corresponding \
                 statistical data row was published, or vice versa. These are candidate \
                 points where the official narrative and the published data diverge.",
                press_only.len(),
                source
            ),
            severity: "info".into(),
            confidence: 0.6,
            evidence: press_only
                .iter()
                .take(10)
                .map(|d| EvidenceRef {
                    record_id: d.to_string(),
                    field: "date".into(),
                    value: serde_json::json!(d.to_string()),
                    context: Some("press release date without matching data".into()),
                })
                .collect(),
        });
    }
    findings
}

// ---------------------------------------------------------------------------
// New deterministic detectors (v6 — richer intelligence).
//
// Each follows the same contract as the two above: a `pub fn` returning
// `Vec<Finding>` with deterministic, reproducible output and `EvidenceRef`s
// pointing back into the record store. They are deliberately dependency-free
// (no statistics crate) — the math is small enough to inline.
// ---------------------------------------------------------------------------

/// Default |z| above which a point is an outlier (`detect_outliers`).
pub const DEFAULT_OUTLIER_Z: f64 = 3.5;
/// Default autocorrelation strength above which seasonality is flagged
/// (`detect_seasonality`).
pub const DEFAULT_SEASONALITY_R: f64 = 0.6;
/// Default minimum record count before a detector will opine — fewer points
/// than this and the statistics are too noisy to be meaningful.
pub const MIN_SAMPLES: usize = 4;
/// Default |r| below which a correlation is flagged as a decoupling
/// (`detect_correlation`) — i.e. two series that *used* to move together no
/// longer do.
pub const DEFAULT_CORRELATION_R: f64 = 0.3;

/// Extract the numeric `series_field` value from each record, paired with its
/// record_id, sorted ascending by record_id. Shared by the series detectors.
fn numeric_series<'a>(records: &'a [NormalizedRecord], series_field: &str) -> Vec<(&'a str, f64)> {
    let mut out: Vec<(&'a str, f64)> = records
        .iter()
        .filter_map(|r| match r.fields.get(series_field)? {
            RecordValue::Float(v) => Some((r.record_id.as_str(), *v)),
            RecordValue::Int(v) => Some((r.record_id.as_str(), *v as f64)),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// Detect statistical outliers using the median absolute deviation (MAD), a
/// robust z-score that isn't skewed by the outliers themselves. Emits one
/// finding per point whose |robust-z| exceeds `z_threshold` (defaults to
/// [`DEFAULT_OUTLIER_Z`]).
pub fn detect_outliers(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    series_field: &str,
    z_threshold: f64,
) -> Vec<Finding> {
    let series = numeric_series(records, series_field);
    if series.len() < MIN_SAMPLES {
        return Vec::new();
    }
    let z_threshold = if z_threshold > 0.0 {
        z_threshold
    } else {
        DEFAULT_OUTLIER_Z
    };

    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
    let median_v = median(&values);
    let abs_devs: Vec<f64> = values.iter().map(|v| (v - median_v).abs()).collect();
    let mad = median(&abs_devs);

    // 0.6745 scales MAD to be comparable to a standard-deviation z-score.
    let scale = 0.6745 * mad;
    if scale < f64::EPSILON {
        // MAD == 0 means >half the points are identical; can't score outliers.
        return Vec::new();
    }

    let mut findings = Vec::new();
    for (id, v) in &series {
        let z = (v - median_v) / scale;
        if z.abs() >= z_threshold {
            let confidence = ((z.abs() / z_threshold).min(4.0) / 4.0).clamp(0.5, 1.0);
            let severity = if z.abs() >= z_threshold * 2.0 {
                "critical"
            } else {
                "warning"
            };
            findings.push(Finding {
                kind: "outlier".into(),
                source,
                dataset: dataset.into(),
                title: format!(
                    "{series_field} outlier at {id}: {v:.2} (robust-z {z:+.1}, median {median_v:.2})"
                ),
                heuristic_summary: format!(
                    "The {series_field} value at {id} ({v:.2}) is {z:+.1} robust standard \
                     deviations from the series median ({median_v:.2}), beyond the {z_threshold:.1} \
                     watch threshold. This is a statistical outlier not explained by a \
                     period-over-period move."
                ),
                severity: severity.into(),
                confidence,
                evidence: vec![
                    EvidenceRef {
                        record_id: id.to_string(),
                        field: series_field.into(),
                        value: serde_json::json!(*v),
                        context: Some("flagged point".into()),
                    },
                    EvidenceRef {
                        record_id: "series".into(),
                        field: "median".into(),
                        value: serde_json::json!(median_v),
                        context: Some("series median (MAD baseline)".into()),
                    },
                ],
            });
        }
    }
    findings
}

/// Detect strong seasonality via autocorrelation at common HKGOV reporting
/// periods (monthly lag 12, quarterly lag 4). Emits one finding per lag whose
/// autocorrelation exceeds `min_r` (defaults to [`DEFAULT_SEASONALITY_R`]).
pub fn detect_seasonality(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    series_field: &str,
    min_r: f64,
) -> Vec<Finding> {
    let series = numeric_series(records, series_field);
    if series.len() < MIN_SAMPLES {
        return Vec::new();
    }
    let min_r = if min_r > 0.0 {
        min_r
    } else {
        DEFAULT_SEASONALITY_R
    };
    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();

    // Candidate lags for HKGOV data: monthly (12) and quarterly (4). Require
    // enough overlap for the pattern to repeat at least twice — otherwise a few
    // coincidental matches produce a spuriously high r on short series.
    let candidate_lags = [12usize, 4];
    let mut findings = Vec::new();
    for &lag in &candidate_lags {
        let overlap = values.len().saturating_sub(lag);
        if overlap < lag.saturating_mul(2) {
            continue;
        }
        let r = autocorrelation(&values, lag);
        if r.abs() >= min_r {
            let confidence = (r.abs().min(1.0) - min_r).max(0.0).clamp(0.5, 1.0);
            findings.push(Finding {
                kind: "seasonality".into(),
                source,
                dataset: dataset.into(),
                title: format!(
                    "{series_field} shows seasonality at lag {lag} (autocorrelation {r:+.2})"
                ),
                heuristic_summary: format!(
                    "The {series_field} series autocorrelates at lag {lag} with r = {r:+.2}, \
                     above the {min_r:.1} watch threshold. This indicates a repeating seasonal \
                     pattern (e.g. month-of-year or quarter-of-year effects) worth accounting \
                     for before reading a single period's move as anomalous."
                ),
                severity: "info".into(),
                confidence,
                evidence: vec![EvidenceRef {
                    record_id: "series".into(),
                    field: format!("autocorrelation_lag_{lag}"),
                    value: serde_json::json!(r),
                    context: Some(format!("lag-{lag} autocorrelation coefficient")),
                }],
            });
        }
    }
    findings
}

/// Detect correlation (or its loss) between two numeric fields over the same
/// records. Emits one finding when |r| is *low* (below `min_r`, default
/// [`DEFAULT_CORRELATION_R`]) — i.e. two series expected to move together have
/// decoupled. A high |r| is not flagged (it's the expected state).
pub fn detect_correlation(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    field_a: &str,
    field_b: &str,
    min_r: f64,
) -> Vec<Finding> {
    // Pair up values where BOTH fields are present on the same record.
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for r in records {
        let Some(a) = numeric(r, field_a) else {
            continue;
        };
        let Some(b) = numeric(r, field_b) else {
            continue;
        };
        pairs.push((a, b));
    }
    if pairs.len() < MIN_SAMPLES {
        return Vec::new();
    }
    let min_r = if min_r > 0.0 {
        min_r
    } else {
        DEFAULT_CORRELATION_R
    };

    let r = pearson(&pairs);
    let mut findings = Vec::new();
    if r.abs() < min_r {
        let confidence = (1.0 - r.abs()).clamp(0.5, 1.0);
        findings.push(Finding {
            kind: "correlation".into(),
            source,
            dataset: dataset.into(),
            title: format!("{field_a} and {field_b} are decoupled (r = {r:+.2})"),
            heuristic_summary: format!(
                "{field_a} and {field_b} correlate at only r = {r:+.2} across {} paired \
                 observations, below the {min_r:.1} watch threshold. If these series are \
                 expected to move together, the decoupling may signal a regime change or a \
                 data-quality issue worth investigating.",
                pairs.len()
            ),
            severity: "info".into(),
            confidence,
            evidence: vec![EvidenceRef {
                record_id: "series".into(),
                field: format!("{field_a}_vs_{field_b}"),
                value: serde_json::json!(r),
                context: Some("Pearson correlation over paired observations".into()),
            }],
        });
    }
    findings
}

// ---------------------------------------------------------------------------
// Cadence-aware detectors (v7) + proxy_divergence + benchmark_deviation.
//
// The v3-v6 detectors assumed every series was a stream of adjacent periods.
// That's wrong for quarterly/annual data: a 10% QoQ move on a seasonal series
// is mostly noise, while 10% YoY is real. These detectors take a `Cadence` and
// a `Comparison` so the threshold is calibrated to what "normal" means for
// that series.
// ---------------------------------------------------------------------------

/// Default % move at a daily cadence. Higher cadences scale this up — see
/// [`scale_threshold_for_cadence`].
pub const DEFAULT_PCT_THRESHOLD: f64 = 15.0;
/// Default minimum records either side of a YoY comparison before we'll opine.
pub const MIN_YOY_SAMPLES: usize = 4;
/// Default |delta/value| above which a proxy divergence is flagged.
pub const DEFAULT_PROXY_DELTA_PCT: f64 = 5.0;
/// Default correlation below which two proxies are considered "decoupled".
pub const DEFAULT_PROXY_R: f64 = 0.6;
/// Default |actual - benchmark| / |benchmark| above which a benchmark deviation
/// is flagged.
pub const DEFAULT_BENCHMARK_PCT: f64 = 10.0;

/// Scale a flat % threshold for a cadence. The intuition: returns compound with
/// the sqrt of time, so an acceptable single-period move scales *down* with the
/// sqrt of how many periods-per-year the cadence represents, normalized to a
/// monthly baseline (sqrt(12)). Daily → ~0.22× (smaller moves flag, since a
/// normal daily move is tiny); annual → ~3.46× (bigger moves tolerated, since a
/// normal annual move is large). `Unknown` → 1.0× (no scaling).
pub fn scale_threshold_for_cadence(base_pct: f64, cadence: Cadence) -> f64 {
    if matches!(cadence, Cadence::Unknown) {
        return base_pct;
    }
    let monthly = 12.0_f64;
    let scale = (monthly / cadence.periods_per_year()).sqrt();
    base_pct * scale
}

/// Detect period-over-period moves, cadence-aware. Same shape as
/// [`detect_series_jumps`] but the threshold is scaled by [`Cadence`] so a
/// normally-sized move for the cadence isn't flagged. For seasonal series, use
/// [`detect_year_over_year`] instead.
pub fn detect_series_jumps_cadenced(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    series_field: &str,
    base_pct_threshold: f64,
    cadence: Cadence,
) -> Vec<Finding> {
    let scaled = scale_threshold_for_cadence(
        if base_pct_threshold > 0.0 {
            base_pct_threshold
        } else {
            DEFAULT_PCT_THRESHOLD
        },
        cadence,
    );
    // Delegate to the original detector with the scaled threshold.
    detect_series_jumps(source, dataset, records, series_field, scaled)
}

/// Detect year-over-year moves. Each period is compared to the period a year
/// ago (by record_id prefix matching), removing seasonality. The right
/// comparison for quarterly retail / tourism / fiscal lines where Q3-vs-Q2 is
/// dominated by calendar effects.
///
/// `periods_per_year` is the number of records per year (4 for quarterly, 12
/// for monthly, 1 for annual). When the offset can't be applied (too few
/// records, or no match a year back), that period is skipped.
pub fn detect_year_over_year(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    series_field: &str,
    pct_threshold: f64,
    periods_per_year: usize,
) -> Vec<Finding> {
    let series = numeric_series(records, series_field);
    if series.len() < periods_per_year + MIN_YOY_SAMPLES {
        return Vec::new();
    }
    let threshold = if pct_threshold > 0.0 {
        pct_threshold
    } else {
        DEFAULT_PCT_THRESHOLD
    };
    let mut findings = Vec::new();
    for (curr_idx, (curr_id, curr_v)) in series.iter().enumerate() {
        // Compare against the period a year ago.
        let prev_idx = match curr_idx.checked_sub(periods_per_year) {
            Some(i) => i,
            None => continue,
        };
        let (prev_id, prev_v) = series[prev_idx];
        if prev_v.abs() < f64::EPSILON {
            continue;
        }
        let pct = ((curr_v - prev_v) / prev_v.abs()) * 100.0;
        if pct.abs() >= threshold {
            let confidence = ((pct.abs() / threshold).min(5.0) / 5.0).clamp(0.5, 1.0);
            let severity = if pct.abs() >= threshold * 3.0 {
                "critical"
            } else {
                "warning"
            };
            findings.push(Finding {
                kind: "year_over_year".into(),
                source,
                dataset: dataset.into(),
                title: format!("{series_field} {pct:+.1}% YoY ({prev_id} → {curr_id})"),
                heuristic_summary: format!(
                    "The {series_field} series changed by {pct:+.1}% year-over-year \
                     ({prev_id}: {prev_v:.2} → {curr_id}: {curr_v:.2}). The {threshold:.0}% \
                     YoY watch threshold was crossed — a real move after removing seasonality."
                ),
                severity: severity.into(),
                confidence,
                evidence: vec![
                    EvidenceRef {
                        record_id: prev_id.to_string(),
                        field: series_field.into(),
                        value: serde_json::json!(prev_v),
                        context: Some("one year prior".into()),
                    },
                    EvidenceRef {
                        record_id: curr_id.to_string(),
                        field: series_field.into(),
                        value: serde_json::json!(curr_v),
                        context: Some("current period".into()),
                    },
                ],
            });
        }
    }
    findings
}

/// Detect divergence between two proxy series that *should* tell the same story.
/// This is the hero detector for "non-obvious lies": two datasets measuring the
/// same underlying fact (e.g. land revenue via fiscal receipts vs. land
/// transactions) that decouple — a candidate signal that one is being
/// misreported, mis-defined, or lags the other.
///
/// Two series are joined on a shared key (`join_field`, default `record_id`) and
/// tested two ways:
/// 1. **Value divergence:** for the most recent joined period, how far apart
///    are the two values, as % of the larger? Above `delta_pct` → flagged.
/// 2. **Relationship breakdown:** Pearson r over the joined history; if below
///    `min_r`, the proxies have decoupled over time → flagged.
///
/// Both `primary` and `companion` are pre-extracted (key, value) pairs, so this
/// detector is pure and testable without the store.
#[allow(clippy::too_many_arguments)] // stable detector API; grouping into a struct would obscure the call sites
pub fn detect_proxy_divergence(
    source: DataSource,
    dataset: &str,
    series_field: &str,
    companion_source: DataSource,
    companion_dataset: &str,
    companion_field: &str,
    primary: &[NormalizedRecord],
    companion: &[NormalizedRecord],
    join_field: Option<&str>,
    delta_pct: f64,
    min_r: f64,
) -> Vec<Finding> {
    let jf = join_field.unwrap_or("record_id");
    // Index the companion by join key → value.
    let companion_map: std::collections::HashMap<String, f64> = companion
        .iter()
        .filter_map(|r| {
            let key = join_key(r, jf)?;
            let val = numeric(r, companion_field)?;
            Some((key, val))
        })
        .collect();

    // Build paired observations on the join key, sorted ascending.
    let mut pairs: Vec<(String, f64, f64)> = primary
        .iter()
        .filter_map(|r| {
            let key = join_key(r, jf)?;
            let a = numeric(r, series_field)?;
            let b = *companion_map.get(&key)?;
            Some((key, a, b))
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    if pairs.len() < MIN_SAMPLES {
        return Vec::new();
    }
    let delta_pct = if delta_pct > 0.0 {
        delta_pct
    } else {
        DEFAULT_PROXY_DELTA_PCT
    };
    let min_r = if min_r > 0.0 { min_r } else { DEFAULT_PROXY_R };

    let mut findings = Vec::new();

    // (1) Value divergence on the most recent joined period.
    let (last_key, last_a, last_b) = pairs.last().cloned().unwrap();
    let base = last_a.abs().max(last_b.abs());
    if base > f64::EPSILON {
        let delta_pct_observed = ((last_a - last_b).abs() / base) * 100.0;
        if delta_pct_observed >= delta_pct {
            let confidence = ((delta_pct_observed / delta_pct).min(4.0) / 4.0).clamp(0.5, 1.0);
            findings.push(Finding {
                kind: "proxy_divergence".into(),
                source,
                dataset: dataset.into(),
                title: format!(
                    "{series_field} vs {companion_field} diverged by {delta_pct_observed:.1}% \
                     at {last_key}"
                ),
                heuristic_summary: format!(
                    "The proxies {series_field} ({source}/{dataset}) and {companion_field} \
                     ({companion_source}/{companion_dataset}) — which should measure the same \
                     underlying fact — disagree by {delta_pct_observed:.1}% at {last_key} \
                     ({last_a:.2} vs {last_b:.2}). This is above the {delta_pct:.0}% watch \
                     threshold and a candidate misalignment worth investigating."
                ),
                severity: "warning".into(),
                confidence,
                evidence: vec![
                    EvidenceRef {
                        record_id: last_key.clone(),
                        field: series_field.into(),
                        value: serde_json::json!(last_a),
                        context: Some(format!("{source}/{dataset} (primary proxy)")),
                    },
                    EvidenceRef {
                        record_id: last_key.clone(),
                        field: companion_field.into(),
                        value: serde_json::json!(last_b),
                        context: Some(format!(
                            "{companion_source}/{companion_dataset} (companion proxy)"
                        )),
                    },
                ],
            });
        }
    }

    // (2) Relationship breakdown over history.
    let r = pearson(&pairs.iter().map(|(_, a, b)| (*a, *b)).collect::<Vec<_>>());
    if r.abs() < min_r {
        let confidence = (1.0 - r.abs()).clamp(0.5, 1.0);
        findings.push(Finding {
            kind: "proxy_divergence".into(),
            source,
            dataset: dataset.into(),
            title: format!(
                "{series_field} and {companion_field} have decoupled (r = {r:+.2} over {} periods)",
                pairs.len()
            ),
            heuristic_summary: format!(
                "The proxies {series_field} and {companion_field} historically moved together \
                 but now correlate at only r = {r:+.2} over {n} joined periods, below the \
                 {min_r:.1} watch threshold. A sustained decoupling often signals one proxy has \
                 changed definition, lagged the other, or the underlying relationship has shifted.",
                n = pairs.len()
            ),
            severity: "warning".into(),
            confidence,
            evidence: vec![EvidenceRef {
                record_id: "joined_history".into(),
                field: format!("pearson_r_{series_field}_vs_{companion_field}"),
                value: serde_json::json!(r),
                context: Some("correlation over all joined periods".into()),
            }],
        });
    }

    findings
}

/// Extract the join key for a record. If `join_field` is `record_id`, use the
/// record's id; otherwise read the named field as a string.
fn join_key(r: &NormalizedRecord, join_field: &str) -> Option<String> {
    if join_field == "record_id" {
        return Some(r.record_id.clone());
    }
    match r.fields.get(join_field)? {
        RecordValue::Str(s) => Some(s.clone()),
        RecordValue::Int(i) => Some(i.to_string()),
        RecordValue::Float(f) => Some(f.to_string()),
        _ => None,
    }
}

/// Detect deviation of a series from a benchmark (an assumed/projected/peer
/// value). The "budget lie" detector: actual receipts vs. the budget speech
/// assumption, actual GDP vs. forecast, actual vs. peer-city index.
///
/// `actual` is the observed series; `benchmark` carries the assumed value(s).
/// Both are joined on `join_field`. A finding is emitted for each joined period
/// where |actual - benchmark| / |benchmark| >= `pct_threshold`.
#[allow(clippy::too_many_arguments)] // stable detector API
pub fn detect_benchmark_deviation(
    source: DataSource,
    dataset: &str,
    field: &str,
    benchmark_field: &str,
    actual: &[NormalizedRecord],
    benchmarks: &[NormalizedRecord],
    join_field: Option<&str>,
    pct_threshold: f64,
) -> Vec<Finding> {
    let jf = join_field.unwrap_or("record_id");
    let bench_map: std::collections::HashMap<String, f64> = benchmarks
        .iter()
        .filter_map(|r| {
            let key = join_key(r, jf)?;
            let val = numeric(r, benchmark_field)?;
            Some((key, val))
        })
        .collect();

    let threshold = if pct_threshold > 0.0 {
        pct_threshold
    } else {
        DEFAULT_BENCHMARK_PCT
    };

    let mut findings = Vec::new();
    for r in actual {
        let key = match join_key(r, jf) {
            Some(k) => k,
            None => continue,
        };
        let actual_v = match numeric(r, field) {
            Some(v) => v,
            None => continue,
        };
        let bench_v = match bench_map.get(&key) {
            Some(v) => *v,
            None => continue,
        };
        if bench_v.abs() < f64::EPSILON {
            continue;
        }
        let pct = ((actual_v - bench_v) / bench_v.abs()) * 100.0;
        if pct.abs() >= threshold {
            let confidence = ((pct.abs() / threshold).min(4.0) / 4.0).clamp(0.5, 1.0);
            let severity = if pct.abs() >= threshold * 2.0 {
                "critical"
            } else {
                "warning"
            };
            findings.push(Finding {
                kind: "benchmark_deviation".into(),
                source,
                dataset: dataset.into(),
                title: format!("{field} is {pct:+.1}% vs benchmark at {key}"),
                heuristic_summary: format!(
                    "The {field} value at {key} ({actual_v:.2}) deviates {pct:+.1}% from the \
                     benchmark {benchmark_field} ({bench_v:.2}). Beyond the {threshold:.0}% watch \
                     threshold — an actual-vs-assumed gap worth flagging."
                ),
                severity: severity.into(),
                confidence,
                evidence: vec![
                    EvidenceRef {
                        record_id: key.clone(),
                        field: field.into(),
                        value: serde_json::json!(actual_v),
                        context: Some("actual".into()),
                    },
                    EvidenceRef {
                        record_id: key.clone(),
                        field: benchmark_field.into(),
                        value: serde_json::json!(bench_v),
                        context: Some("benchmark (assumed/projected)".into()),
                    },
                ],
            });
        }
    }
    findings
}

/// Direction of a threshold crossing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrossDirection {
    /// The value rose above the threshold (e.g. a rate exceeded a watch level).
    Above,
    /// The value fell below the threshold (e.g. reserves dipped under a floor).
    Below,
}

impl CrossDirection {
    /// Did `value` cross this direction relative to `threshold`?
    pub fn crossed(&self, value: f64, threshold: f64) -> bool {
        match self {
            CrossDirection::Above => value > threshold,
            CrossDirection::Below => value < threshold,
        }
    }
}

/// Detect a threshold crossing — the event detector (v9). Unlike the deviation
/// detectors (`series_jump`, `outlier`), this doesn't look at *change*; it looks
/// at an absolute level against a configured watch line. This is how the
/// platform surfaces **opportunities and risks** rather than just anomalies:
///
/// - Below: "HIBOR overnight fell below 1.0% — cheap funding window open" (opportunity)
/// - Above: "closing balance exceeded HKD 50bn — ample liquidity" (context)
/// - Below: "FX reserves fell below the IMF adequacy ratio" (risk)
///
/// Emits at most one finding (the latest period), since a crossing is a
/// state, not a series of events. Recurrence is deduped downstream by the
/// insight id (which fingerprints the threshold + direction, not the value).
pub fn detect_threshold_crossing(
    source: DataSource,
    dataset: &str,
    records: &[NormalizedRecord],
    field: &str,
    threshold: f64,
    direction: CrossDirection,
) -> Vec<Finding> {
    let series = numeric_series(records, field);
    let Some((latest_id, latest_v)) = series.last().copied() else {
        return Vec::new();
    };
    if !direction.crossed(latest_v, threshold) {
        return Vec::new();
    }
    // Did the previous period also cross? If so, this isn't a fresh crossing —
    // it's a sustained state. We still report it (the user asked to watch this
    // level) but frame it as "still crossed" rather than "just crossed".
    let prev_crossed = series
        .len()
        .checked_sub(2)
        .and_then(|i| series.get(i))
        .map(|(_, v)| direction.crossed(*v, threshold))
        .unwrap_or(false);
    let verb = if prev_crossed { "remains" } else { "crossed" };
    let prep = match direction {
        CrossDirection::Above => "above",
        CrossDirection::Below => "below",
    };
    let severity = match direction {
        CrossDirection::Below => "warning",
        CrossDirection::Above => "info",
    };
    let confidence = if prev_crossed { 0.6 } else { 0.9 };
    vec![Finding {
        kind: "threshold_crossing".into(),
        source,
        dataset: dataset.into(),
        title: format!("{field} {verb} {prep} {threshold} at {latest_id} ({latest_v:.2})"),
        heuristic_summary: format!(
            "The {field} value at {latest_id} ({latest_v:.2}) is {prep} the {threshold} \
             watch threshold. This is an event signal — a level the operator flagged as worth \
             knowing about — rather than a deviation from history."
        ),
        severity: severity.into(),
        confidence,
        evidence: vec![
            EvidenceRef {
                record_id: latest_id.to_string(),
                field: field.into(),
                value: serde_json::json!(latest_v),
                context: Some("latest value".into()),
            },
            EvidenceRef {
                record_id: "threshold".into(),
                field: field.into(),
                value: serde_json::json!(threshold),
                context: Some(format!("watch {direction:?} threshold")),
            },
        ],
    }]
}

/// Look up a numeric field on one record, coercing Int → Float.
fn numeric(r: &NormalizedRecord, field: &str) -> Option<f64> {
    match r.fields.get(field)? {
        RecordValue::Float(v) => Some(*v),
        RecordValue::Int(v) => Some(*v as f64),
        _ => None,
    }
}

/// Median of a (non-empty) slice. Sorts a copy; callers pass small vectors.
fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Pearson correlation over paired observations. Returns 0.0 if either series
/// has zero variance (the correlation is undefined).
fn pearson(pairs: &[(f64, f64)]) -> f64 {
    let n = pairs.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (sx, sy, sxy, sx2, sy2) = pairs.iter().fold((0.0, 0.0, 0.0, 0.0, 0.0), |acc, (x, y)| {
        (
            acc.0 + x,
            acc.1 + y,
            acc.2 + x * y,
            acc.3 + x * x,
            acc.4 + y * y,
        )
    });
    let cov = n * sxy - sx * sy;
    let var_x = n * sx2 - sx * sx;
    let var_y = n * sy2 - sy * sy;
    let denom = (var_x * var_y).sqrt();
    if denom < f64::EPSILON {
        return 0.0;
    }
    cov / denom
}

/// Autocorrelation of a series at a given lag. Returns the Pearson-style r of
/// the series against itself shifted by `lag`.
fn autocorrelation(values: &[f64], lag: usize) -> f64 {
    if lag == 0 || values.len() <= lag {
        return 0.0;
    }
    let pairs: Vec<(f64, f64)> = values
        .iter()
        .zip(values.iter().skip(lag))
        .map(|(a, b)| (*a, *b))
        .collect();
    pearson(&pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hkgov_common::RecordValue;
    use std::collections::BTreeMap;

    fn rec(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut f = BTreeMap::new();
        f.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields: f,
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn flags_large_jump() {
        let recs = vec![rec("2026-01", "rate", 2.0), rec("2026-02", "rate", 4.0)];
        let f = detect_series_jumps(DataSource::Hkma, "x", &recs, "rate", 50.0);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "series_jump");
        assert!(f[0].confidence >= 0.5);
    }

    #[test]
    fn ignores_small_move() {
        let recs = vec![rec("2026-01", "rate", 2.0), rec("2026-02", "rate", 2.1)];
        let f = detect_series_jumps(DataSource::Hkma, "x", &recs, "rate", 50.0);
        assert!(f.is_empty());
    }

    #[test]
    fn detects_press_data_gap() {
        let press = vec!["2026-06-18".to_string(), "2026-06-19".to_string()];
        let data = vec!["2026-06-18".to_string()];
        let f = detect_cross_source_gaps(DataSource::Press, "hkma-press-releases", &press, &data);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "cross_source_gap");
    }

    #[test]
    fn finding_promotes_to_insight_with_stable_id() {
        let recs = vec![rec("2026-01", "rate", 2.0), rec("2026-02", "rate", 5.0)];
        let f = detect_series_jumps(DataSource::Hkma, "x", &recs, "rate", 50.0);
        let i = f
            .into_iter()
            .next()
            .unwrap()
            .into_insight("summary".into(), 0.9, "heuristic");
        assert!(i.id.starts_with("series_jump:hkma:x:"));
        assert!(i.confidence <= 1.0);
    }

    // ---- outlier detector -------------------------------------------------

    #[test]
    fn outlier_flags_spike() {
        // A normally-varying series with one obvious spike. MAD must be > 0 for
        // the robust z-score to be defined, so the baseline needs real variance.
        let baseline = [9.8, 10.1, 9.9, 10.2, 10.0, 9.7];
        let mut recs: Vec<NormalizedRecord> = baseline
            .iter()
            .enumerate()
            .map(|(i, v)| rec(&format!("2026-{i:02}"), "v", *v))
            .collect();
        recs.push(rec("2026-99", "v", 100.0));
        let f = detect_outliers(DataSource::Hkma, "x", &recs, "v", 3.5);
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].kind, "outlier");
        // 100 vs a ~10 baseline is a >2× robust-z multiple → critical.
        assert_eq!(f[0].severity, "critical");
    }

    #[test]
    fn outlier_ignores_flat_series() {
        // MAD == 0 (all values identical): no outliers, no panic.
        let recs: Vec<NormalizedRecord> = (0..6)
            .map(|i| rec(&format!("2026-{i:02}"), "v", 5.0))
            .collect();
        let f = detect_outliers(DataSource::Hkma, "x", &recs, "v", 3.5);
        assert!(f.is_empty());
    }

    #[test]
    fn outlier_needs_min_samples() {
        // Fewer than MIN_SAMPLES points → no findings.
        let recs = vec![rec("2026-01", "v", 1.0), rec("2026-02", "v", 99.0)];
        assert!(detect_outliers(DataSource::Hkma, "x", &recs, "v", 3.5).is_empty());
    }

    // ---- seasonality detector --------------------------------------------

    #[test]
    fn seasonality_flags_repeating_pattern() {
        // A perfect monthly cycle of length 4 (e.g. quarterly) repeated 4×.
        let pattern = [10.0, 20.0, 30.0, 40.0];
        let recs: Vec<NormalizedRecord> = (0..16)
            .map(|i| rec(&format!("p{i:02}"), "v", pattern[i % 4]))
            .collect();
        // lag 4 should be a near-perfect autocorrelation.
        let f = detect_seasonality(DataSource::Hkma, "x", &recs, "v", 0.6);
        assert!(f.iter().any(|x| x.kind == "seasonality"));
    }

    #[test]
    fn seasonality_ignores_noise() {
        // A genuinely aperiodic sequence: sin(i * 1.3) has no exact periodicity
        // at lag 4 or 12. The autocorrelation stays well below the 0.6 watch
        // threshold, so the detector should produce no findings.
        let recs: Vec<NormalizedRecord> = (0..32)
            .map(|i| rec(&format!("p{i:02}"), "v", ((i as f64) * 1.3).sin() * 10.0))
            .collect();
        let f = detect_seasonality(DataSource::Hkma, "x", &recs, "v", 0.6);
        assert!(
            f.is_empty(),
            "expected no seasonality findings, got: {:?}",
            f.iter()
                .map(|x| (x.kind.clone(), x.title.clone()))
                .collect::<Vec<_>>()
        );
    }

    // ---- correlation detector --------------------------------------------

    #[test]
    fn correlation_flags_decoupling() {
        // Two fields with zero correlation.
        let recs: Vec<NormalizedRecord> = (0..8)
            .map(|i| {
                let mut fields = BTreeMap::new();
                fields.insert("a".into(), RecordValue::Float((i as f64).sin() * 10.0));
                fields.insert("b".into(), RecordValue::Float((i as f64).cos() * 10.0));
                NormalizedRecord {
                    source: DataSource::Hkma,
                    dataset: "x".into(),
                    record_id: format!("p{i:02}"),
                    fields,
                    fetched_at: Utc::now(),
                }
            })
            .collect();
        let f = detect_correlation(DataSource::Hkma, "x", &recs, "a", "b", 0.3);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "correlation");
    }

    #[test]
    fn correlation_quiet_when_coupled() {
        // b == a * 2: perfect correlation → NOT flagged (expected state).
        let recs: Vec<NormalizedRecord> = (0..8)
            .map(|i| {
                let mut fields = BTreeMap::new();
                fields.insert("a".into(), RecordValue::Float(i as f64));
                fields.insert("b".into(), RecordValue::Float(i as f64 * 2.0));
                NormalizedRecord {
                    source: DataSource::Hkma,
                    dataset: "x".into(),
                    record_id: format!("p{i:02}"),
                    fields,
                    fetched_at: Utc::now(),
                }
            })
            .collect();
        let f = detect_correlation(DataSource::Hkma, "x", &recs, "a", "b", 0.3);
        assert!(f.is_empty());
    }

    // ---- math helpers -----------------------------------------------------

    #[test]
    fn median_handles_even_and_odd() {
        assert_eq!(median(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(median(&[5.0]), 5.0);
    }

    #[test]
    fn pearson_known_values() {
        // Perfect positive correlation.
        assert!((pearson(&[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)]) - 1.0).abs() < 1e-9);
        // Perfect negative correlation.
        assert!((pearson(&[(1.0, 6.0), (2.0, 4.0), (3.0, 2.0)]) + 1.0).abs() < 1e-9);
        // Zero variance → 0.0, not NaN.
        assert_eq!(pearson(&[(1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]), 0.0);
    }

    #[test]
    fn numeric_series_sorts_and_filters() {
        let recs = vec![
            rec("2026-03", "v", 3.0),
            rec("2026-01", "v", 1.0),
            rec("2026-02", "v", 2.0),
        ];
        let s = numeric_series(&recs, "v");
        assert_eq!(
            s,
            vec![("2026-01", 1.0), ("2026-02", 2.0), ("2026-03", 3.0)]
        );
    }

    // ---- v7: cadence scaling ---------------------------------------------

    #[test]
    fn cadence_scales_threshold_correctly() {
        // Monthly is the baseline → 1.0×.
        let base = 10.0;
        assert!((scale_threshold_for_cadence(base, Cadence::Monthly) - 10.0).abs() < 1e-9);
        // Daily → smaller (more sensitive).
        assert!(scale_threshold_for_cadence(base, Cadence::Daily) < 10.0);
        // Annual → larger (less sensitive).
        assert!(scale_threshold_for_cadence(base, Cadence::Annual) > 10.0);
        // Unknown → unchanged.
        assert!((scale_threshold_for_cadence(base, Cadence::Unknown) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn cadenced_series_jump_tolerates_normal_annual_move() {
        // A 12% move that would flag at daily (threshold ~6.7%) should NOT flag
        // at annual (threshold ~34.6%).
        let recs = vec![rec("2025", "v", 100.0), rec("2026", "v", 112.0)];
        // Daily: flags.
        assert!(!detect_series_jumps_cadenced(
            DataSource::Hkma,
            "x",
            &recs,
            "v",
            10.0,
            Cadence::Daily
        )
        .is_empty());
        // Annual: tolerated.
        assert!(detect_series_jumps_cadenced(
            DataSource::Hkma,
            "x",
            &recs,
            "v",
            10.0,
            Cadence::Annual
        )
        .is_empty());
    }

    // ---- v7: year_over_year ----------------------------------------------

    #[test]
    fn yoy_flags_real_move_ignoring_seasonality() {
        // Quarterly series (4/year): a Q with a 10% YoY jump.
        // Layout: 2025-Q1..Q4 (baseline 100), 2026-Q1..Q4 (Q1 jumps to 130).
        let mut recs: Vec<NormalizedRecord> = Vec::new();
        for q in 1..=4 {
            recs.push(rec(&format!("2025-Q{q}"), "v", 100.0));
        }
        recs.push(rec("2026-Q1", "v", 130.0)); // +30% YoY
        for q in 2..=4 {
            recs.push(rec(&format!("2026-Q{q}"), "v", 100.0));
        }
        let f = detect_year_over_year(DataSource::Hkma, "x", &recs, "v", 25.0, 4);
        assert!(f
            .iter()
            .any(|x| x.kind == "year_over_year" && x.title.contains("Q1")));
    }

    #[test]
    fn yoy_needs_enough_history() {
        // Fewer than periods_per_year + MIN_YOY_SAMPLES → empty.
        let recs = vec![rec("2025-Q1", "v", 1.0), rec("2026-Q1", "v", 100.0)];
        assert!(detect_year_over_year(DataSource::Hkma, "x", &recs, "v", 25.0, 4).is_empty());
    }

    // ---- v7: proxy_divergence --------------------------------------------

    fn rec_with(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut f = BTreeMap::new();
        f.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields: f,
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn proxy_divergence_flags_value_mismatch() {
        // Two proxies measuring the "same" thing: agree for 3 periods, then
        // diverge sharply on the last.
        let primary = vec![
            rec_with("p1", "a", 100.0),
            rec_with("p2", "a", 110.0),
            rec_with("p3", "a", 120.0),
            rec_with("p4", "a", 200.0), // primary says 200
        ];
        let companion = vec![
            rec_with("p1", "b", 100.0),
            rec_with("p2", "b", 110.0),
            rec_with("p3", "b", 120.0),
            rec_with("p4", "b", 125.0), // companion says 125 → big divergence
        ];
        let f = detect_proxy_divergence(
            DataSource::Hkma,
            "x",
            "a",
            DataSource::DataGovHk,
            "y",
            "b",
            &primary,
            &companion,
            None,
            5.0,
            0.6,
        );
        // Should flag the value divergence (200 vs 125 is ~46% apart).
        assert!(f
            .iter()
            .any(|x| x.kind == "proxy_divergence" && x.title.contains("diverged")));
    }

    #[test]
    fn proxy_divergence_flags_decoupling() {
        // Proxies that tracked each other then decoupled: primary keeps rising,
        // companion reverses and falls. r drops well below 0.6.
        let primary = vec![
            rec_with("p1", "a", 1.0),
            rec_with("p2", "a", 2.0),
            rec_with("p3", "a", 3.0),
            rec_with("p4", "a", 4.0),
            rec_with("p5", "a", 5.0),
            rec_with("p6", "a", 6.0),
            rec_with("p7", "a", 7.0),
            rec_with("p8", "a", 8.0),
        ];
        // Companion tracks 1:1 for first 4, then declines — a real decoupling.
        let companion = vec![
            rec_with("p1", "b", 1.0),
            rec_with("p2", "b", 2.0),
            rec_with("p3", "b", 3.0),
            rec_with("p4", "b", 4.0),
            rec_with("p5", "b", 3.5),
            rec_with("p6", "b", 3.0),
            rec_with("p7", "b", 2.5),
            rec_with("p8", "b", 2.0),
        ];
        let f = detect_proxy_divergence(
            DataSource::Hkma,
            "x",
            "a",
            DataSource::DataGovHk,
            "y",
            "b",
            &primary,
            &companion,
            None,
            90.0, // high delta threshold so only the decoupling fires
            0.6,
        );
        assert!(f.iter().any(|x| x.title.contains("decoupled")), "got {f:?}");
    }

    #[test]
    fn proxy_divergence_quiet_when_aligned() {
        // Two identical proxies → no findings.
        let recs = vec![
            rec_with("p1", "a", 100.0),
            rec_with("p2", "a", 110.0),
            rec_with("p3", "a", 120.0),
            rec_with("p4", "a", 130.0),
        ];
        let f = detect_proxy_divergence(
            DataSource::Hkma,
            "x",
            "a",
            DataSource::DataGovHk,
            "y",
            "a",
            &recs,
            &recs,
            None,
            5.0,
            0.6,
        );
        assert!(f.is_empty(), "expected no divergence findings, got {f:?}");
    }

    #[test]
    fn proxy_divergence_joins_on_named_field() {
        // Join on a "quarter" field, not record_id. Need >= MIN_SAMPLES joined.
        let mk = |id: &str, q: &str, field: &str, v: f64| NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields: {
                let mut m = BTreeMap::new();
                m.insert(field.into(), RecordValue::Float(v));
                m.insert("quarter".into(), RecordValue::Str(q.into()));
                m
            },
            fetched_at: Utc::now(),
        };
        let primary = vec![
            mk("r1", "2025Q4", "a", 100.0),
            mk("r2", "2026Q1", "a", 100.0),
            mk("r3", "2026Q2", "a", 100.0),
            mk("r4", "2026Q3", "a", 100.0),
            mk("r5", "2026Q4", "a", 200.0), // diverges sharply in last period
        ];
        let companion = vec![
            mk("s1", "2025Q4", "b", 100.0),
            mk("s2", "2026Q1", "b", 100.0),
            mk("s3", "2026Q2", "b", 100.0),
            mk("s4", "2026Q3", "b", 100.0),
            mk("s5", "2026Q4", "b", 105.0),
        ];
        let f = detect_proxy_divergence(
            DataSource::Hkma,
            "x",
            "a",
            DataSource::DataGovHk,
            "y",
            "b",
            &primary,
            &companion,
            Some("quarter"),
            5.0,
            0.6,
        );
        assert!(f.iter().any(|x| x.title.contains("diverged")), "got {f:?}");
    }

    // ---- v7: benchmark_deviation -----------------------------------------

    #[test]
    fn benchmark_deviation_flags_actual_above_assumed() {
        // Actual receipts vs budget assumption: actual runs hot.
        let actual = vec![
            rec_with("2026-Q1", "receipts", 120.0), // +20% vs 100 benchmark
            rec_with("2026-Q2", "receipts", 95.0),  // -5% vs 100, under threshold
        ];
        let benchmarks = vec![
            rec_with("2026-Q1", "assumed", 100.0),
            rec_with("2026-Q2", "assumed", 100.0),
        ];
        let f = detect_benchmark_deviation(
            DataSource::Hkma,
            "x",
            "receipts",
            "assumed",
            &actual,
            &benchmarks,
            None,
            10.0,
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "benchmark_deviation");
        assert!(f[0].title.contains("Q1"));
    }

    #[test]
    fn benchmark_deviation_quiet_when_within_tolerance() {
        let actual = vec![rec_with("2026-Q1", "v", 105.0)];
        let benchmarks = vec![rec_with("2026-Q1", "b", 100.0)];
        let f = detect_benchmark_deviation(
            DataSource::Hkma,
            "x",
            "v",
            "b",
            &actual,
            &benchmarks,
            None,
            10.0,
        );
        assert!(f.is_empty());
    }

    #[test]
    fn benchmark_deviation_handles_missing_join() {
        // Actual has a period the benchmark doesn't (and vice versa).
        let actual = vec![
            rec_with("2026-Q1", "v", 200.0),
            rec_with("2026-Q2", "v", 50.0), // no benchmark → skipped
        ];
        let benchmarks = vec![rec_with("2026-Q1", "b", 100.0)];
        let f = detect_benchmark_deviation(
            DataSource::Hkma,
            "x",
            "v",
            "b",
            &actual,
            &benchmarks,
            None,
            10.0,
        );
        assert_eq!(f.len(), 1); // only Q1
    }

    // ---- v9: threshold_crossing (event detector) -------------------------

    #[test]
    fn threshold_crossing_flags_below_opportunity() {
        // HIBOR fell below 1.0 — a cheap-funding opportunity.
        let recs = vec![rec("2026-01", "v", 1.5), rec("2026-02", "v", 0.8)];
        let f = detect_threshold_crossing(
            DataSource::Hkma,
            "x",
            &recs,
            "v",
            1.0,
            CrossDirection::Below,
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "threshold_crossing");
        assert!(f[0].title.contains("crossed below"));
        assert_eq!(f[0].severity, "warning");
        // Fresh crossing (previous period was above) → high confidence.
        assert!(f[0].confidence > 0.8);
    }

    #[test]
    fn threshold_crossing_flags_above() {
        let recs = vec![rec("2026-01", "v", 40.0), rec("2026-02", "v", 55.0)];
        let f = detect_threshold_crossing(
            DataSource::Hkma,
            "x",
            &recs,
            "v",
            50.0,
            CrossDirection::Above,
        );
        assert_eq!(f.len(), 1);
        assert!(f[0].title.contains("crossed above"));
        assert_eq!(f[0].severity, "info");
    }

    #[test]
    fn threshold_crossing_quiet_when_not_crossed() {
        let recs = vec![rec("2026-01", "v", 1.5), rec("2026-02", "v", 1.2)];
        assert!(detect_threshold_crossing(
            DataSource::Hkma,
            "x",
            &recs,
            "v",
            1.0,
            CrossDirection::Below
        )
        .is_empty());
    }

    #[test]
    fn threshold_crossing_sustained_states_use_remains() {
        // Both periods below → sustained state, not a fresh cross.
        let recs = vec![rec("2026-01", "v", 0.8), rec("2026-02", "v", 0.7)];
        let f = detect_threshold_crossing(
            DataSource::Hkma,
            "x",
            &recs,
            "v",
            1.0,
            CrossDirection::Below,
        );
        assert_eq!(f.len(), 1);
        assert!(f[0].title.contains("remains below"));
        assert!(f[0].confidence < 0.9); // discounted: not fresh
    }

    #[test]
    fn threshold_crossing_direction_predicate() {
        assert!(CrossDirection::Above.crossed(5.0, 4.0));
        assert!(!CrossDirection::Above.crossed(3.0, 4.0));
        assert!(CrossDirection::Below.crossed(3.0, 4.0));
        assert!(!CrossDirection::Below.crossed(5.0, 4.0));
    }
}
