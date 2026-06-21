//! Shared application state handed to every handler via axum's `State` extractor.

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
    pub settings: Arc<Settings>,
}
