//! Connectors to Hong Kong Government public data sources.
//!
//! Each connector is responsible for ONE upstream family and knows how to turn
//! its raw payloads into [`NormalizedRecord`]s. The ingest pipeline orchestrates
//! them; the serving API never calls a connector directly.
//!
//! v1 ships the **HKMA** connector (verified live against
//! `api.hkma.gov.hk`). The others are defined as traits/stubs so the ingest
//! layer can reference them now and the implementations land in later milestones
//! (see docs/ROADMAP.md).

pub mod hkma;
pub mod registry;

use async_trait::async_trait;
use hkgov_common::{DataSource, NormalizedRecord, Result};

/// What every connector must do. Implementations are constructed once at startup
/// and shared (via `Arc`) across the ingestion scheduler and reload fan-out.
#[async_trait]
pub trait Connector: Send + Sync + 'static {
    /// Which [`DataSource`] family this connector handles.
    fn source(&self) -> DataSource;

    /// Datasets this connector can fetch. Stable identifiers — HKMA uses its
    /// documentation slugs (e.g. `capital-market-statistics`).
    fn datasets(&self) -> &[DatasetSpec];

    /// Fetch one dataset's records. Large datasets should be paged upstream and
    /// streamed back; the caller decides how big a batch to cache.
    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>>;
}

/// Static description of a dataset a connector exposes.
#[derive(Debug, Clone)]
pub struct DatasetSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub description: Option<&'static str>,
    /// How often the ingest scheduler should refresh this dataset, seconds.
    pub refresh_interval_secs: u64,
}
