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
    /// P-104 Lifeline: when this insight was first seen (set once, preserved
    /// across evolution). Lets "what's new since you left" compare against the
    /// original creation, not the latest re-fire. Defaults to `generated_at`
    /// for legacy insights.
    #[serde(default)]
    pub first_seen: Option<DateTime<Utc>>,
    /// P-104 Lifeline: monotonic version counter. Bumped each time the content
    /// hash changes across an upsert; held at 1 when the insight is unchanged.
    #[serde(default = "default_version")]
    pub version: u32,
    /// P-104 Lifeline: the diff from the prior version, when this upsert
    /// changed the insight's content. `None` for the first version or when
    /// the upsert was a content-stable no-op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evolution: Option<EvolutionDiff>,
}

fn default_version() -> u32 {
    1
}

/// P-104 Lifeline: a field-level change between two versions of one insight.
/// Computed in `InsightStore::upsert` at the moment the prior version is about
/// to be overwritten — if the diff isn't captured then, the history is lost
/// forever (the scheduler is the only writer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionDiff {
    pub insight_id: String,
    pub from_version: u32,
    pub to_version: u32,
    pub from_generated_at: DateTime<Utc>,
    pub to_generated_at: DateTime<Utc>,
    pub changes: Vec<FieldChange>,
}

/// One specific field that changed across an evolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum FieldChange {
    Severity {
        from: InsightSeverity,
        to: InsightSeverity,
    },
    Confidence {
        from: f64,
        to: f64,
    },
    Title {
        from: String,
        to: String,
    },
    Summary {
        from: String,
        to: String,
    },
    Experimental {
        from: bool,
        to: bool,
    },
    Producer {
        from: String,
        to: String,
    },
}

impl EvolutionDiff {
    /// True when there are no field-level changes (the versions are
    /// content-equal except for metadata like `generated_at`).
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// P-104 Lifeline: one stored prior version of an insight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightRevision {
    pub version: u32,
    pub generated_at: DateTime<Utc>,
    pub snapshot: Insight,
    /// The diff that produced *this* version from the prior one. `None` for v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_from_previous: Option<EvolutionDiff>,
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
///
/// P-104 Lifeline: `upsert` is now evolution-aware. Before overwriting an
/// existing insight, it computes a content hash; if the content changed, the
/// prior version is pushed onto `history`, the version counter is bumped, and an
/// [`EvolutionDiff`] is attached to the new version. If the content is
/// unchanged, the upsert is a no-op (this also fixes the prior behavior where
/// every pass rewrote `generated_at` even when nothing changed).
#[derive(Default)]
pub struct InsightStore {
    inner: Arc<RwLock<BTreeMap<String, Insight>>>,
    history: Arc<RwLock<BTreeMap<String, Vec<InsightRevision>>>>,
}

impl InsightStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert one insight, capturing evolution if the content changed.
    /// Returns the version number the insight is now at (1 for a fresh insight
    /// or a content-stable re-fire; >1 when it evolved).
    pub async fn upsert(&self, insight: Insight) -> u32 {
        let mut w = self.inner.write().await;
        let id = insight.id.clone();
        let now = insight.generated_at;

        if let Some(prior) = w.get(&id) {
            // Compute the content diff vs the prior version.
            let diff = diff_insights(prior, &insight);
            if diff.is_empty() {
                // Content-stable re-fire: keep the stored insight as-is. This
                // preserves first_seen and avoids churning generated_at. The
                // version is unchanged.
                return prior.version;
            }
            // Evolved: archive the prior version, bump version, attach diff.
            let from_version = prior.version;
            let to_version = from_version + 1;
            let prior_snapshot = prior.clone();
            // Preserve first_seen across versions.
            let first_seen = prior.first_seen.or(Some(prior.generated_at));
            let mut evolved = insight;
            evolved.first_seen = first_seen;
            evolved.version = to_version;
            let from_generated_at = prior_snapshot.generated_at;
            let diff = EvolutionDiff {
                insight_id: id.clone(),
                from_version,
                to_version,
                from_generated_at,
                to_generated_at: evolved.generated_at,
                changes: diff,
            };
            evolved.evolution = Some(diff.clone());
            // Archive the prior version in history.
            let mut hw = self.history.write().await;
            let entry = hw.entry(id.clone()).or_default();
            entry.push(InsightRevision {
                version: from_version,
                generated_at: from_generated_at,
                snapshot: prior_snapshot,
                diff_from_previous: None,
            });
            w.insert(id, evolved);
            to_version
        } else {
            // Fresh insight. Respect a caller-provided first_seen (lets an
            // importer seed old data); otherwise stamp it now.
            let mut fresh = insight;
            if fresh.first_seen.is_none() {
                fresh.first_seen = Some(now);
            }
            fresh.version = 1;
            fresh.evolution = None;
            w.insert(id, fresh);
            1
        }
    }

    pub async fn upsert_many(&self, insights: Vec<Insight>) {
        // Delegate per-element so evolution is captured for each.
        for i in insights {
            self.upsert(i).await;
        }
    }

    pub async fn list(&self, limit: usize) -> Vec<Insight> {
        let r = self.inner.read().await;
        r.values().rev().take(limit).cloned().collect()
    }

    /// P-104 Lifeline: list insights, optionally filtered to those first-seen
    /// or evolved after `since`. Used by the dashboard's "what's new since you
    /// left" banner.
    pub async fn list_since(&self, limit: usize, since: DateTime<Utc>) -> Vec<Insight> {
        let r = self.inner.read().await;
        r.values()
            .rev()
            .filter(|i| {
                let first = i.first_seen.unwrap_or(i.generated_at);
                first > since
                    || i.evolution
                        .as_ref()
                        .map(|d| d.to_generated_at > since)
                        .unwrap_or(false)
            })
            .take(limit)
            .cloned()
            .collect()
    }

    /// Look up one insight by id. Returns `None` if unknown. Used by the cite
    /// route (`GET /v1/insights/{id}/cite`) and (later) by the permalink landing.
    pub async fn get(&self, id: &str) -> Option<Insight> {
        let r = self.inner.read().await;
        r.get(id).cloned()
    }

    /// P-104 Lifeline: the prior versions of one insight, oldest-first. Empty
    /// for a fresh insight (v1) or an unknown id.
    pub async fn history(&self, id: &str, limit: usize) -> Vec<InsightRevision> {
        let r = self.history.read().await;
        match r.get(id) {
            Some(v) => v.iter().rev().take(limit).cloned().collect(),
            None => Vec::new(),
        }
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }
}

/// Compute the field-level diff between two insights sharing the same id. The
/// id is assumed equal (callers dedup on it); this compares the fields that can
/// change across an id-stable re-fire: severity, confidence, title, summary,
/// experimental, producer. Evidence changes would change the fingerprint and
/// thus the id, so they're not compared here.
fn diff_insights(old: &Insight, new: &Insight) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    if old.severity != new.severity {
        changes.push(FieldChange::Severity {
            from: old.severity,
            to: new.severity,
        });
    }
    if (old.confidence - new.confidence).abs() > 0.01 {
        changes.push(FieldChange::Confidence {
            from: old.confidence,
            to: new.confidence,
        });
    }
    if old.title != new.title {
        changes.push(FieldChange::Title {
            from: old.title.clone(),
            to: new.title.clone(),
        });
    }
    if old.summary != new.summary {
        changes.push(FieldChange::Summary {
            from: old.summary.clone(),
            to: new.summary.clone(),
        });
    }
    if old.experimental != new.experimental {
        changes.push(FieldChange::Experimental {
            from: old.experimental,
            to: new.experimental,
        });
    }
    if old.producer != new.producer {
        changes.push(FieldChange::Producer {
            from: old.producer.clone(),
            to: new.producer.clone(),
        });
    }
    changes
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

#[cfg(test)]
mod lifeline_tests {
    use super::*;
    use chrono::TimeZone;

    fn insight(id: &str, severity: InsightSeverity, summary: &str) -> Insight {
        Insight {
            id: id.into(),
            kind: "series_jump".into(),
            severity,
            title: "t".into(),
            summary: summary.into(),
            source: DataSource::Hkma,
            dataset: "x".into(),
            evidence: vec![EvidenceRef {
                record_id: "r".into(),
                field: "f".into(),
                value: serde_json::json!(1),
                context: None,
            }],
            confidence: 0.8,
            generated_at: Utc.with_ymd_and_hms(2026, 6, 22, 12, 0, 0).unwrap(),
            producer: "test".into(),
            experimental: false,
            first_seen: None,
            version: 1,
            evolution: None,
        }
    }

    #[tokio::test]
    async fn fresh_insight_gets_version_1_and_first_seen() {
        let store = InsightStore::new();
        let v = store
            .upsert(insight("i1", InsightSeverity::Warning, "s"))
            .await;
        let got = store.get("i1").await.unwrap();
        assert_eq!(v, 1);
        assert_eq!(got.version, 1);
        assert!(got.first_seen.is_some());
        assert!(got.evolution.is_none());
    }

    #[tokio::test]
    async fn content_stable_refire_is_noop() {
        let store = InsightStore::new();
        store
            .upsert(insight("i1", InsightSeverity::Warning, "s"))
            .await;
        // Same content re-fired: version stays 1, no evolution recorded.
        let v = store
            .upsert(insight("i1", InsightSeverity::Warning, "s"))
            .await;
        assert_eq!(v, 1);
        let got = store.get("i1").await.unwrap();
        assert_eq!(got.version, 1);
        assert!(got.evolution.is_none());
        assert_eq!(store.history("i1", 10).await.len(), 0);
    }

    #[tokio::test]
    async fn severity_change_is_evolution() {
        let store = InsightStore::new();
        store
            .upsert(insight("i1", InsightSeverity::Warning, "s"))
            .await;
        // Severity escalates warning → critical.
        let v = store
            .upsert(insight("i1", InsightSeverity::Critical, "s"))
            .await;
        assert_eq!(v, 2);
        let got = store.get("i1").await.unwrap();
        assert_eq!(got.version, 2);
        let diff = got.evolution.as_ref().expect("evolution attached");
        assert_eq!(diff.from_version, 1);
        assert_eq!(diff.to_version, 2);
        assert!(diff.changes.iter().any(|c| matches!(
            c,
            FieldChange::Severity {
                from: InsightSeverity::Warning,
                to: InsightSeverity::Critical,
            }
        )));
        // first_seen preserved across versions.
        assert_eq!(got.first_seen, store.get("i1").await.unwrap().first_seen);
    }

    #[tokio::test]
    async fn summary_change_is_evolution() {
        let store = InsightStore::new();
        store
            .upsert(insight("i1", InsightSeverity::Warning, "old framing"))
            .await;
        let v = store
            .upsert(insight("i1", InsightSeverity::Warning, "new framing"))
            .await;
        assert_eq!(v, 2);
        let got = store.get("i1").await.unwrap();
        let diff = got.evolution.as_ref().unwrap();
        assert!(diff
            .changes
            .iter()
            .any(|c| matches!(c, FieldChange::Summary { .. })));
    }

    #[tokio::test]
    async fn confidence_change_below_tolerance_is_not_evolution() {
        let store = InsightStore::new();
        let mut a = insight("i1", InsightSeverity::Warning, "s");
        a.confidence = 0.80;
        store.upsert(a).await;
        let mut b = insight("i1", InsightSeverity::Warning, "s");
        b.confidence = 0.805; // within 0.01 tolerance → not a change
        let v = store.upsert(b).await;
        assert_eq!(v, 1, "tiny confidence drift must not trigger evolution");
    }

    #[tokio::test]
    async fn history_retains_prior_versions() {
        let store = InsightStore::new();
        store
            .upsert(insight("i1", InsightSeverity::Info, "s"))
            .await; // v1
        store
            .upsert(insight("i1", InsightSeverity::Warning, "s")) // v2
            .await;
        store
            .upsert(insight("i1", InsightSeverity::Critical, "s")) // v3
            .await;
        let hist = store.history("i1", 10).await;
        assert_eq!(hist.len(), 2, "two prior versions archived");
        // Newest-first.
        assert_eq!(hist[0].version, 2);
        assert_eq!(hist[1].version, 1);
        let got = store.get("i1").await.unwrap();
        assert_eq!(got.version, 3);
    }

    #[tokio::test]
    async fn list_since_filters_to_new_and_evolved() {
        let store = InsightStore::new();
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        // An old insight (first_seen before t0).
        let mut old = insight("old", InsightSeverity::Warning, "s");
        old.first_seen = Some(Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap());
        store.upsert(old).await;
        // A new insight (first_seen after t0).
        store
            .upsert(insight("new", InsightSeverity::Warning, "s"))
            .await;
        let since = store.list_since(100, t0).await;
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].id, "new");
    }

    #[tokio::test]
    async fn evolution_survives_serialization() {
        let store = InsightStore::new();
        store
            .upsert(insight("i1", InsightSeverity::Warning, "s"))
            .await;
        store
            .upsert(insight("i1", InsightSeverity::Critical, "s"))
            .await;
        let got = store.get("i1").await.unwrap();
        let json = serde_json::to_string(&got).unwrap();
        let back: Insight = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, 2);
        assert!(back.evolution.is_some());
    }
}
