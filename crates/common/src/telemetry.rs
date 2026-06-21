//! Observability bootstrap.
//!
//! We keep this tiny on purpose: structured tracing with an env filter, JSON or
//! human-readable depending on config. When we move to the multi-node tier this
//! is where OpenTelemetry exporters get wired in.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(format: &str, filter: &str) {
    let env_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(env_filter);

    match format {
        "json" => {
            registry.with(fmt::layer().json()).init();
        }
        _ => {
            registry.with(fmt::layer()).init();
        }
    }
}
