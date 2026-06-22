//! Shared application state handed to every handler via axum's `State` extractor.

use hkgov_agent::{
    AlertLog, FeedbackStore, InsightStore, InvestigationStore, LlmClient, SignalStore, UserStore,
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
    pub settings: Arc<Settings>,
}
