//! Bilingual surface (P-106) — zh-HK insight summaries.
//!
//! Per the integration map's key finding: bilingual summaries cannot be purely
//! request-time because the supervisor produces insights in the background
//! (no request context). The cleanest v1 design that preserves the determinism
//! invariant: a **deterministic re-framer** that takes an insight's structured
//! fields and renders a zh-HK summary on demand — no LLM, no scheduler change.
//!
//! ## Determinism
//!
//! [`frame_zh_hk`] is pure Rust over the insight's own fields. Same insight in
//! → same zh-HK summary out → CI-reproducible. When an LLM is configured, a
//! richer zh-HK frame can be produced server-side at produce time (future work);
//! this module is the zero-config fallback that ships now.
//!
//! ## Scope
//!
//! The detector `kind` strings (`series_jump`, `outlier`, …) are machine keys
//! and are NOT translated. Only the human-readable `summary` is re-framed.
//! Evidence values (numbers/dates) are language-neutral and pass through
//! unchanged.

use crate::insight::{Insight, InsightSeverity};

/// The supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    ZhHk,
}

impl Language {
    /// Parse a `?lang=` query value. Defaults to `En` for unknown values.
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(str::to_ascii_lowercase).as_deref() {
            Some("zh-hk") | Some("zh") | Some("zh-hant") | Some("繁") => Language::ZhHk,
            _ => Language::En,
        }
    }
}

/// Re-frame an insight's summary in zh-HK, deterministically. Falls back to the
/// original English summary when the `kind` isn't one we have a template for —
/// so a new detector never produces an empty string.
pub fn frame_zh_hk(insight: &Insight) -> String {
    let sev = severity_zh(insight.severity);
    let field = insight
        .evidence
        .first()
        .map(|e| e.field.as_str())
        .unwrap_or("該指標");
    match insight.kind.as_str() {
        "series_jump" => {
            let prev = insight.evidence.first().and_then(|e| e.value.as_f64());
            let curr = insight.evidence.get(1).and_then(|e| e.value.as_f64());
            match (prev, curr) {
                (Some(p), Some(c)) if p.abs() > f64::EPSILON => {
                    let pct = ((c - p) / p.abs()) * 100.0;
                    format!(
                        "{sev}：{field} 在一個結算周期內變動 {pct:+.1}%（{p:.2} → {c:.2}），超出監察阈值。金管局未就該變動發出新聞稿。"
                    )
                }
                _ => format!("{sev}：{field} 出現顯著變動，金管局未就該變動發出新聞稿。"),
            }
        }
        "outlier" => {
            let v = insight.evidence.first().and_then(|e| e.value.as_f64());
            let median = insight
                .evidence
                .get(1)
                .filter(|e| e.field == "median")
                .and_then(|e| e.value.as_f64());
            match (v, median) {
                (Some(v), Some(m)) => format!(
                    "{sev}：{field} 數值 {v:.2} 偏離中位數 {m:.2}，屬統計離群值，未能由單日波動解釋。"
                ),
                _ => format!("{sev}：{field} 數值屬統計離群值。"),
            }
        }
        "cross_source_gap" => {
            let n = insight.evidence.len();
            format!(
                "{sev}：金管局在 {n} 個日期發出新聞稿但無對應統計數據（或相反）——官方敘事與發布數據出現分歧。"
            )
        }
        "threshold_crossing" => {
            let v = insight.evidence.first().and_then(|e| e.value.as_f64());
            let threshold = insight
                .evidence
                .iter()
                .find(|e| e.record_id == "threshold")
                .and_then(|e| e.value.as_f64());
            match (v, threshold) {
                (Some(v), Some(t)) => {
                    format!("{sev}：{field} 最新數值 {v:.2} 超出監察界綫 {t:.2}。")
                }
                _ => format!("{sev}：{field} 超出監察界綫。"),
            }
        }
        "year_over_year" => {
            format!("{sev}：{field} 按年變動顯著（扣除季節性後仍屬異常）。")
        }
        "seasonality" => {
            format!("{sev}：{field} 呈現季節性規律，分析單期變動時應先扣除季節效應。")
        }
        "correlation" => {
            format!("{sev}：兩個原應同步的指標出現脫鉤，可能反映定義改變或滯後。")
        }
        "proxy_divergence" => {
            format!("{sev}：兩個量度同一事實的代理指標出現分歧，值得調查。")
        }
        "benchmark_deviation" => {
            format!("{sev}：實際數值偏離基準（預算假設／預測），值得標記。")
        }
        _ => insight.summary.clone(), // unknown kind → keep the English summary
    }
}

/// Select the summary for the requested language. `En` returns the stored
/// summary unchanged; `ZhHk` returns the deterministic zh-HK frame.
pub fn select_summary(insight: &Insight, lang: Language) -> String {
    match lang {
        Language::En => insight.summary.clone(),
        Language::ZhHk => frame_zh_hk(insight),
    }
}

fn severity_zh(s: InsightSeverity) -> &'static str {
    match s {
        InsightSeverity::Critical => "嚴重",
        InsightSeverity::Warning => "警告",
        InsightSeverity::Info => "資訊",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::EvidenceRef;
    use chrono::{TimeZone, Utc};
    use hkgov_common::DataSource;
    use serde_json::json;

    fn insight_with(kind: &str, evidence: Vec<EvidenceRef>, severity: InsightSeverity) -> Insight {
        Insight {
            id: format!("{kind}:test"),
            kind: kind.into(),
            severity,
            title: "t".into(),
            summary: "English summary".into(),
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            evidence,
            confidence: 0.8,
            generated_at: Utc.with_ymd_and_hms(2026, 6, 22, 12, 0, 0).unwrap(),
            producer: "test".into(),
            experimental: false,
            first_seen: None,
            version: 1,
            evolution: None,
        }
    }

    fn ev(record_id: &str, field: &str, value: f64, context: Option<&str>) -> EvidenceRef {
        EvidenceRef {
            record_id: record_id.into(),
            field: field.into(),
            value: json!(value),
            context: context.map(str::to_string),
        }
    }

    #[test]
    fn language_parse_handles_common_forms() {
        assert_eq!(Language::parse(Some("zh-HK")), Language::ZhHk);
        assert_eq!(Language::parse(Some("zh")), Language::ZhHk);
        assert_eq!(Language::parse(Some("zh-Hant")), Language::ZhHk);
        assert_eq!(Language::parse(Some("en")), Language::En);
        assert_eq!(Language::parse(None), Language::En);
        assert_eq!(Language::parse(Some("nonsense")), Language::En);
    }

    #[test]
    fn series_jump_zh_frame_has_percentage() {
        let i = insight_with(
            "series_jump",
            vec![
                ev(
                    "2026-02-13",
                    "hibor_overnight",
                    1.47,
                    Some("previous period"),
                ),
                ev(
                    "2026-02-16",
                    "hibor_overnight",
                    2.93,
                    Some("current period"),
                ),
            ],
            InsightSeverity::Critical,
        );
        let zh = frame_zh_hk(&i);
        assert!(zh.contains("變動"));
        assert!(
            zh.contains("99.3%"),
            "zh frame should include the pct: {zh}"
        );
        assert!(zh.contains("嚴重"), "severity translated");
    }

    #[test]
    fn outlier_zh_frame_mentions_median() {
        let i = insight_with(
            "outlier",
            vec![
                ev("2026-03-19", "hibor_overnight", 1.10, Some("flagged point")),
                ev("series", "median", 2.06, Some("series median")),
            ],
            InsightSeverity::Warning,
        );
        let zh = frame_zh_hk(&i);
        assert!(zh.contains("離群值"));
        assert!(zh.contains("中位數"));
    }

    #[test]
    fn cross_source_gap_zh_frame_has_count() {
        let i = insight_with(
            "cross_source_gap",
            vec![ev("2026-05-10", "date", 0.0, Some("press only"))],
            InsightSeverity::Info,
        );
        let zh = frame_zh_hk(&i);
        assert!(zh.contains("新聞稿"));
        assert!(zh.contains("分歧"));
    }

    #[test]
    fn threshold_crossing_zh_frame_has_value() {
        let i = insight_with(
            "threshold_crossing",
            vec![
                ev("2026-06-18", "hibor_overnight", 2.93, Some("latest value")),
                ev("threshold", "hibor_overnight", 2.5, Some("watch threshold")),
            ],
            InsightSeverity::Warning,
        );
        let zh = frame_zh_hk(&i);
        assert!(zh.contains("監察界綫"));
        assert!(zh.contains("2.50") || zh.contains("2.5"));
    }

    #[test]
    fn unknown_kind_falls_back_to_english() {
        let i = insight_with("mystery_detector", vec![], InsightSeverity::Info);
        let zh = frame_zh_hk(&i);
        assert_eq!(zh, "English summary", "unknown kind → keep en summary");
    }

    #[test]
    fn select_summary_returns_en_or_zh() {
        let i = insight_with(
            "series_jump",
            vec![ev("a", "f", 1.0, None), ev("b", "f", 2.0, None)],
            InsightSeverity::Warning,
        );
        assert_eq!(select_summary(&i, Language::En), "English summary");
        let zh = select_summary(&i, Language::ZhHk);
        assert!(zh.contains("變動"), "zh selected: {zh}");
    }

    #[test]
    fn frame_is_deterministic() {
        let i = insight_with(
            "series_jump",
            vec![ev("a", "f", 1.0, None), ev("b", "f", 2.0, None)],
            InsightSeverity::Warning,
        );
        let a = frame_zh_hk(&i);
        let b = frame_zh_hk(&i);
        assert_eq!(a, b, "same insight → identical zh-HK frame");
    }

    #[test]
    fn severity_translates() {
        let crit = insight_with("series_jump", vec![], InsightSeverity::Critical);
        let warn = insight_with("series_jump", vec![], InsightSeverity::Warning);
        let info = insight_with("series_jump", vec![], InsightSeverity::Info);
        assert!(frame_zh_hk(&crit).contains("嚴重"));
        assert!(frame_zh_hk(&warn).contains("警告"));
        assert!(frame_zh_hk(&info).contains("資訊"));
    }
}
