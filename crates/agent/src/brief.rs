//! The daily brief — the product layer (v9).
//!
//! [`build_brief`] ranks the currently-held insights into a prioritized
//! "morning brief": the handful of items a user should know about today. This
//! is the user-facing product the whole agent stack exists to feed.
//!
//! Ranking is deterministic and explainable: a composite score blending
//! severity, confidence, recency, and experimental-status (experimental
//! findings are discounted). No LLM — the brief is reproducible.

use crate::insight::{Insight, InsightSeverity, InsightStore};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One ranked item in the daily brief.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefItem {
    /// 1-based rank within the brief (1 = most important).
    pub rank: usize,
    /// The composite score (0–100, higher = more important).
    pub score: f64,
    #[serde(flatten)]
    pub insight: Insight,
}

/// A ranked daily brief.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    /// When this brief was generated.
    pub generated_at: DateTime<Utc>,
    /// The prioritized items, best first. Length <= `limit`.
    pub items: Vec<BriefItem>,
}

/// Build a daily brief from the insight store.
///
/// `limit` caps the number of items (default 5 — a brief, not a feed).
/// `now` is injected so tests are deterministic.
pub async fn build_brief(store: &Arc<InsightStore>, limit: usize, now: DateTime<Utc>) -> Brief {
    let limit = limit.clamp(1, 50);
    // Pull a generous pool, then rank + truncate.
    let pool = store.list(200).await;
    let mut scored: Vec<(f64, Insight)> = pool
        .into_iter()
        .map(|i| (score_insight(&i, now), i))
        .collect();
    // Sort by score desc, then by severity as a tiebreak, then by recency.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| severity_rank(&b.1.severity).cmp(&severity_rank(&a.1.severity)))
            .then_with(|| b.1.generated_at.cmp(&a.1.generated_at))
    });
    let items: Vec<BriefItem> = scored
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(idx, (score, insight))| BriefItem {
            rank: idx + 1,
            score: (score * 100.0).round(),
            insight,
        })
        .collect();
    Brief {
        generated_at: now,
        items,
    }
}

/// Severity → numeric weight for ranking.
fn severity_rank(s: &InsightSeverity) -> u8 {
    match s {
        InsightSeverity::Critical => 3,
        InsightSeverity::Warning => 2,
        InsightSeverity::Info => 1,
    }
}

/// Composite score in 0..=1. Blends:
/// - severity weight (50%): critical ≈ 1.0, warning ≈ 0.6, info ≈ 0.3
/// - confidence (25%): the detector/LLM's own confidence
/// - recency (25%): decays over 7 days (1.0 at now → 0 at +7d)
///
/// Experimental findings are discounted ×0.7 to demote unvalidated detectors.
fn score_insight(insight: &Insight, now: DateTime<Utc>) -> f64 {
    let sev_weight = match insight.severity {
        InsightSeverity::Critical => 1.0,
        InsightSeverity::Warning => 0.6,
        InsightSeverity::Info => 0.3,
    };
    let confidence = insight.confidence.clamp(0.0, 1.0);
    let age = now.signed_duration_since(insight.generated_at);
    let recency = if age < Duration::zero() {
        1.0
    } else {
        (1.0 - (age.num_seconds() as f64 / (7.0 * 24.0 * 3600.0))).max(0.0)
    };
    let raw = sev_weight * 0.5 + confidence * 0.25 + recency * 0.25;
    let discounted = if insight.experimental { raw * 0.7 } else { raw };
    discounted.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{EvidenceRef, InsightSeverity};
    use hkgov_common::DataSource;
    use serde_json::json;

    fn insight(id: &str, sev: InsightSeverity, confidence: f64, experimental: bool) -> Insight {
        Insight {
            id: id.into(),
            kind: "test".into(),
            severity: sev,
            title: format!("insight {id}"),
            summary: "s".into(),
            source: DataSource::Hkma,
            dataset: "x".into(),
            evidence: vec![EvidenceRef {
                record_id: "r".into(),
                field: "f".into(),
                value: json!(1),
                context: None,
            }],
            confidence,
            generated_at: Utc::now(),
            producer: "test".into(),
            experimental,
            first_seen: None,
            version: 1,
            evolution: None,
        }
    }

    #[tokio::test]
    async fn brief_ranks_critical_above_info() {
        let store = Arc::new(InsightStore::new());
        store
            .upsert(insight("info-1", InsightSeverity::Info, 0.9, false))
            .await;
        store
            .upsert(insight("crit-1", InsightSeverity::Critical, 0.5, false))
            .await;
        let now = Utc::now();
        let brief = build_brief(&store, 5, now).await;
        assert_eq!(brief.items[0].insight.id, "crit-1");
        assert_eq!(brief.items[1].insight.id, "info-1");
        assert!(brief.items[0].score > brief.items[1].score);
    }

    #[tokio::test]
    async fn brief_discounts_experimental() {
        let store = Arc::new(InsightStore::new());
        // Two identical-severity insights; one experimental. Non-experimental wins.
        store
            .upsert(insight("exp", InsightSeverity::Warning, 0.8, true))
            .await;
        store
            .upsert(insight("val", InsightSeverity::Warning, 0.8, false))
            .await;
        let now = Utc::now();
        let brief = build_brief(&store, 5, now).await;
        assert_eq!(brief.items[0].insight.id, "val");
        assert_eq!(brief.items[1].insight.id, "exp");
    }

    #[tokio::test]
    async fn brief_respects_limit() {
        let store = Arc::new(InsightStore::new());
        for i in 0..10 {
            store
                .upsert(insight(&format!("i{i}"), InsightSeverity::Info, 0.5, false))
                .await;
        }
        let now = Utc::now();
        let brief = build_brief(&store, 3, now).await;
        assert_eq!(brief.items.len(), 3);
        assert_eq!(brief.items[0].rank, 1);
        assert_eq!(brief.items[2].rank, 3);
    }

    #[tokio::test]
    async fn brief_empty_when_no_insights() {
        let store = Arc::new(InsightStore::new());
        let brief = build_brief(&store, 5, Utc::now()).await;
        assert!(brief.items.is_empty());
    }
}
