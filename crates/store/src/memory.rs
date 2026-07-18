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
///
/// D-029 fix: `last_record_count` and `last_refreshed_at` are persisted here
/// (in the registry, which is NOT subject to the record cache's TTL) so the
/// catalog's `record_count` survives a record-cache eviction. Before this,
/// `record_count` was derived live from the moka cache — so once the TTL
/// (default 600s) elapsed and the entry was evicted, `/v1/sources` showed 0
/// records for every dataset until the next refresh (24h for datagovhk). The
/// live cache count is still preferred when present (it reflects an in-flight
/// refresh); the persisted count is the fallback that keeps the catalog honest
/// between refreshes.
#[derive(Debug, Clone, Default)]
struct RegisteredMeta {
    title: String,
    description: Option<String>,
    refresh_interval_secs: u64,
    category: Category,
    tags: Vec<String>,
    cadence: Cadence,
    /// Persisted count from the last successful `put_dataset`. Survives
    /// record-cache TTL expiry (D-029).
    last_record_count: usize,
    /// Persisted timestamp of the last successful `put_dataset`.
    last_refreshed_at: Option<DateTime<Utc>>,
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
        // D-031 fix: a `ttl_secs` of 0 disables time-based eviction entirely
        // (records stay cached until the ingest supervisor's per-dataset
        // refresh replaces them, or `max_capacity` evicts by LRU under
        // pressure). This is the correct default for a cache-aside store whose
        // staleness is already bounded by the refresh interval: the previous
        // default (600s) was far shorter than the typical refresh interval
        // (1800s–604800s), so every dataset's records evaporated ~10 minutes
        // after each refresh and stayed gone until the next one — up to an
        // hour for the flagship interbank dataset, a week for some HKMA feeds.
        // That made /records, /cite, and /unprecedentedness return 502 for
        // most of each refresh cycle. Only set a non-zero TTL if you
        // deliberately want records to expire between refreshes (e.g. a memory
        //-constrained single-node where you'd rather 502 than hold the data).
        let mut builder = Cache::builder().max_capacity(max_entries);
        if ttl_secs > 0 {
            builder = builder.time_to_live(std::time::Duration::from_secs(ttl_secs));
        }
        let records = builder.build();
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
        let mut registry = self.registry.write().await;
        // D-029: preserve the persisted count/timestamp across a re-register
        // (ingest re-registers every dataset on each boot). Dropping them here
        // would reset the catalog to 0 until the first fetch completes.
        let preserved_count = registry.get(&id).map(|m| m.last_record_count).unwrap_or(0);
        let preserved_last = registry.get(&id).and_then(|m| m.last_refreshed_at);
        registry.insert(
            id,
            RegisteredMeta {
                title,
                description,
                refresh_interval_secs,
                category,
                tags,
                cadence,
                last_record_count: preserved_count,
                last_refreshed_at: preserved_last,
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
        let count = records.len();
        // Single atomic publish: records + refreshed_at land together, so a
        // concurrent reader cannot observe new records with an old timestamp.
        let entry = CacheEntry {
            records: Arc::new(records),
            refreshed_at: now,
        };
        self.records.insert(dataset_id.clone(), entry).await;
        // D-029: persist the count + timestamp in the registry so the catalog
        // stays correct after the record cache's TTL evicts the entry.
        {
            let mut registry = self.registry.write().await;
            if let Some(meta) = registry.get_mut(dataset_id) {
                meta.last_record_count = count;
                meta.last_refreshed_at = Some(now);
            }
        }
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
        // D-031 defense-in-depth: if the cache is cold (fresh boot before the
        // first fetch completes, LRU eviction under memory pressure, or an
        // explicit non-zero TTL that has lapsed), return an honest empty page
        // rather than a 502. The records endpoint is read-only and browse-
        // oriented; "we have 0 rows cached right now" is a recoverable,
        // non-fatal state — the ingest supervisor will repopulate on the next
        // refresh tick. The prior 502 crashed the dashboard's record drill-
        // down, the divergence explorer, and the unprecedentedness comparator.
        let Some(entry) = self.records.get(dataset_id).await else {
            return Ok(RecordPage {
                source: dataset_id.source,
                dataset: dataset_id.dataset.clone(),
                total: 0,
                offset,
                limit,
                records: Vec::new(),
            });
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
        // Unlike `get_page`, a cache miss here is a real error: the cite
        // manifest hashes the returned records, so silently returning an empty
        // set would produce a wrong hash and falsely claim "reproduces as of
        // {ts}". Surface it as `StoreUnavailable` (mapped to 503, retryable)
        // rather than the prior generic `Store` (502) so the cite drawer can
        // tell the user "data temporarily unavailable, retry" instead of
        // showing an internal-error toast.
        let Some(entry) = self.records.get(dataset_id).await else {
            return Err(Error::StoreUnavailable(format!(
                "records for {}/{} are not cached (refresh in progress or cache cold); retry shortly",
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
        // D-029: prefer the live cache count when present (reflects an in-flight
        // refresh); fall back to the persisted count so the catalog stays
        // correct after the record cache's TTL evicts the entry.
        let (count, last) = match &entry {
            Some(e) => (e.records.len(), Some(e.refreshed_at)),
            None => (static_meta.last_record_count, static_meta.last_refreshed_at),
        };
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
            // D-029: same live-then-persisted fallback as `meta()`.
            let (count, last) = match &entry {
                Some(e) => (e.records.len(), Some(e.refreshed_at)),
                None => (static_meta.last_record_count, static_meta.last_refreshed_at),
            };
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
