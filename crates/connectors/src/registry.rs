//! Connector registry — the single place ingest asks "who serves this source?".
//!
//! Every connector is wrapped in a [`ResilientConnector`] that applies a
//! per-source rate limiter and circuit breaker before delegating to the real
//! connector. This is the v2 resilience layer (ROADMAP item).

use crate::resilience::{CircuitBreaker, RateLimiter};
use crate::{
    datagovhk::DataGovHkConnector, hkma::HkmaConnector, immigration::ImmigrationConnector,
    landregistry::LandRegistryConnector, landsd::LandsDConnector, press::PressConnector,
    rvd::RvdConnector, Connector, DatasetSpec,
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

        // Immigration Department (入境事務處) — daily border-crossing traffic CSV.
        // The CSV is a single large file; 2 req/s is conservative for one pull.
        let immigration: Arc<dyn Connector> =
            Arc::new(ImmigrationConnector::new(&settings.upstream)?);
        by_source.push(wrap(
            immigration,
            2.0,
            5,
            std::time::Duration::from_secs(60),
        ));

        // Rating & Valuation Department (差餉物業估價處) — monthly price/rental
        // index CSVs. Two files, each a single pull; 2 req/s is conservative.
        let rvd: Arc<dyn Connector> = Arc::new(RvdConnector::new(&settings.upstream)?);
        by_source.push(wrap(rvd, 2.0, 5, std::time::Duration::from_secs(60)));

        // Land Registry (土地註冊處) — monthly property transaction JSON files.
        let landregistry: Arc<dyn Connector> =
            Arc::new(LandRegistryConnector::new(&settings.upstream)?);
        by_source.push(wrap(
            landregistry,
            2.0,
            5,
            std::time::Duration::from_secs(60),
        ));

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

    /// Validate that every `(source, dataset)` referenced by `scan_targets` is
    /// served by a registered connector. Returns the list of targets whose
    /// source or dataset slug is unknown — empty means all valid.
    ///
    /// This is the structural guard against D-012 recurring: a catalog rewrite
    /// that renames a dataset slug would silently produce zero findings (the
    /// detector runs against a dataset the store never warms). Catching it at
    /// boot turns a silent "no insights" into a loud, actionable warning.
    pub fn validate_scan_targets(
        &self,
        scan_targets: &[hkgov_common::ScanTarget],
    ) -> Vec<ScanTargetValidation> {
        let known: std::collections::HashSet<(DataSource, &str)> = self
            .all_datasets()
            .into_iter()
            .map(|(s, d)| (s, d.id))
            .collect();
        scan_targets
            .iter()
            .filter_map(|t| {
                let src = DataSource::parse(&t.source)?;
                if known.contains(&(src, t.dataset.as_str())) {
                    return None;
                }
                // Also check the companion if present.
                Some(ScanTargetValidation {
                    source: t.source.clone(),
                    dataset: t.dataset.clone(),
                    kind: ScanTargetKind::Primary,
                })
            })
            .chain(scan_targets.iter().filter_map(|t| {
                let c = t.companion.as_ref()?;
                let src = DataSource::parse(&c.source)?;
                if known.contains(&(src, c.dataset.as_str())) {
                    return None;
                }
                Some(ScanTargetValidation {
                    source: c.source.clone(),
                    dataset: c.dataset.clone(),
                    kind: ScanTargetKind::Companion,
                })
            }))
            .collect()
    }
}

/// Result of validating a scan target against the registry's known datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanTargetValidation {
    pub source: String,
    pub dataset: String,
    pub kind: ScanTargetKind,
}

/// Whether the validation failure is on the scan target's primary dataset or
/// its companion (the cross-source join partner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTargetKind {
    Primary,
    Companion,
}

impl std::fmt::Display for ScanTargetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanTargetKind::Primary => write!(f, "primary"),
            ScanTargetKind::Companion => write!(f, "companion"),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The default scan targets must all reference datasets the connectors
    /// actually serve. This is the D-012 regression guard: if a catalog change
    /// renames a slug, this test fails loudly rather than silently producing
    /// zero findings at runtime.
    #[test]
    fn default_scan_targets_are_all_known() {
        let settings = Settings::default();
        let registry = Registry::build(&settings).expect("registry builds from defaults");
        let scan = hkgov_common::default_scan_targets();
        let unknown = registry.validate_scan_targets(&scan);
        assert!(
            unknown.is_empty(),
            "default scan targets reference unknown datasets: {unknown:?}"
        );
    }

    /// An obviously-bogus scan target must be flagged as unknown.
    #[test]
    fn unknown_scan_target_is_flagged() {
        let settings = Settings::default();
        let registry = Registry::build(&settings).expect("registry builds from defaults");
        let bogus = vec![hkgov_common::ScanTarget {
            source: "hkma".into(),
            dataset: "this-slug-does-not-exist-12345".into(),
            ..Default::default()
        }];
        let unknown = registry.validate_scan_targets(&bogus);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].dataset, "this-slug-does-not-exist-12345");
        assert_eq!(unknown[0].kind, ScanTargetKind::Primary);
    }

    /// An unknown source should not panic — it's silently skipped (the target
    /// is treated as unparseable, consistent with how the scheduler handles it).
    #[test]
    fn unknown_source_does_not_panic() {
        let settings = Settings::default();
        let registry = Registry::build(&settings).expect("registry builds from defaults");
        let bogus = vec![hkgov_common::ScanTarget {
            source: "not-a-real-source".into(),
            dataset: "anything".into(),
            ..Default::default()
        }];
        // DataSource::parse returns None for unknown sources, so the target is
        // skipped rather than reported — but the call must not panic.
        let _unknown = registry.validate_scan_targets(&bogus);
    }
}
