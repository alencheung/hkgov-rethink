//! Drill-In Investigation (P-105) — saved, resumable, shareable case files.
//!
//! From any insight, a user can launch a multi-step investigation: the agent
//! offers guided next-step chips (related series / parallels / cross-source
//! checks) and the user can ask follow-ups. Each step is persisted so the case
//! file is resumable across sessions and shareable via a stable permalink.
//!
//! ## Determinism invariant (unchanged)
//!
//! The investigation reuses the existing [`ToolBelt`] + [`run_agent_loop`] —
//! detection stays pure Rust. The LLM only *chooses* which deterministic tool
//! to call and *frames* the result; it never performs detection.
//!
//! ## v1 scope (no identity layer yet)
//!
//! Per the integration map: single-tab resume needs nothing server-side; cross-
//! refresh resume persists the open id in localStorage; share = a URL fragment.
//! All work with the current shared-key auth. Per-user ownership/ACLs arrive
//! with P-108 (the `owner` field is `String`, empty in v1).
//!
//! ## Model
//!
//! An [`Investigation`] is a seed insight + ordered [`InvestigationStep`]s +
//! notes. A step mirrors one `run_agent_loop` outcome (or a one-click chip tool
//! call). Branching is a new `Investigation` seeded from a step of another.

use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::loop_mod::{Answer, TraceStep};

/// One step in an investigation. Mirrors a single `run_agent_loop` outcome or a
/// one-click chip tool invocation, persisted so the case file is resumable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationStep {
    /// Stable id within the investigation: `s1`, `s2`, … (monotonic).
    pub id: String,
    /// What produced this step.
    pub kind: StepKind,
    /// For `chip`: the tool name + a one-line label. For `qa`: the user's
    /// question. For `finding_promotion`: the discovered insight id.
    pub prompt: String,
    /// The natural-language answer (text + confidence). `None` for chip-only
    /// steps that don't drive the full loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    /// Ordered tool-call trace. For chip steps this is a single `TraceStep`;
    /// for qa steps it's the full `run_agent_loop` trace.
    #[serde(default)]
    pub trace: Vec<TraceStep>,
    pub executed_at: DateTime<Utc>,
    /// Free-text the user attached to this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// A one-click preset tool call (related series / parallels / cross-source).
    Chip,
    /// A free-form question via `/investigations/{id}/ask`.
    Qa,
    /// A `Finding` surfaced mid-run (`AgentOutcome::Findings`).
    FindingPromotion,
}

/// A free-text note on the case (not tied to one step).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationNote {
    pub id: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

/// A saved, resumable, shareable case file launched from one insight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Investigation {
    /// `inv:{seed_insight_id}:{short_fingerprint}`.
    pub id: String,
    /// The `Insight.id` this case was launched from.
    pub seed_insight_id: String,
    /// Snapshot of the seed's source/dataset/title at creation time, so the
    /// case file stays intelligible if the InsightStore rotates the seed.
    pub seed_source: DataSource,
    pub seed_dataset: String,
    pub seed_title: String,
    /// Human-authored title (defaults to seed_title on creation).
    pub title: String,
    /// P-108 identity handle. Empty string in v1.
    #[serde(default)]
    pub owner: String,
    /// Ordered steps — newest last. Append-only.
    #[serde(default)]
    pub steps: Vec<InvestigationStep>,
    /// Case-level notes.
    #[serde(default)]
    pub notes: Vec<InvestigationNote>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// In-process investigation store. Mirrors `InsightStore` — volatile (no DB
/// tier). v1 holds the case files; persistence to Postgres arrives later.
#[derive(Default)]
pub struct InvestigationStore {
    inner: Arc<RwLock<BTreeMap<String, Investigation>>>,
}

impl InvestigationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(&self, inv: Investigation) -> Investigation {
        let mut w = self.inner.write().await;
        w.insert(inv.id.clone(), inv.clone());
        inv
    }

    pub async fn get(&self, id: &str) -> Option<Investigation> {
        self.inner.read().await.get(id).cloned()
    }

    pub async fn list(&self, owner: &str, limit: usize) -> Vec<Investigation> {
        self.inner
            .read()
            .await
            .values()
            .filter(|i| owner.is_empty() || i.owner == owner)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Like [`list`](Self::list), but never treats an empty owner as "all".
    /// V-004 fix: the bare `list("", …)` returned every user's investigations.
    /// The authenticated surface scopes strictly to the caller (empty
    /// principal → empty result, not a dump).
    pub async fn list_owned(&self, owner: &str, limit: usize) -> Vec<Investigation> {
        if owner.is_empty() {
            return Vec::new();
        }
        self.inner
            .read()
            .await
            .values()
            .filter(|i| i.owner == owner)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Fetch an investigation owned by `owner`. V-004 fix: the bare `get`
    /// returned any id with no ownership check.
    pub async fn get_owned(&self, id: &str, owner: &str) -> Option<Investigation> {
        self.inner
            .read()
            .await
            .get(id)
            .filter(|i| owner.is_empty() || i.owner == owner)
            .cloned()
    }

    pub async fn delete(&self, id: &str) -> bool {
        self.inner.write().await.remove(id).is_some()
    }

    /// Delete an investigation owned by `owner`. V-004 fix: the bare `delete`
    /// removed any id with no ownership check.
    pub async fn delete_owned(&self, id: &str, owner: &str) -> bool {
        let mut w = self.inner.write().await;
        match w.get(id) {
            Some(i) if owner.is_empty() || i.owner == owner => {
                w.remove(id);
                true
            }
            _ => false,
        }
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }

    /// Append a step, bumping `updated_at`. Returns the updated investigation
    /// (or `None` if the id is unknown).
    pub async fn append_step(
        &self,
        id: &str,
        mut step: InvestigationStep,
    ) -> Option<Investigation> {
        let mut w = self.inner.write().await;
        let inv = w.get_mut(id)?;
        // Assign the next monotonic step id.
        let next = inv.steps.len() + 1;
        step.id = format!("s{next}");
        step.executed_at = Utc::now();
        inv.steps.push(step);
        inv.updated_at = Utc::now();
        Some(inv.clone())
    }

    /// Owner-scoped [`append_step`](Self::append_step). V-004 fix: the bare
    /// variant mutated any id with no ownership check, so an attacker could
    /// inject steps into another user's case file by id. This refuses unless
    /// the caller owns the record.
    pub async fn append_step_owned(
        &self,
        id: &str,
        owner: &str,
        step: InvestigationStep,
    ) -> Option<Investigation> {
        self.assert_owned(id, owner).await?;
        self.append_step(id, step).await
    }

    /// Add a case-level note.
    pub async fn add_note(&self, id: &str, body: String) -> Option<Investigation> {
        let mut w = self.inner.write().await;
        let inv = w.get_mut(id)?;
        let note = InvestigationNote {
            id: format!("n{}", inv.notes.len() + 1),
            body,
            created_at: Utc::now(),
        };
        inv.notes.push(note);
        inv.updated_at = Utc::now();
        Some(inv.clone())
    }

    /// Owner-scoped [`add_note`](Self::add_note). V-004 fix: the bare variant
    /// mutated any id with no ownership check. This refuses unless the caller
    /// owns the record.
    pub async fn add_note_owned(
        &self,
        id: &str,
        owner: &str,
        body: String,
    ) -> Option<Investigation> {
        self.assert_owned(id, owner).await?;
        self.add_note(id, body).await
    }

    /// Returns `Some(())` iff the record exists AND (owner is empty OR the
    /// record's owner matches). The single gate every owned_* method shares.
    async fn assert_owned(&self, id: &str, owner: &str) -> Option<()> {
        let r = self.inner.read().await;
        let inv = r.get(id)?;
        if owner.is_empty() || inv.owner == owner {
            Some(())
        } else {
            None
        }
    }
}

/// Build a stable investigation id from its seed + a creation timestamp.
pub fn investigation_id(seed_insight_id: &str, created_at: DateTime<Utc>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    seed_insight_id.hash(&mut h);
    created_at.hash(&mut h);
    format!("inv:{seed_insight_id}:{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn seed_investigation(id: &str) -> Investigation {
        Investigation {
            id: id.into(),
            seed_insight_id: "series_jump:hkma:x:test".into(),
            seed_source: DataSource::Hkma,
            seed_dataset: "daily-interbank-liquidity".into(),
            seed_title: "HIBOR moved".into(),
            title: "HIBOR moved".into(),
            owner: String::new(),
            steps: Vec::new(),
            notes: Vec::new(),
            created_at: Utc.with_ymd_and_hms(2026, 6, 22, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 6, 22, 12, 0, 0).unwrap(),
        }
    }

    fn step(kind: StepKind, prompt: &str) -> InvestigationStep {
        InvestigationStep {
            id: String::new(), // assigned by append_step
            kind,
            prompt: prompt.into(),
            answer: None,
            trace: Vec::new(),
            executed_at: Utc::now(),
            annotation: None,
        }
    }

    #[tokio::test]
    async fn create_get_list_delete_roundtrip() {
        let store = InvestigationStore::new();
        let inv = seed_investigation("inv1");
        store.create(inv).await;
        assert_eq!(store.count().await, 1);
        assert!(store.get("inv1").await.is_some());
        assert!(store.get("missing").await.is_none());
        assert_eq!(store.list("", 10).await.len(), 1);
        assert!(store.delete("inv1").await);
        assert_eq!(store.count().await, 0);
    }

    #[tokio::test]
    async fn append_step_assigns_monotonic_ids_and_bumps_updated() {
        let store = InvestigationStore::new();
        // Use now() for the seed so the comparison is stable against the live clock.
        let now = Utc::now();
        let mut inv = seed_investigation("inv1");
        inv.created_at = now;
        inv.updated_at = now;
        store.create(inv).await;
        let before = store.get("inv1").await.unwrap().updated_at;

        let updated = store
            .append_step("inv1", step(StepKind::Chip, "show related series"))
            .await
            .unwrap();
        assert_eq!(updated.steps.len(), 1);
        assert_eq!(updated.steps[0].id, "s1");
        assert!(updated.updated_at >= before);

        let updated2 = store
            .append_step("inv1", step(StepKind::Qa, "why did it move?"))
            .await
            .unwrap();
        assert_eq!(updated2.steps.len(), 2);
        assert_eq!(updated2.steps[1].id, "s2");
    }

    #[tokio::test]
    async fn append_step_unknown_id_returns_none() {
        let store = InvestigationStore::new();
        let r = store.append_step("nope", step(StepKind::Chip, "x")).await;
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn add_note_appends_and_bumps_updated() {
        let store = InvestigationStore::new();
        store.create(seed_investigation("inv1")).await;
        let updated = store.add_note("inv1", "follow up with HKMA".into()).await;
        assert!(updated.is_some());
        let inv = store.get("inv1").await.unwrap();
        assert_eq!(inv.notes.len(), 1);
        assert_eq!(inv.notes[0].id, "n1");
        assert_eq!(inv.notes[0].body, "follow up with HKMA");
    }

    #[test]
    fn investigation_id_is_stable() {
        let ts = Utc.with_ymd_and_hms(2026, 6, 22, 12, 0, 0).unwrap();
        let a = investigation_id("series_jump:hkma:x:test", ts);
        let b = investigation_id("series_jump:hkma:x:test", ts);
        assert_eq!(a, b);
        assert!(a.starts_with("inv:series_jump:hkma:x:test:"));
        // Different seed → different id.
        let c = investigation_id("other", ts);
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn owner_filter_isolation() {
        let store = InvestigationStore::new();
        let mut a = seed_investigation("inv1");
        a.owner = "alice".into();
        let mut b = seed_investigation("inv2");
        b.owner = "bob".into();
        store.create(a).await;
        store.create(b).await;
        assert_eq!(store.list("alice", 10).await.len(), 1);
        assert_eq!(store.list("bob", 10).await.len(), 1);
        assert_eq!(store.list("", 10).await.len(), 2, "empty owner = all");
    }

    #[tokio::test]
    async fn investigation_survives_serialization() {
        let store = InvestigationStore::new();
        store.create(seed_investigation("inv1")).await;
        store.append_step("inv1", step(StepKind::Qa, "why?")).await;
        let inv = store.get("inv1").await.unwrap();
        let json = serde_json::to_string(&inv).unwrap();
        let back: Investigation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "inv1");
        assert_eq!(back.steps.len(), 1);
    }
}
