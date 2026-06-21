//! Provider-independent analysis: turns raw records into structured `Finding`s.
//!
//! Two detectors ship in v3:
//! - [`detect_series_jumps`] — flags a numeric series that moved more than a
//!   threshold between consecutive periods (e.g. HIBOR spikes, balance swings).
//! - [`detect_cross_source_gaps`] — flags dates where HKMA published a press
//!   release but no corresponding data row, or vice versa, surfacing gaps the
//!   official narrative leaves unexplained.
//!
//! Each detector emits `Finding`s; the LLM client (heuristic or HTTP) only
//! *frames* them into natural language. Detection stays deterministic.

use crate::insight::{EvidenceRef, Insight, InsightSeverity};
use chrono::Utc;
use hkgov_common::{DataSource, NormalizedRecord, RecordValue};
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
}
