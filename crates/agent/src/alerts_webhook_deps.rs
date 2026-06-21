//! Shared reqwest client construction for feature-gated HTTP sinks.
//!
//! Kept in its own module so the heavy `reqwest::Client::builder` call site is
//! only compiled when one of the HTTP-bearing features (`alerts`) is on. This
//! avoids pulling reqwest into the default build's type-check surface.

/// Build a reqwest client tuned for outbound webhook delivery: short timeout,
/// small connection pool, the project user-agent.
pub fn build_webhook_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(4)
        .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
