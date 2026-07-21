//! Shared application state handed to every handler via axum's `State` extractor.

use crate::daily_view::DailyViewSlot;
use hkgov_agent::{
    AlertLog, FeedbackStore, InsightStore, InvestigationStore, LlmClient, MagicLinkDelivery,
    ProvenanceStore, SignalStore, UserStore,
};
use hkgov_common::Settings;
use hkgov_connectors::registry::Registry;
use hkgov_store::MemoryStore;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    /// Reserved for the on-demand refresh + AI-agent endpoints (ROADMAP v2).
    /// Read by future routes; kept here so AppState stays a single shared handle.
    #[allow(dead_code)]
    pub registry: Arc<Registry>,
    pub store: Arc<MemoryStore>,
    pub insights: Arc<InsightStore>,
    /// M3 Responsible AI Audit Layer: provenance sidecar. Every insight the
    /// agent produces gets a ProvenanceRecord here (detector, threshold,
    /// evidence hash, producer, deterministic flag). Surfaced via the
    /// /v1/insights/{id}/provenance, /v1/audit, /v1/audit/attestation routes.
    pub provenance: Arc<ProvenanceStore>,
    /// User feedback (was-this-useful) — the cheapest success metric. v9.
    pub feedback: Arc<FeedbackStore>,
    /// P-102 Signal subscriptions (authoring + preview; push waits on P-108).
    pub signals: Arc<SignalStore>,
    /// P-105 Drill-In Investigations (saved, resumable case files).
    pub investigations: Arc<InvestigationStore>,
    /// P-108 Identity tier (email + magic-link). The principal the per-user
    /// features (signals/investigations/read-state) key on as `owner`.
    pub users: Arc<UserStore>,
    /// The agent's LLM client. Used by `POST /v1/ask` to drive the agent loop.
    /// The periodic supervisor owns its own clone. Heuristic by default; HTTP
    /// when the `llm` feature + a configured base URL are present.
    pub llm: Arc<dyn LlmClient>,
    /// Dispatch log for proactive alerting (always present; empty when alerts
    /// are disabled). Exposed via `GET /v1/alerts`.
    pub alert_log: Arc<AlertLog>,
    /// Magic-link email delivery sink. Log-based by default (dev/CI); HTTP
    /// email-gateway when the `alerts` feature + delivery config are present.
    pub magic_link_delivery: Arc<dyn MagicLinkDelivery>,
    /// Precomputed daily-view snapshot (Silence Index, Transparency Index,
    /// property composite/divergence/portals, the brief). Populated on boot
    /// from `daily_view.json` and regenerated on the tail of each agent pass.
    /// Hero read routes consult this first and fall back to live compute when
    /// the slot is empty or stale — see `crate::daily_view`. Fixes the
    /// \>1-min dashboard load by serving yesterday's hero numbers in <100ms
    /// while the moka cache warms.
    pub daily_view: DailyViewSlot,
    pub settings: Arc<Settings>,
}
