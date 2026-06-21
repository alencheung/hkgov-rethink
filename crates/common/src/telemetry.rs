//! Observability bootstrap.
//!
//! Structured tracing with an env filter, JSON or human-readable depending on
//! config. With the `otel` feature, also wires an OpenTelemetry tracer so spans
//! flow out for collection — the default exporter is stdout (zero-config,
//! stable across OTel versions). To ship spans to an OTLP collector instead,
//! swap the exporter builder here and set `OTEL_EXPORTER_OTLP_ENDPOINT`.

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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

/// Initialize tracing WITH an OpenTelemetry exporter (requires the `otel`
/// feature). Uses the stdout exporter by default — stable and zero-config.
/// Falls back to plain init when the feature is off.
#[cfg(feature = "otel")]
pub fn init_with_otel(format: &str, filter: &str) {
    use opentelemetry::trace::TracerProvider as _;

    let env_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let exporter = opentelemetry_stdout::SpanExporter::default();
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_simple_exporter(exporter)
        .build();
    let tracer = provider.tracer("hkgov");
    let ol = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(ol)
        .with(fmt_layer(format))
        .init();

    // Keep the provider alive for the process lifetime.
    std::mem::forget(provider);
}

#[cfg(feature = "otel")]
fn fmt_layer<S>(format: &str) -> Box<dyn tracing_subscriber::Layer<S> + Send + Sync>
where
    S: for<'a> tracing_subscriber::registry::LookupSpan<'a> + tracing::Subscriber + Send + Sync,
{
    match format {
        "json" => Box::new(fmt::layer().json()),
        _ => Box::new(fmt::layer()),
    }
}

/// When the `otel` feature is off, delegate to plain init.
#[cfg(not(feature = "otel"))]
pub fn init_with_otel(format: &str, filter: &str) {
    init(format, filter);
}
