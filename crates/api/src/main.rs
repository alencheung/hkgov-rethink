//! hkgov-api — the public serving binary.
//!
//! Responsibilities:
//! - Boot settings, telemetry, store, connectors, the ingest supervisor, and
//!   (optionally) the AI-agent supervisor.
//! - Expose a thin read-only HTTP API over the warmed cache + agent insights.
//! - Wrap every route in the tower stack (timeout, concurrency limit, trace,
//!   CORS, gzip) that will carry us toward the 100k-concurrency target.

mod auth;
mod error;
mod routes;
mod state;

use crate::state::AppState;
use hkgov_agent::{AgentSupervisor, HeuristicClient, InsightStore, LlmClient};
use hkgov_common::Settings;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings = Settings::load().unwrap_or_else(|e| {
        eprintln!("failed to load config ({e}); continuing with defaults");
        Settings::default()
    });

    hkgov_common::telemetry::init_with_otel(&settings.log.format, &settings.log.filter);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "hkgov-api starting");

    let registry = Arc::new(hkgov_connectors::registry::Registry::build(&settings)?);
    let store = Arc::new(hkgov_store::MemoryStore::new(
        settings.cache.max_entries,
        settings.cache.ttl_secs,
    ));
    let insights = Arc::new(InsightStore::new());

    // Background cache warmer. Lives for the lifetime of the process.
    let _ingest = hkgov_ingest::IngestSupervisor::spawn(registry.clone(), store.clone());

    // AI-agent layer. The LLM client is the heuristic baseline by default; the
    // `llm` feature swaps in an HTTP client. The supervisor reads from the
    // warmed store, so we give it a moment to warm before the first pass.
    let _agent = if settings.agent.enabled {
        let llm: Arc<dyn LlmClient> = build_llm_client(&settings);
        let store_for_agent = store.clone();
        let insights_for_agent = insights.clone();
        // Delay the first pass so the cache has something to analyze.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let sup = AgentSupervisor::spawn(
                store_for_agent,
                insights_for_agent,
                llm,
                Duration::from_secs(settings.agent.run_interval_secs.max(300)),
            );
            // Keep the supervisor alive for the process lifetime.
            // (abort_all is only relevant in tests.)
            std::future::pending::<()>().await;
            sup.abort_all();
        });
        tracing::info!(producer = "agent", "agent supervisor enabled");
        true
    } else {
        tracing::info!("agent supervisor disabled (set [agent] enabled=true to enable)");
        false
    };

    let state = AppState {
        registry,
        store,
        insights,
        settings: Arc::new(settings.clone()),
    };

    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&settings.api.bind).await?;
    tracing::info!(bind = %settings.api.bind, agent_enabled = _agent, "hkgov-api listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("hkgov-api stopped");
    Ok(())
}

/// Construct the LLM client. Default build uses the heuristic client; the `llm`
/// feature on the agent crate + a configured base URL selects the HTTP client.
fn build_llm_client(settings: &Settings) -> Arc<dyn LlmClient> {
    #[cfg(feature = "llm")]
    if !settings.agent.llm_base_url.is_empty() {
        if let Ok(c) = hkgov_agent::llm::HttpLlmClient::new(&settings.agent) {
            return Arc::new(c);
        }
    }
    let _ = settings;
    Arc::new(HeuristicClient::new())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl-c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
