//! Responsible-AI provenance — M3 (Responsible AI Audit Layer).
//!
//! Every insight the agent produces is backed by a `ProvenanceRecord` that
//! attests *how* it was made: which detector, what threshold, the SHA-256 of
//! the exact input evidence, the detector version, the runtime version, who
//! framed it (heuristic vs a specific LLM model), and — critically — whether
//! it is **deterministic-reproducible** (originated from a pure-Rust detector)
//! or **LLM-framed** (the LLM selected/framed but detection stayed in Rust).
//!
//! This is the typed, checkable form of the determinism guarantee that
//! `docs/ARCHITECTURE.md` describes informally ("the LLM never performs
//! detection; it only selects and frames"). The `deterministic` flag on
//! `Finding` is set only inside `analysis.rs`; the LLM framing path cannot
//! flip it. So a regulator or researcher querying `/v1/audit?deterministic=true`
//! gets findings they can recompute in CI, and `/v1/audit?producer=llm:*` gets
//! the ones where an LLM was in the loop.
//!
//! ## Sidecar, not a field on Insight
//!
//! Mirrors M1's lineage pattern and cite.rs's manifest pattern: provenance is
//! keyed by `insight.id` in a sidecar `ProvenanceStore`, not bolted onto the
//! `Insight` struct. The `Insight.producer` string stays as the existing
//! backward-compat tag; the full audit trail lives here.
//!
//! ## Hash consistency
//!
//! `input_sha256` reuses cite.rs's `evidence_hash` (NaN/Inf-safe, drift-
//! detecting). The provenance hash and the cite-manifest hash are therefore
//! guaranteed to agree on the same insight — a cross-module invariant the
//! `provenance_hash_matches_cite_hash` test enforces.

use crate::cite::reproducibility_hash;
use crate::insight::{EvidenceRef, Insight};
use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// The version of the detector layer as a whole. Re-exported from
/// `analysis::DETECTOR_VERSION` (the single source of truth — that's where the
/// detectors live). Recorded on every `ProvenanceRecord` so a reviewer can tell
/// whether a re-run would reproduce against the same detector version.
pub use crate::analysis::DETECTOR_VERSION;

/// The version of this provenance schema itself. Bumped if the
/// `ProvenanceRecord` shape changes in a backward-incompatible way.
pub const PROVENANCE_VERSION: &str = "1.0";

/// How a finding was produced. `Heuristic` is the deterministic baseline (no
/// LLM in the loop); `Llm` records the model id so the audit trail can answer
/// "which model framed this?".
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Producer {
    /// Pure-Rust heuristic framing — the `HeuristicClient` default. Fully
    /// deterministic; re-running with the same inputs reproduces byte-identical
    /// output.
    Heuristic,
    /// An OpenAI-compatible LLM selected/framed the finding. Detection still
    /// originated in a pure-Rust detector (the determinism guarantee), but the
    /// *framing* (summary text, confidence) came from `model`.
    Llm {
        /// The model id (e.g. "gpt-4o-mini"), for the audit trail.
        model: String,
    },
}

impl Producer {
    /// Parse the scheduler's `llm.name()` string (the existing `Insight.producer`
    /// value) into the richer `Producer` enum. "heuristic" → `Heuristic`;
    /// anything else is treated as an LLM with that name as the model id.
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("heuristic") {
            Self::Heuristic
        } else {
            Self::Llm {
                model: name.to_string(),
            }
        }
    }

    /// The string form for the existing `Insight.producer` field (back-compat).
    pub fn as_name(&self) -> String {
        match self {
            Self::Heuristic => "heuristic".to_string(),
            Self::Llm { model } => model.clone(),
        }
    }
}

/// The full audit trail for one insight: the exact recipe that produced it.
///
/// `input_sha256` is the SHA-256 over the insight's evidence refs (the same
/// canonical, NaN/Inf-safe hash cite.rs computes for the reproducibility
/// manifest). A reviewer re-runs the detector against current data; if the
/// recomputed hash matches, the finding reproduces.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceRecord {
    pub provenance_version: &'static str,
    pub insight_id: String,
    /// The detector kind (matches `Insight.kind` / `Finding.kind`).
    pub detector: String,
    pub source: DataSource,
    pub dataset: String,
    /// The threshold the detector applied, recovered from the evidence refs
    /// (the same logic cite.rs::derive_threshold uses). None when the detector
    /// isn't threshold-based.
    pub threshold: Option<f64>,
    /// SHA-256 over the evidence refs — the reproducibility anchor. Matches the
    /// cite manifest's evidence hash byte-for-byte (cross-module invariant).
    pub input_sha256: String,
    /// The detector-layer version (DETECTOR_VERSION). Changes here mean a
    /// re-run may produce a different finding.
    pub detector_version: &'static str,
    /// The runtime (hkgov-api) version, from CARGO_PKG_VERSION at build time.
    pub runtime_version: String,
    /// Who framed the finding — heuristic (deterministic) or a specific LLM.
    pub producer: Producer,
    /// The determinism attestation: true iff the finding originated from a
    /// pure-Rust detector in analysis.rs. The LLM framing path cannot set this
    /// (it's a sealed field on Finding). This is the typed, checkable form of
    /// the determinism guarantee.
    pub deterministic: bool,
    pub produced_at: DateTime<Utc>,
}

impl ProvenanceRecord {
    /// Build a provenance record for an insight + the producer that framed it.
    /// `deterministic` comes from `Finding.deterministic` (set inside
    /// analysis.rs only); the evidence hash is recomputed here from the
    /// insight's evidence refs so it always reflects what was stored.
    pub fn for_insight(insight: &Insight, producer: Producer, deterministic: bool) -> Self {
        let threshold = derive_threshold(&insight.evidence);
        Self {
            provenance_version: PROVENANCE_VERSION,
            insight_id: insight.id.clone(),
            detector: insight.kind.clone(),
            source: insight.source,
            dataset: insight.dataset.clone(),
            threshold,
            input_sha256: reproducibility_hash(&insight.evidence),
            detector_version: DETECTOR_VERSION,
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            producer,
            deterministic,
            produced_at: insight.generated_at,
        }
    }
}

/// Recover the detector threshold from evidence refs. Mirrors cite.rs's
/// `derive_threshold`: the evidence ref whose `context` mentions "threshold"
/// carries the watch line as its value.
pub(crate) fn derive_threshold(evidence: &[EvidenceRef]) -> Option<f64> {
    for e in evidence {
        if e.context.as_deref().unwrap_or("").contains("threshold") {
            if let Some(f) = e.value.as_f64() {
                return Some(f);
            }
        }
    }
    None
}

/// Sidecar provenance store, keyed by `insight.id`. Volatile; mirrors the
/// InsightStore storage shape (in-process map + snapshot/restore) so it
/// persists the same way the other agent-layer stores do.
#[derive(Debug, Default, Clone)]
pub struct ProvenanceStore {
    inner: Arc<RwLock<HashMap<String, ProvenanceRecord>>>,
}

impl ProvenanceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record provenance for an insight. Called by the scheduler right after
    /// `insights.upsert()`. Overwrites any prior record for the same insight id
    /// (a re-fire with changed evidence supersedes the prior attestation — the
    /// hash will differ, which the audit log makes visible).
    pub async fn record(&self, rec: ProvenanceRecord) {
        let mut inner = self.inner.write().await;
        inner.insert(rec.insight_id.clone(), rec);
    }

    /// Look up provenance for an insight. None if no provenance was recorded
    /// (e.g. an insight loaded from a snapshot taken before M3 shipped).
    pub async fn get(&self, insight_id: &str) -> Option<ProvenanceRecord> {
        let inner = self.inner.read().await;
        inner.get(insight_id).cloned()
    }

    /// Snapshot for persistence (same shape as the other agent stores).
    pub async fn snapshot(&self) -> Vec<ProvenanceRecord> {
        let inner = self.inner.read().await;
        let mut out: Vec<ProvenanceRecord> = inner.values().cloned().collect();
        out.sort_by(|a, b| a.insight_id.cmp(&b.insight_id));
        out
    }

    /// Restore from a snapshot (boot-time).
    pub async fn restore(&self, snapshot: Vec<ProvenanceRecord>) {
        let mut inner = self.inner.write().await;
        inner.clear();
        for rec in snapshot {
            inner.insert(rec.insight_id.clone(), rec);
        }
    }
}

/// An attestation bundle: the insight + its provenance + the cite reproducibility
/// manifest, asserting "this finding is deterministic-reproducible; here is the
/// recipe + hash." Surfaced via `GET /v1/audit/attestation/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct Attestation {
    pub attestation_version: &'static str,
    pub insight: Insight,
    pub provenance: ProvenanceRecord,
    /// The cite.rs reproducibility manifest (detector + threshold + data hash).
    /// `None` only when the insight's evidence records aren't held in the store
    /// (cold cache) — the provenance hash alone still attests reproducibility.
    pub reproducibility_manifest: Option<crate::cite::ReproducibilityManifest>,
    /// The attestation claim itself, in plain text, for a human reviewer.
    pub claim: String,
}

pub const ATTESTATION_VERSION: &str = "1.0";

/// Build an attestation for an insight. Reads the provenance sidecar + computes
/// the cite manifest (if the evidence records are available). The `claim` text
/// is the honest summary: deterministic findings assert byte-reproducibility;
/// LLM-framed findings assert detection-is-deterministic-but-framing-is-not.
pub async fn build_attestation(
    insight: &Insight,
    provenance_store: &ProvenanceStore,
    cite_manifest: Option<crate::cite::ReproducibilityManifest>,
) -> Option<Attestation> {
    let provenance = provenance_store.get(&insight.id).await?;
    let claim = if provenance.deterministic {
        format!(
            "Finding {} is deterministic-reproducible: produced by the pure-Rust '{}' detector \
             (v{}) over {}/{} with evidence hash {}. Re-running the detector against the same \
             input data reproduces the finding byte-for-byte.",
            insight.id,
            provenance.detector,
            provenance.detector_version,
            provenance.source,
            provenance.dataset,
            provenance.input_sha256,
        )
    } else {
        format!(
            "Finding {} was framed by an LLM ({}). Detection originated in the pure-Rust '{}' \
             detector, but the summary/confidence framing is model-dependent and not guaranteed \
             to reproduce byte-for-byte. The evidence hash {} covers the deterministic input.",
            insight.id,
            provenance.producer.as_name(),
            provenance.detector,
            provenance.input_sha256,
        )
    };
    Some(Attestation {
        attestation_version: ATTESTATION_VERSION,
        insight: insight.clone(),
        provenance,
        reproducibility_manifest: cite_manifest,
        claim,
    })
}

/// Query filters for the audit-log endpoint. All optional; compose with AND.
/// `deterministic=true` returns only deterministic-reproducible findings;
/// `deterministic=false` returns only LLM-framed ones; unset returns both.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Only records produced at/after this timestamp.
    pub since: Option<DateTime<Utc>>,
    /// Only records from this producer (heuristic, or an LLM model id).
    /// Substring match on the producer name.
    pub producer: Option<String>,
    /// Filter by the determinism flag.
    pub deterministic: Option<bool>,
}

/// Apply the audit filters to a snapshot of provenance records. Sorted newest-
/// first by `produced_at` (the audit log reads top-down).
pub fn filter_audit(records: Vec<ProvenanceRecord>, q: &AuditQuery) -> Vec<ProvenanceRecord> {
    let mut out: Vec<ProvenanceRecord> = records
        .into_iter()
        .filter(|r| q.since.is_none_or(|s| r.produced_at >= s))
        .filter(|r| {
            q.deterministic
                .is_none_or(|d| r.deterministic == d)
        })
        .filter(|r| {
            q.producer
                .as_deref()
                .is_none_or(|p| r.producer.as_name().contains(p))
        })
        .collect();
    out.sort_by(|a, b| b.produced_at.cmp(&a.produced_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{Insight, InsightSeverity};
    use chrono::Utc;
    use hkgov_common::DataSource;
    use serde_json::json;

    fn make_insight(id: &str, kind: &str, evidence: Vec<EvidenceRef>) -> Insight {
        Insight {
            id: id.into(),
            kind: kind.into(),
            severity: InsightSeverity::Warning,
            title: "test".into(),
            summary: "test summary".into(),
            source: DataSource::Hkma,
            dataset: "test-dataset".into(),
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

    #[tokio::test]
    async fn provenance_record_carries_evidence_hash_and_threshold() {
        let evidence = vec![
            EvidenceRef {
                record_id: "2026-02-16".into(),
                field: "hibor_overnight".into(),
                value: json!(2.93),
                context: Some("the move".into()),
            },
            EvidenceRef {
                record_id: "threshold".into(),
                field: "pct_threshold".into(),
                value: json!(25.0),
                context: Some("threshold applied".into()),
            },
        ];
        let insight = make_insight("series_jump:hkma:d:abc", "series_jump", evidence);
        let rec = ProvenanceRecord::for_insight(&insight, Producer::Heuristic, true);
        assert_eq!(rec.detector, "series_jump");
        assert_eq!(rec.threshold, Some(25.0));
        assert!(rec.deterministic);
        assert!(!rec.input_sha256.is_empty());
        assert_eq!(rec.detector_version, DETECTOR_VERSION);
    }

    #[tokio::test]
    async fn provenance_hash_matches_cite_evidence_hash() {
        // Cross-module invariant: the provenance input_sha256 and the cite
        // reproducibility hash must agree on the same evidence. Both reuse the
        // same canonical, NaN/Inf-safe hashing.
        let evidence = vec![
            EvidenceRef {
                record_id: "2026-02-16".into(),
                field: "hibor_overnight".into(),
                value: json!(2.93),
                context: None,
            },
            EvidenceRef {
                record_id: "2026-02-13".into(),
                field: "hibor_overnight".into(),
                value: json!(1.47),
                context: None,
            },
        ];
        let insight = make_insight("sj:hkma:d:h", "series_jump", evidence.clone());
        let rec = ProvenanceRecord::for_insight(&insight, Producer::Heuristic, true);
        let cite_hash = reproducibility_hash(&evidence);
        assert_eq!(
            rec.input_sha256, cite_hash,
            "provenance + cite hashes must agree (cross-module invariant)"
        );
    }

    #[tokio::test]
    async fn provenance_store_record_get_restore() {
        let store = ProvenanceStore::new();
        let insight = make_insight("id1", "series_jump", vec![]);
        let rec = ProvenanceRecord::for_insight(&insight, Producer::Heuristic, true);
        store.record(rec.clone()).await;
        let got = store.get("id1").await.expect("recorded");
        assert_eq!(got.detector, "series_jump");

        let snap = store.snapshot().await;
        assert_eq!(snap.len(), 1);
        let restored = ProvenanceStore::new();
        restored.restore(snap).await;
        assert!(restored.get("id1").await.is_some());
    }

    #[tokio::test]
    async fn audit_filter_deterministic_flag() {
        let store = ProvenanceStore::new();
        // One deterministic, one LLM-framed.
        let i1 = make_insight("det", "series_jump", vec![]);
        let i2 = make_insight("llm", "series_jump", vec![]);
        store
            .record(ProvenanceRecord::for_insight(&i1, Producer::Heuristic, true))
            .await;
        store
            .record(ProvenanceRecord::for_insight(
                &i2,
                Producer::Llm {
                    model: "gpt-4o-mini".into(),
                },
                false,
            ))
            .await;

        let all = store.snapshot().await;
        let det_only = filter_audit(all.clone(), &AuditQuery {
            deterministic: Some(true),
            ..Default::default()
        });
        assert_eq!(det_only.len(), 1);
        assert_eq!(det_only[0].insight_id, "det");

        let llm_only = filter_audit(all, &AuditQuery {
            producer: Some("gpt-4o".into()),
            ..Default::default()
        });
        assert_eq!(llm_only.len(), 1);
        assert_eq!(llm_only[0].insight_id, "llm");
    }

    #[tokio::test]
    async fn producer_from_name_round_trips() {
        assert!(matches!(
            Producer::from_name("heuristic"),
            Producer::Heuristic
        ));
        assert!(matches!(
            Producer::from_name("HEURISTIC"),
            Producer::Heuristic
        ));
        match Producer::from_name("gpt-4o-mini") {
            Producer::Llm { model } => assert_eq!(model, "gpt-4o-mini"),
            _ => panic!("expected Llm"),
        }
    }
}
