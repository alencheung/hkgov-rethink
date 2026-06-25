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
mod ratelimit;
mod routes;
mod secrets;
mod state;

use crate::state::AppState;
use hkgov_agent::{AgentSupervisor, HeuristicClient, InsightStore, LlmClient};
use hkgov_common::Settings;
use hkgov_store::MemoryStore;
use std::sync::Arc;
use std::time::Duration;

/// Build the hot-tier record store from `store.backend` config.
///
/// The architecture is cache-first: an in-process `moka` hot tier (the default)
/// fronts optional redis/pg cold tiers. v1 ships the hot tier only; the
/// `store.backend` knob selects it. Previously this config was **dead** —
/// hardcoded to `MemoryStore`, so `HKGOV_STORE__BACKEND=redis` was silently
/// ignored (FEATURES_TRACKER F-085). Now the selection is honored: an unknown
/// or not-yet-implemented backend fails loudly at boot with an actionable
/// message, rather than silently degrading to memory.
///
/// The full multi-tier store (moka → redis → pg read-through) and the
/// generalization of the agent supervisor to `Arc<dyn RecordStore>` are
/// documented roadmap items (G2 persistence workstream).
fn build_store(settings: &Settings) -> anyhow::Result<Arc<MemoryStore>> {
    let backend = settings.store.backend.trim().to_ascii_lowercase();
    match backend.as_str() {
        "" | "memory" => Ok(Arc::new(MemoryStore::new(
            settings.cache.max_entries,
            settings.cache.ttl_secs,
        ))),
        "redis" | "pg" | "postgres" => anyhow::bail!(
            "store.backend={backend} is not yet wired into the boot path. The hot-tier \
             (moka) store is the only backend the agent supervisor and serving API currently \
             bind to. The multi-tier (moka → redis → pg read-through) store is a documented \
             roadmap item (G2 persistence workstream). For now, omit store.backend to use the \
             zero-config in-process cache, which is the architecture's intended hot tier."
        ),
        other => {
            anyhow::bail!("unknown store.backend={other:?} (expected memory; redis/pg are roadmap)")
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings = Settings::load().unwrap_or_else(|e| {
        eprintln!("failed to load config ({e}); continuing with defaults");
        Settings::default()
    });

    hkgov_common::telemetry::init_with_otel(&settings.log.format, &settings.log.filter);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "hkgov-api starting");

    let registry = Arc::new(hkgov_connectors::registry::Registry::build(&settings)?);
    let store: Arc<MemoryStore> = build_store(&settings)?;
    let insights = Arc::new(InsightStore::new());
    let feedback = Arc::new(hkgov_agent::FeedbackStore::new());
    let signals = Arc::new(hkgov_agent::SignalStore::new());
    let investigations = Arc::new(hkgov_agent::InvestigationStore::new());
    let users = Arc::new(hkgov_agent::UserStore::new());
    // Build the LLM client up front so both the supervisor and the /v1/ask
    // endpoint share the same instance.
    let llm: Arc<dyn LlmClient> = build_llm_client(&settings);

    // Background cache warmer. Lives for the lifetime of the process.
    let _ingest = hkgov_ingest::IngestSupervisor::spawn(registry.clone(), store.clone());

    // AI-agent layer. The LLM client is the heuristic baseline by default; the
    // `llm` feature swaps in an HTTP client. The supervisor reads from the
    // warmed store, so we give it a moment to warm before the first pass.
    // Proactive alerting is built from settings when enabled (needs the `alerts`
    // feature for the webhook sink; the dispatcher itself is always available).
    let alert_dispatcher: Option<Arc<hkgov_agent::AlertDispatcher>> = if settings.agent.enabled {
        hkgov_agent::AlertDispatcher::from_settings(&settings.alerts).map(Arc::new)
    } else {
        None
    };
    let alert_log: Arc<hkgov_agent::AlertLog> = alert_dispatcher
        .as_ref()
        .map(|d| d.log())
        .unwrap_or_else(|| Arc::new(hkgov_agent::AlertLog::new(200)));

    let _agent = if settings.agent.enabled {
        let store_for_agent = store.clone();
        let insights_for_agent = insights.clone();
        let llm_for_agent = llm.clone();
        let settings_for_agent = Arc::new(settings.clone());
        let alerts_for_agent = alert_dispatcher.clone();
        // D-012: previously the first agent pass fired after a fixed 20s delay.
        // With the catalog widened to 186 datasets warming concurrently under
        // per-source rate limits (HKMA 5/s ⇒ ~37s for HKMA alone), 20s was not
        // long enough for the flagship HIBOR feed to be fetched — so the first
        // (and for hours the only) pass scanned an empty dataset and produced
        // no HIBOR findings. Instead of a blind sleep, wait until the datasets
        // the scan targets reference actually have records, capped at a few
        // minutes so a permanently-unreachable source never blocks the agent.
        let readiness_store = store.clone();
        let readiness_settings = settings_for_agent.clone();
        tokio::spawn(async move {
            wait_for_scan_readiness(&readiness_store, &readiness_settings.agent).await;
            let sup = AgentSupervisor::spawn(
                store_for_agent,
                insights_for_agent,
                llm_for_agent,
                settings_for_agent,
                alerts_for_agent,
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
        feedback,
        signals,
        investigations,
        users,
        llm,
        alert_log,
        settings: Arc::new(settings.clone()),
    };

    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&settings.api.bind).await?;
    tracing::info!(bind = %settings.api.bind, agent_enabled = _agent, "hkgov-api listening");
    // V-003: `into_make_service_with_connect_info` exposes the peer IP to the
    // rate-limit middleware (it keys the token bucket per source IP).
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
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

/// Wait until the datasets the configured scan targets reference have at least
/// one record each before letting the agent's first pass run. Capped at
/// `cap` so a permanently-unreachable upstream never blocks the agent — it
/// just proceeds with whatever has warmed. Polls every 2s; the minimum wait is
/// a short grace so the ingest supervisor has been scheduled at all.
///
/// D-012: this replaces the old fixed 20s sleep. The sleep was too short once
/// the catalog grew to 186 datasets under per-source rate limits, so the first
/// (and for hours the only) analysis pass ran against an empty store and the
/// flagship HIBOR detector produced nothing.
async fn wait_for_scan_readiness(store: &Arc<MemoryStore>, agent: &hkgov_common::AgentSettings) {
    use hkgov_common::{DataSource, ScanTarget};
    use hkgov_store::{DatasetId, RecordStore};
    // Resolve the effective scan list (defaults when none configured).
    let scan: Vec<ScanTarget> = if agent.scan.is_empty() {
        hkgov_common::default_scan_targets()
    } else {
        agent.scan.clone()
    };
    // Collect the set of datasets the targets need (primary + companion).
    let mut needed: Vec<DatasetId> = Vec::new();
    for t in &scan {
        if let Some(s) = DataSource::parse(&t.source) {
            needed.push(DatasetId::new(s, t.dataset.clone()));
        }
        if let Some(c) = &t.companion {
            if let Some(s) = DataSource::parse(&c.source) {
                needed.push(DatasetId::new(s, c.dataset.clone()));
            }
        }
    }
    // A short initial grace so the ingest supervisor (spawned just before us)
    // has actually been polled and kicked off its fetch tasks.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let cap = Duration::from_secs(180);
    let deadline = tokio::time::Instant::now() + cap;
    loop {
        let mut ready = 0;
        for id in &needed {
            match store.meta(id).await {
                Ok(Some(m)) if m.record_count > 0 => ready += 1,
                _ => {}
            }
        }
        if ready == needed.len() || needed.is_empty() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                ready,
                total = needed.len(),
                "agent: scan-target readiness wait timed out after 180s; \
                 running the first pass with the datasets that have warmed"
            );
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    tracing::info!("agent: scan-target datasets warmed, starting first analysis pass");
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
