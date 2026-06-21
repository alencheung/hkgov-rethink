//! Insight model — the normalized output of every agent analysis pass.
//!
//! Insights are first-class records: they get stored, served, and (in v5)
//! streamed to a dashboard. They are NOT raw LLM text blobs — each one carries
//! structured evidence so a reader can verify the claim against the source data.

use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// How strongly the agent believes this finding matters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InsightSeverity {
    /// Informational — a notable observation with no anomaly.
    Info,
    /// Something moved beyond normal bounds.
    Warning,
    /// A cross-source divergence: the data and the official narrative disagree,
    /// or a metric is far outside its historical range.
    Critical,
}

impl std::fmt::Display for InsightSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsightSeverity::Info => f.write_str("info"),
            InsightSeverity::Warning => f.write_str("warning"),
            InsightSeverity::Critical => f.write_str("critical"),
        }
    }
}

/// A single insight produced by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    /// Stable id: `{kind}:{source}:{dataset}:{fingerprint}`.
    pub id: String,
    /// Human-readable category, e.g. `series_jump`, `cross_source_gap`.
    pub kind: String,
    pub severity: InsightSeverity,
    pub title: String,
    /// Plain-language explanation. When an LLM is configured this is the
    /// model's framing; otherwise it's the heuristic's templated summary.
    pub summary: String,
    /// The (source, dataset) the evidence was drawn from.
    pub source: DataSource,
    pub dataset: String,
    /// Pointers back into the record store: record_ids + the field(s) that
    /// triggered the finding, with their values.
    pub evidence: Vec<EvidenceRef>,
    /// 0–1 confidence. Heuristics emit deterministic scores; LLM framing may
    /// adjust.
    pub confidence: f64,
    pub generated_at: DateTime<Utc>,
    /// Which client produced this: `heuristic` or `llm`.
    pub producer: String,
    /// True when the producing detector is experimental (v6/v7 detectors not
    /// yet validated on real data — see EXAMPLES.md). Surfaced so the UI can
    /// badge it and users can discount it. Defaults false (back-compat).
    #[serde(default)]
    pub experimental: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub record_id: String,
    pub field: String,
    pub value: serde_json::Value,
    pub context: Option<String>,
}

/// In-process insight store. v3 keeps it simple — an append+replace-by-id map
/// guarded by a RwLock. The serving API reads from here. Persisting insights to
/// the same RecordStore as raw data is a v4 task.
#[derive(Default)]
pub struct InsightStore {
    inner: Arc<RwLock<BTreeMap<String, Insight>>>,
}

impl InsightStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn upsert(&self, insight: Insight) {
        let mut w = self.inner.write().await;
        w.insert(insight.id.clone(), insight);
    }

    pub async fn upsert_many(&self, insights: Vec<Insight>) {
        let mut w = self.inner.write().await;
        for i in insights {
            w.insert(i.id.clone(), i);
        }
    }

    pub async fn list(&self, limit: usize) -> Vec<Insight> {
        let r = self.inner.read().await;
        r.values().rev().take(limit).cloned().collect()
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }
}

/// One user feedback signal on an insight. The cheapest possible success
/// metric: did this insight help, or not?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    pub insight_id: String,
    /// `up` = useful, `down` = not useful. The simplest possible signal.
    pub useful: bool,
    /// Optional free-text reason (for the "down" case especially).
    pub note: Option<String>,
    pub submitted_at: DateTime<Utc>,
}

/// In-process feedback store. Counts up/down per insight id so the brief
/// ranker (and product analytics) can learn what users value.
#[derive(Default)]
pub struct FeedbackStore {
    inner: Arc<RwLock<Vec<Feedback>>>,
}

impl FeedbackStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one feedback signal. Idempotent at the store level (each submit
    /// is a new row — callers may dedup by source IP/session upstream).
    pub async fn record(&self, feedback: Feedback) {
        self.inner.write().await.push(feedback);
    }

    /// Net usefulness (up − down) for one insight. Negative = users found it
    /// unhelpful more often than helpful.
    pub async fn net_useful(&self, insight_id: &str) -> i64 {
        self.inner
            .read()
            .await
            .iter()
            .filter(|f| f.insight_id == insight_id)
            .map(|f| if f.useful { 1 } else { -1 })
            .sum()
    }

    /// Total feedback count (all insights).
    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }

    /// All feedback, most recent first (for analytics export).
    pub async fn list(&self, limit: usize) -> Vec<Feedback> {
        self.inner
            .read()
            .await
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}
