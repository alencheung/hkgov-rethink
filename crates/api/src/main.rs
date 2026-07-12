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
    // PR-013: a malformed config.toml or a bad env override used to silently drop
    // to defaults — in prod that means an operator fat-fingers HKGOV_API__API_KEY
    // and the server boots with auth *off*. Config-load failures must be fatal.
    // (figment treats a *missing* config.toml as an optional merge, so this does
    // NOT break `cargo run` with no config file — only genuinely broken config.)
    // Escape hatch HKGOV_STRICT_CONFIG=false preserves the old forgive-on-error
    // behavior for local experimentation.
    let settings = Settings::load().unwrap_or_else(|e| {
        let strict = std::env::var("HKGOV_STRICT_CONFIG")
            .map(|v| v != "false")
            .unwrap_or(true);
        if strict {
            eprintln!("failed to load config ({e}).");
            eprintln!("aborting — a malformed config in production is a security risk (e.g. silently auth-off).");
            eprintln!("set HKGOV_STRICT_CONFIG=false to fall back to defaults and continue.");
            std::process::exit(1);
        }
        eprintln!("HKGOV_STRICT_CONFIG=false: ignoring config error ({e}); continuing with defaults");
        Settings::default()
    });

    hkgov_common::telemetry::init_with_otel(&settings.log.format, &settings.log.filter);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "hkgov-api starting");

    // V-005 loud warning: `dev_return_auth_token` makes POST /auth/request-token
    // return the magic-link token in the body, so anyone who can reach the API
    // can mint a session for any email. It must only be on in dev/CI. Emit this
    // after telemetry init so it lands in the structured log, before any binding.
    if settings.api.dev_return_auth_token {
        tracing::warn!(
            "dev_return_auth_token is enabled — anyone who can reach the API can mint \
             sessions for any email. Disable in production!"
        );
    }

    let registry = Arc::new(hkgov_connectors::registry::Registry::build(&settings)?);

    // D-012 guard: validate that every scan-target slug the agent will scan is
    // actually served by a registered connector. A catalog rewrite that renames
    // a slug silently produced zero findings (the detector ran against a dataset
    // the store never warmed). This turns that into a loud boot-time warning so
    // it's caught before the first pass rather than hours later via empty output.
    //
    // Also logs the *effective* scan targets (defaults when `agent.scan` is
    // empty), so an operator can see exactly which detectors will run without
    // reading source code — the defaults previously lived only in
    // `default_scan_targets()` and were invisible at runtime.
    if settings.agent.enabled {
        let scan: Vec<hkgov_common::ScanTarget> = if settings.agent.scan.is_empty() {
            hkgov_common::default_scan_targets()
        } else {
            settings.agent.scan.clone()
        };
        let using_defaults = settings.agent.scan.is_empty();
        tracing::info!(
            count = scan.len(),
            source = if using_defaults {
                "built-in defaults"
            } else {
                "config.toml [[agent.scan]]"
            },
            "agent scan targets (effective):"
        );
        for t in &scan {
            tracing::info!(
                source = %t.source,
                dataset = %t.dataset,
                detector = %t.detector,
                field = ?t.field,
                "  scan target",
            );
        }
        let unknown = registry.validate_scan_targets(&scan);
        if !unknown.is_empty() {
            for v in &unknown {
                tracing::error!(
                    source = %v.source,
                    dataset = %v.dataset,
                    kind = %v.kind,
                    "scan target references a dataset no connector serves — \
                     this detector will produce no findings. Update the slug or \
                     remove the scan target."
                );
            }
            tracing::error!(
                count = unknown.len(),
                "scan-target validation failed; {} target(s) reference unknown datasets. \
                 The agent will run but these detectors will be no-ops. Fix the slugs in \
                 config.toml [[agent.scan]] or report a connector/catalog drift.",
                unknown.len()
            );
        }
    }

    let store: Arc<MemoryStore> = build_store(&settings)?;
    let insights = Arc::new(InsightStore::new());
    let feedback = Arc::new(hkgov_agent::FeedbackStore::new());
    let signals = Arc::new(hkgov_agent::SignalStore::new());
    let investigations = Arc::new(hkgov_agent::InvestigationStore::new());
    let users = Arc::new(hkgov_agent::UserStore::new());

    // File-based persistence stopgap: restore all user-authored state from the
    // snapshot directory so signals, investigations, identity, insights, and
    // feedback survive a graceful restart. A missing or corrupt snapshot is a
    // no-op (store starts empty). The snapshot directory defaults to `./data`;
    // override with `HKGOV_PERSIST__DIR`. Set to an empty string to disable.
    let persist_dir = std::env::var("HKGOV_PERSIST__DIR").unwrap_or_else(|_| "data".into());
    let persist_dir_path = if !persist_dir.is_empty() {
        let dir = std::path::PathBuf::from(&persist_dir);
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    } else {
        None
    };

    // Helper: restore a store from its snapshot file (no-op if missing/corrupt).
    macro_rules! restore_store {
        ($store:expr, $snap:path, $name:literal, $dir:expr) => {{
            let path = $dir.join(concat!($name, ".json"));
            if let Some(snap) = hkgov_agent::persist::restore_from_file::<$snap>(&path).await {
                $store.restore(snap).await;
                tracing::info!(path = %path.display(), concat!("restored ", $name, " store from snapshot"));
            }
            path
        }};
    }

    let users_snapshot_path = match &persist_dir_path {
        Some(dir) => Some(restore_store!(
            users,
            hkgov_agent::UserStoreSnapshot,
            "users",
            dir
        )),
        None => None,
    };
    let insights_snapshot_path = match &persist_dir_path {
        Some(dir) => Some(restore_store!(
            insights,
            hkgov_agent::InsightStoreSnapshot,
            "insights",
            dir
        )),
        None => None,
    };
    let signals_snapshot_path = match &persist_dir_path {
        Some(dir) => Some(restore_store!(
            signals,
            hkgov_agent::SignalStoreSnapshot,
            "signals",
            dir
        )),
        None => None,
    };
    let investigations_snapshot_path = match &persist_dir_path {
        Some(dir) => Some(restore_store!(
            investigations,
            hkgov_agent::InvestigationStoreSnapshot,
            "investigations",
            dir
        )),
        None => None,
    };
    let feedback_snapshot_path = match &persist_dir_path {
        Some(dir) => Some(restore_store!(
            feedback,
            hkgov_agent::FeedbackStoreSnapshot,
            "feedback",
            dir
        )),
        None => None,
    };

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
        tracing::info!(
            "agent supervisor disabled (insights + Silence Index will stay empty; \
             set [agent] enabled=true to run the scanner)"
        );
        false
    };

    // Periodic snapshot of all user-authored stores (stopgap persistence).
    // Saves identity, insights, signals, investigations, and feedback to disk
    // every 60s so a graceful restart doesn't wipe user state. The first
    // snapshot fires after one interval, covering a quick restart cycle.
    //
    // Helper: spawn a periodic snapshot task for one store.
    macro_rules! spawn_snapshot_task {
        ($store:expr, $path:expr, $name:literal) => {
            if let Some(ref path) = $path {
                let store = $store.clone();
                let path = path.clone();
                tokio::spawn(async move {
                    let interval = Duration::from_secs(60);
                    loop {
                        tokio::time::sleep(interval).await;
                        let snap = store.snapshot().await;
                        if let Err(e) = hkgov_agent::persist::snapshot_to_file(&path, &snap).await {
                            tracing::warn!(
                                path = %path.display(),
                                error = %e,
                                concat!($name, "-store snapshot failed")
                            );
                        }
                    }
                });
            }
        };
    }

    spawn_snapshot_task!(users, users_snapshot_path, "user");
    spawn_snapshot_task!(insights, insights_snapshot_path, "insight");
    spawn_snapshot_task!(signals, signals_snapshot_path, "signal");
    spawn_snapshot_task!(
        investigations,
        investigations_snapshot_path,
        "investigation"
    );
    spawn_snapshot_task!(feedback, feedback_snapshot_path, "feedback");

    // Magic-link email delivery sink. The default is the log-based sink (dev/CI
    // — logs the delivery event so a log-shipper pipeline can transport it).
    // When `HKGOV_MAGIC_LINK__API_URL` is set (and the `alerts` feature is on
    // for reqwest), the HTTP email-gateway sink is used instead — the production
    // path for real email delivery via SendGrid/Mailgun/SES-via-HTTP.
    let magic_link_delivery: Arc<dyn hkgov_agent::MagicLinkDelivery> = build_magic_link_delivery();

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
        magic_link_delivery,
        settings: Arc::new(settings.clone()),
    };

    let app = routes::router(state);

    // Railway (and most PaaS hosts: Fly, Render, Cloud Run) inject a `PORT`
    // env var and route their public proxy to that exact port — the
    // Dockerfile's EXPOSE is informational only and is ignored for routing.
    // Honoring `PORT` ahead of `api.bind` is what makes the container reachable
    // there. `PORT` carries just the port (e.g. "8080"), so we splice it onto
    // the wildcard host from config; a full `host:port` in `PORT` is also
    // accepted. Locally (no `PORT`) we fall through to `api.bind`, preserving
    // the zero-config `0.0.0.0:8080` default and the `HKGOV_API__BIND` override.
    let bind = match std::env::var("PORT") {
        Ok(p) if !p.trim().is_empty() => {
            let p = p.trim();
            if p.contains(':') {
                p.to_string()
            } else {
                format!("0.0.0.0:{p}")
            }
        }
        _ => settings.api.bind.clone(),
    };
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(bind = %bind, agent_enabled = _agent, "hkgov-api listening");
    // V-003: `into_make_service_with_connect_info` exposes the peer IP to the
    // rate-limit middleware (it keys the token bucket per source IP).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
        match hkgov_agent::llm::HttpLlmClient::new(&settings.agent) {
            Ok(c) => return Arc::new(c),
            // Surface the failure rather than silently degrading to the
            // heuristic keyword matcher. Without this, a misconfigured base URL
            // or bad key boots cleanly and /v1/ask quietly returns shallow
            // answers with no signal to the operator.
            Err(e) => tracing::warn!(
                error = %e,
                base_url = %settings.agent.llm_base_url,
                "LLM client construction failed; falling back to the heuristic \
                 client. Rich /v1/ask framing will be disabled until this is fixed."
            ),
        }
    }
    let _ = settings;
    Arc::new(HeuristicClient::new())
}

/// Construct the magic-link delivery sink. Default: log-based (dev/CI). When
/// `HKGOV_MAGIC_LINK__API_URL` is set, the HTTP email-gateway sink is used
/// (SendGrid/Mailgun/SES-via-HTTP — behind the `alerts` feature for reqwest).
fn build_magic_link_delivery() -> Arc<dyn hkgov_agent::MagicLinkDelivery> {
    let api_url = std::env::var("HKGOV_MAGIC_LINK__API_URL").unwrap_or_default();
    if !api_url.is_empty() {
        #[cfg(feature = "alerts")]
        {
            let token = std::env::var("HKGOV_MAGIC_LINK__API_TOKEN").unwrap_or_default();
            let from = std::env::var("HKGOV_MAGIC_LINK__FROM")
                .unwrap_or_else(|_| "HK City Pulse <noreply@hkgov-rethink.example>".into());
            let redeem_base = std::env::var("HKGOV_MAGIC_LINK__REDEEM_BASE_URL")
                .unwrap_or_else(|_| "https://hkgov-rethink-production.up.railway.app".into());
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            tracing::info!(
                api_url = %api_url,
                from = %from,
                "magic-link HTTP email delivery configured"
            );
            return Arc::new(hkgov_agent::HttpMagicLinkDelivery::new(
                api_url,
                token,
                from,
                redeem_base,
                client,
            ));
        }
        #[cfg(not(feature = "alerts"))]
        {
            tracing::warn!(
                api_url = %api_url,
                "HKGOV_MAGIC_LINK__API_URL is set but the `alerts` feature is not enabled — \
                 falling back to log-based delivery. Rebuild with --features alerts to use HTTP email."
            );
        }
    }
    Arc::new(hkgov_agent::LogMagicLinkDelivery)
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
