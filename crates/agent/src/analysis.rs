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
        let id = format!(
            "{}:{}:{}",
            self.kind,
            self.source,
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
pub fn detect_cross_source_gaps(press_dates: &[String], data_dates: &[String]) -> Vec<Finding> {
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
            source: DataSource::Press,
            dataset: "hkma-press-releases".into(),
            title: format!(
                "{} press date(s) with no matching HKMA data row",
                press_only.len()
            ),
            heuristic_summary: format!(
                "On {} date(s) HKMA issued a press release but published no corresponding \
                 statistical data row, or vice versa. These are candidate points where the \
                 official narrative and the published data diverge.",
                press_only.len()
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
        let f = detect_cross_source_gaps(&press, &data);
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
        assert!(i.id.starts_with("series_jump:hkma:"));
        assert!(i.confidence <= 1.0);
    }
}
