//! hkgov-api — the public serving binary.
//!
//! Responsibilities (v1):
//! - Boot settings, telemetry, store, connectors, and the ingest supervisor.
//! - Expose a thin read-only HTTP API over the warmed cache.
//! - Wrap every route in the tower stack (timeout, concurrency limit, trace,
//!   CORS, gzip) that will carry us toward the 100k-concurrency target.
//!
//! The "AI-agent analysis" layer ships in a later milestone; this binary is the
//! substrate it will sit on (see docs/ROADMAP.md).

mod error;
mod routes;
mod state;

use crate::state::AppState;
use hkgov_common::Settings;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings = Settings::load().unwrap_or_else(|e| {
        eprintln!("failed to load config ({e}); continuing with defaults");
        Settings::default()
    });

    hkgov_common::telemetry::init(&settings.log.format, &settings.log.filter);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "hkgov-api starting");

    let registry = Arc::new(hkgov_connectors::registry::Registry::build(&settings)?);
    let store = Arc::new(hkgov_store::MemoryStore::new(
        settings.cache.max_entries,
        settings.cache.ttl_secs,
    ));

    // Background cache warmer. Lives for the lifetime of the process.
    let _supervisor = hkgov_ingest::IngestSupervisor::spawn(registry.clone(), store.clone());

    let state = AppState {
        registry,
        store,
        settings: Arc::new(settings.clone()),
    };

    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&settings.api.bind).await?;
    tracing::info!(bind = %settings.api.bind, "hkgov-api listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("hkgov-api stopped");
    Ok(())
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
