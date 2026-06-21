//! In-process store backed by `moka` (future-aware LRU + TTL).
//!
//! This is the v1 implementation. It is deliberately simple: one cache of
//! record vectors keyed by [`DatasetId`], plus a parallel small cache of
//! metadata so counts/refresh timestamps survive independently of the data.

use crate::{DatasetId, RecordPage, RecordStore};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{Cadence, Category, DataSource, DatasetMeta, Error, NormalizedRecord, Result};
use moka::future::Cache;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Titles/descriptions are registered once per dataset by the ingest layer;
/// counts and refresh timestamps are updated by `put_dataset`.
#[derive(Debug, Clone, Default)]
struct RegisteredMeta {
    title: String,
    description: Option<String>,
    refresh_interval_secs: u64,
    category: Category,
    tags: Vec<String>,
    cadence: Cadence,
}

pub struct MemoryStore {
    records: Cache<DatasetId, Arc<Vec<NormalizedRecord>>>,
    /// Light-touch registry of static dataset metadata.
    registry: RwLock<HashMap<DatasetId, RegisteredMeta>>,
    /// Last refresh timestamp per dataset.
    refreshed_at: RwLock<HashMap<DatasetId, chrono::DateTime<Utc>>>,
}

impl MemoryStore {
    pub fn new(max_entries: u64, ttl_secs: u64) -> Self {
        let records = Cache::builder()
            .max_capacity(max_entries)
            .time_to_live(std::time::Duration::from_secs(ttl_secs))
            .build();
        Self {
            records,
            registry: RwLock::new(HashMap::new()),
            refreshed_at: RwLock::new(HashMap::new()),
        }
    }

    /// Register static metadata. Idempotent. The category/tags/cadence come from
    /// the connector's `DatasetSpec`; title/description/cadence from the same.
    #[allow(clippy::too_many_arguments)] // mirrors DatasetSpec's fields; grouping would obscure call sites
    pub async fn register(
        &self,
        id: DatasetId,
        title: String,
        description: Option<String>,
        refresh_interval_secs: u64,
        category: Category,
        tags: Vec<String>,
        cadence: Cadence,
    ) {
        self.registry.write().await.insert(
            id,
            RegisteredMeta {
                title,
                description,
                refresh_interval_secs,
                category,
                tags,
                cadence,
            },
        );
    }
}

#[async_trait]
impl RecordStore for MemoryStore {
    async fn put_dataset(
        &self,
        dataset_id: &DatasetId,
        records: Vec<NormalizedRecord>,
    ) -> Result<()> {
        let now = Utc::now();
        self.records
            .insert(dataset_id.clone(), Arc::new(records))
            .await;
        self.refreshed_at
            .write()
            .await
            .insert(dataset_id.clone(), now);
        tracing::debug!(
            source = %dataset_id.source,
            dataset = %dataset_id.dataset,
            "store: dataset refreshed"
        );
        Ok(())
    }

    async fn get_page(
        &self,
        dataset_id: &DatasetId,
        offset: usize,
        limit: usize,
    ) -> Result<RecordPage> {
        let limit = limit.clamp(1, 500);
        let Some(records) = self.records.get(dataset_id).await else {
            return Err(Error::Store(format!(
                "no records cached for {}/{}",
                dataset_id.source, dataset_id.dataset
            )));
        };
        let total = records.len();
        let page: Vec<NormalizedRecord> =
            records.iter().skip(offset).take(limit).cloned().collect();
        Ok(RecordPage {
            source: dataset_id.source,
            dataset: dataset_id.dataset.clone(),
            total,
            offset,
            limit,
            records: page,
        })
    }

    async fn meta(&self, dataset_id: &DatasetId) -> Result<Option<DatasetMeta>> {
        let registry = self.registry.read().await;
        let Some(static_meta) = registry.get(dataset_id) else {
            return Ok(None);
        };
        let count = self
            .records
            .get(dataset_id)
            .await
            .map(|r| r.len())
            .unwrap_or(0);
        let last = self.refreshed_at.read().await.get(dataset_id).copied();
        Ok(Some(DatasetMeta {
            source: dataset_id.source,
            dataset: dataset_id.dataset.clone(),
            title: static_meta.title.clone(),
            description: static_meta.description.clone(),
            category: static_meta.category,
            tags: static_meta.tags.clone(),
            cadence: static_meta.cadence,
            refresh_interval_secs: static_meta.refresh_interval_secs,
            last_refreshed_at: last,
            record_count: count,
        }))
    }

    async fn list(&self, source: Option<DataSource>) -> Result<Vec<DatasetMeta>> {
        let registry = self.registry.read().await;
        // Collect the static meta snapshot first so we don't hold the registry
        // read lock across awaits.
        let snapshot: Vec<(DatasetId, RegisteredMeta)> = registry
            .iter()
            .filter(|(id, _)| source.is_none_or(|s| s == id.source))
            .map(|(id, m)| (id.clone(), m.clone()))
            .collect();
        drop(registry);

        let mut out = Vec::new();
        for (id, static_meta) in snapshot {
            let count = self.records.get(&id).await.map(|r| r.len()).unwrap_or(0);
            let last = self.refreshed_at.read().await.get(&id).copied();
            out.push(DatasetMeta {
                source: id.source,
                dataset: id.dataset.clone(),
                title: static_meta.title.clone(),
                description: static_meta.description.clone(),
                category: static_meta.category,
                tags: static_meta.tags.clone(),
                cadence: static_meta.cadence,
                refresh_interval_secs: static_meta.refresh_interval_secs,
                last_refreshed_at: last,
                record_count: count,
            });
        }
        Ok(out)
    }
}
