//! Cache-first record store.
//!
//! The serving API only ever talks to a [`RecordStore`]. v1 ships an in-process
//! [`moka`]-backed implementation that is good for tens of thousands of cached
//! records on a single node. The trait is the contract the multi-node tier
//! (Redis / Postgres read replica) will satisfy later — see
//! docs/ARCHITECTURE.md §"Scaling path".

pub mod memory;
#[cfg(feature = "pg")]
pub mod pg_store;
#[cfg(feature = "redis")]
pub mod redis_store;

pub use memory::MemoryStore;
#[cfg(feature = "pg")]
pub use pg_store::PgStore;
#[cfg(feature = "redis")]
pub use redis_store::RedisStore;

use async_trait::async_trait;
use hkgov_common::{DataSource, DatasetMeta, NormalizedRecord, Result};

/// A page of records. We never hand the caller unbounded arrays.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordPage {
    pub source: DataSource,
    pub dataset: String,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub records: Vec<NormalizedRecord>,
}

/// What every record store must support. Implementations are free to be local
/// (moka) or remote (Redis cluster) — callers stay agnostic.
#[async_trait]
pub trait RecordStore: Send + Sync + 'static {
    /// Put a batch of normalized records for one dataset. Replaces prior contents
    /// for that dataset atomically.
    async fn put_dataset(
        &self,
        dataset_id: &DatasetId,
        records: Vec<NormalizedRecord>,
    ) -> Result<()>;

    /// Read a page of records for a dataset.
    async fn get_page(
        &self,
        dataset_id: &DatasetId,
        offset: usize,
        limit: usize,
    ) -> Result<RecordPage>;

    /// Best-effort metadata for a dataset (counts, last refresh). Returns None
    /// if the dataset has never been ingested.
    async fn meta(&self, dataset_id: &DatasetId) -> Result<Option<DatasetMeta>>;

    /// All datasets currently held, by source.
    async fn list(&self, source: Option<DataSource>) -> Result<Vec<DatasetMeta>>;
}

/// Stable identity for a (source, dataset) pair — used as a cache key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct DatasetId {
    pub source: DataSource,
    pub dataset: String,
}

impl DatasetId {
    pub fn new(source: DataSource, dataset: impl Into<String>) -> Self {
        Self {
            source,
            dataset: dataset.into(),
        }
    }
}
