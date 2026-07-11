//! In-process store backed by `moka` (future-aware LRU + TTL).
//!
//! This is the v1 implementation. It is deliberately simple: one cache of
//! record vectors keyed by [`DatasetId`], plus a parallel small cache of
//! metadata so counts/refresh timestamps survive independently of the data.

use crate::{DatasetId, RecordPage, RecordStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

/// One cached dataset: its records plus the timestamp of the last `put_dataset`.
///
/// Keeping both in a single moka entry means `put_dataset` publishes them
/// atomically in one [`Cache::insert`] — a concurrent `meta()` reader can never
/// observe new records paired with a stale `last_refreshed_at` (the previous
/// two-step insert raced in exactly that direction).
#[derive(Debug, Clone)]
struct CacheEntry {
    records: Arc<Vec<NormalizedRecord>>,
    refreshed_at: DateTime<Utc>,
}

pub struct MemoryStore {
    records: Cache<DatasetId, CacheEntry>,
    /// Light-touch registry of static dataset metadata.
    registry: RwLock<HashMap<DatasetId, RegisteredMeta>>,
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
        // Single atomic publish: records + refreshed_at land together, so a
        // concurrent reader cannot observe new records with an old timestamp.
        let entry = CacheEntry {
            records: Arc::new(records),
            refreshed_at: now,
        };
        self.records.insert(dataset_id.clone(), entry).await;
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
        let Some(entry) = self.records.get(dataset_id).await else {
            return Err(Error::Store(format!(
                "no records cached for {}/{}",
                dataset_id.source, dataset_id.dataset
            )));
        };
        let total = entry.records.len();
        let page: Vec<NormalizedRecord> = entry
            .records
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        Ok(RecordPage {
            source: dataset_id.source,
            dataset: dataset_id.dataset.clone(),
            total,
            offset,
            limit,
            records: page,
        })
    }

    // PR-003: efficient by-id lookup for the citation manifest. Iterates the
    // cached record vector once (no paging) so the reproducibility hash is over
    // the insight's actual evidence records, not a 500-row page head.
    async fn get_by_ids(
        &self,
        dataset_id: &DatasetId,
        ids: &[String],
    ) -> Result<Vec<NormalizedRecord>> {
        let Some(entry) = self.records.get(dataset_id).await else {
            return Err(Error::Store(format!(
                "no records cached for {}/{}",
                dataset_id.source, dataset_id.dataset
            )));
        };
        let want: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
        Ok(entry
            .records
            .iter()
            .filter(|r| want.contains(r.record_id.as_str()))
            .cloned()
            .collect())
    }

    async fn meta(&self, dataset_id: &DatasetId) -> Result<Option<DatasetMeta>> {
        let registry = self.registry.read().await;
        let Some(static_meta) = registry.get(dataset_id) else {
            return Ok(None);
        };
        let entry = self.records.get(dataset_id).await;
        let count = entry.as_ref().map(|e| e.records.len()).unwrap_or(0);
        let last = entry.as_ref().map(|e| e.refreshed_at);
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
            let entry = self.records.get(&id).await;
            let count = entry.as_ref().map(|e| e.records.len()).unwrap_or(0);
            let last = entry.as_ref().map(|e| e.refreshed_at);
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
