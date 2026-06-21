//! Connector registry — the single place ingest asks "who serves this source?".
//!
//! Every connector is wrapped in a [`ResilientConnector`] that applies a
//! per-source rate limiter and circuit breaker before delegating to the real
//! connector. This is the v2 resilience layer (ROADMAP item).

use crate::resilience::{CircuitBreaker, RateLimiter};
use crate::{
    datagovhk::DataGovHkConnector, hkma::HkmaConnector, landsd::LandsDConnector,
    press::PressConnector, Connector, DatasetSpec,
};
use async_trait::async_trait;
use hkgov_common::{DataSource, NormalizedRecord, Result, Settings};
use std::sync::Arc;

/// Wraps a connector with rate limiting + circuit breaking. The wrapper is
/// transparent: `source()`/`datasets()` delegate, only `fetch()` is guarded.
pub struct ResilientConnector {
    inner: Arc<dyn Connector>,
    limiter: Arc<RateLimiter>,
    breaker: Arc<CircuitBreaker>,
}

impl ResilientConnector {
    pub fn new(
        inner: Arc<dyn Connector>,
        limiter: Arc<RateLimiter>,
        breaker: Arc<CircuitBreaker>,
    ) -> Self {
        Self {
            inner,
            limiter,
            breaker,
        }
    }

    pub fn breaker_state(&self) -> &'static str {
        self.breaker.state_label()
    }
}

#[async_trait]
impl Connector for ResilientConnector {
    fn source(&self) -> DataSource {
        self.inner.source()
    }
    fn datasets(&self) -> &[DatasetSpec] {
        self.inner.datasets()
    }
    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        if let Err(reason) = self.breaker.before_call() {
            tracing::warn!(
                source = %self.inner.source(),
                dataset,
                reason,
                "circuit open — skipping fetch"
            );
            return Err(hkgov_common::Error::Upstream {
                origin: self.inner.source().as_str(),
                status: 503,
                detail: format!("circuit breaker open ({reason})"),
            });
        }
        self.limiter.acquire().await;
        match self.inner.fetch(dataset).await {
            Ok(r) => {
                self.breaker.on_success();
                Ok(r)
            }
            Err(e) => {
                self.breaker.on_failure();
                Err(e)
            }
        }
    }
}

/// All live connectors, keyed by source.
pub struct Registry {
    by_source: Vec<(DataSource, Arc<ResilientConnector>)>,
}

impl Registry {
    /// Build the registry from settings. Each source gets its own limiter +
    /// breaker tuned to that source's politeness budget.
    pub fn build(settings: &Settings) -> Result<Self> {
        let mut by_source: Vec<(DataSource, Arc<ResilientConnector>)> = Vec::new();

        let hkma: Arc<dyn Connector> = Arc::new(HkmaConnector::new(&settings.upstream)?);
        by_source.push(wrap(
            hkma,
            settings.upstream.hkma_rate_per_sec as f64,
            5,
            std::time::Duration::from_secs(30),
        ));

        let datagovhk: Arc<dyn Connector> = Arc::new(DataGovHkConnector::new(&settings.upstream)?);
        by_source.push(wrap(datagovhk, 3.0, 5, std::time::Duration::from_secs(60)));

        let press: Arc<dyn Connector> = Arc::new(PressConnector::new(&settings.upstream)?);
        by_source.push(wrap(press, 2.0, 5, std::time::Duration::from_secs(60)));

        let landsd: Arc<dyn Connector> = Arc::new(LandsDConnector::new(&settings.upstream)?);
        by_source.push(wrap(landsd, 1.0, 3, std::time::Duration::from_secs(120)));

        Ok(Self { by_source })
    }

    pub fn lookup(&self, source: DataSource) -> Option<Arc<dyn Connector>> {
        self.by_source
            .iter()
            .find(|(s, _)| *s == source)
            .map(|(_, c)| c.clone() as Arc<dyn Connector>)
    }

    /// Every (source, dataset) we currently expose — feeds `/sources`.
    pub fn all_datasets(&self) -> Vec<(DataSource, &DatasetSpec)> {
        self.by_source
            .iter()
            .flat_map(|(s, c)| c.datasets().iter().map(move |d| (*s, d)))
            .collect()
    }

    pub fn sources(&self) -> Vec<DataSource> {
        self.by_source.iter().map(|(s, _)| *s).collect()
    }

    /// Health snapshot of each source's circuit breaker — used by `/health/sources`.
    pub fn breaker_states(&self) -> Vec<(DataSource, &'static str)> {
        self.by_source
            .iter()
            .map(|(s, c)| (*s, c.breaker_state()))
            .collect()
    }
}

fn wrap(
    inner: Arc<dyn Connector>,
    rate_per_sec: f64,
    failure_threshold: u64,
    cooldown: std::time::Duration,
) -> (DataSource, Arc<ResilientConnector>) {
    let source = inner.source();
    let limiter = Arc::new(RateLimiter::new(
        rate_per_sec.ceil().max(1.0) as u64,
        rate_per_sec,
    ));
    let breaker = Arc::new(CircuitBreaker::new(failure_threshold, cooldown));
    (
        source,
        Arc::new(ResilientConnector::new(inner, limiter, breaker)),
    )
}
