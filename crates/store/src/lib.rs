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

    /// Fetch the specific records whose `record_id` is in `ids`, for the given
    /// dataset. Used by the citation manifest (PR-003) so the reproducibility
    /// hash is computed over the insight's *actual* evidence records rather than
    /// an arbitrary 500-row page head — two reviewers with the same data must get
    /// the same hash regardless of row ordering. `MemoryStore` overrides this for
    /// efficiency; the default pages through and filters, which is correct (if
    /// slower) for the remote backends.
    async fn get_by_ids(
        &self,
        dataset_id: &DatasetId,
        ids: &[String],
    ) -> Result<Vec<NormalizedRecord>> {
        let want: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
        let mut out = Vec::with_capacity(ids.len());
        // Page through the whole dataset in 500-row pages until exhausted.
        let mut offset = 0usize;
        loop {
            let page = self.get_page(dataset_id, offset, 500).await?;
            let remaining = page.records;
            let got = remaining.len();
            for r in remaining {
                if want.contains(r.record_id.as_str()) {
                    out.push(r);
                }
            }
            if got < 500 || out.len() >= ids.len() {
                break;
            }
            offset += got;
        }
        Ok(out)
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use hkgov_common::{Cadence, Category, DataSource, NormalizedRecord, RecordValue};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    /// Build a [`NormalizedRecord`] with a single `value` field — the shape all
    /// the `get_page` / `get_by_ids` assertions index against.
    fn make_record(id: &str, val: f64) -> NormalizedRecord {
        let mut fields = BTreeMap::new();
        fields.insert("value".into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "test".into(),
            record_id: id.into(),
            fields,
            fetched_at: Utc::now(),
        }
    }

    async fn register_test(store: &MemoryStore, id: &DatasetId) {
        store
            .register(
                id.clone(),
                "Test Dataset".into(),
                Some("for unit tests".into()),
                300,
                Category::Monetary,
                vec!["hibor".into()],
                Cadence::Monthly,
            )
            .await;
    }

    #[tokio::test]
    async fn memory_store_put_and_get() {
        let store = MemoryStore::new(1000, 300);
        let id = DatasetId::new(DataSource::Hkma, "test-dataset");
        let records = vec![make_record("r1", 1.0), make_record("r2", 2.0)];
        store.put_dataset(&id, records).await.unwrap();

        let page = store.get_page(&id, 0, 10).await.unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.total, 2);
        assert_eq!(page.offset, 0);
        assert_eq!(page.limit, 10);
        assert_eq!(page.source, DataSource::Hkma);
        assert_eq!(page.dataset, "test-dataset");
    }

    #[tokio::test]
    async fn memory_store_pagination() {
        let store = MemoryStore::new(1000, 300);
        let id = DatasetId::new(DataSource::Hkma, "paged");
        let records: Vec<_> = (0..10)
            .map(|i| make_record(&format!("r{i}"), i as f64))
            .collect();
        store.put_dataset(&id, records).await.unwrap();

        // First page of 3.
        let page = store.get_page(&id, 0, 3).await.unwrap();
        assert_eq!(page.records.len(), 3);
        assert_eq!(page.total, 10);
        assert_eq!(page.offset, 0);
        assert_eq!(page.limit, 3);

        // Middle page.
        let page = store.get_page(&id, 3, 3).await.unwrap();
        assert_eq!(page.records.len(), 3);
        assert_eq!(page.total, 10);
        assert_eq!(page.offset, 3);

        // Tail page returns the remainder.
        let page = store.get_page(&id, 8, 3).await.unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.total, 10);
    }

    #[tokio::test]
    async fn memory_store_meta() {
        let store = MemoryStore::new(1000, 300);
        let id = DatasetId::new(DataSource::Hkma, "meta-dataset");
        register_test(&store, &id).await;

        // Before any put_dataset, meta shows zero records and no refresh time.
        let meta = store.meta(&id).await.unwrap().expect("registered dataset");
        assert_eq!(meta.title, "Test Dataset");
        assert_eq!(meta.description.as_deref(), Some("for unit tests"));
        assert_eq!(meta.category, Category::Monetary);
        assert_eq!(meta.cadence, Cadence::Monthly);
        assert_eq!(meta.refresh_interval_secs, 300);
        assert_eq!(meta.tags, vec!["hibor"]);
        assert_eq!(meta.record_count, 0);
        assert!(meta.last_refreshed_at.is_none());

        // After put_dataset, the refresh timestamp and count appear.
        store
            .put_dataset(&id, vec![make_record("r1", 1.0)])
            .await
            .unwrap();
        let meta = store.meta(&id).await.unwrap().expect("registered dataset");
        assert_eq!(meta.record_count, 1);
        assert!(meta.last_refreshed_at.is_some());
    }

    #[tokio::test]
    async fn memory_store_empty_dataset() {
        let store = MemoryStore::new(1000, 300);
        let id = DatasetId::new(DataSource::Hkma, "uncached");
        let err = store.get_page(&id, 0, 10).await.unwrap_err();
        assert!(
            matches!(err, hkgov_common::Error::Store(_)),
            "expected Store error for uncached dataset"
        );
    }

    #[tokio::test]
    async fn memory_store_register_then_list() {
        let store = MemoryStore::new(1000, 300);
        let id_a = DatasetId::new(DataSource::Hkma, "ds-a");
        let id_b = DatasetId::new(DataSource::DataGovHk, "ds-b");
        register_test(&store, &id_a).await;
        register_test(&store, &id_b).await;

        // list(None) returns both, regardless of source.
        let all = store.list(None).await.unwrap();
        assert_eq!(all.len(), 2);

        // list filtered by source returns only that source's datasets.
        let hkma_only = store.list(Some(DataSource::Hkma)).await.unwrap();
        assert_eq!(hkma_only.len(), 1);
        assert_eq!(hkma_only[0].source, DataSource::Hkma);
        assert_eq!(hkma_only[0].dataset, "ds-a");
    }

    #[tokio::test]
    async fn memory_store_get_by_ids() {
        let store = MemoryStore::new(1000, 300);
        let id = DatasetId::new(DataSource::Hkma, "by-id");
        store
            .put_dataset(
                &id,
                vec![
                    make_record("r1", 1.0),
                    make_record("r2", 2.0),
                    make_record("r3", 3.0),
                ],
            )
            .await
            .unwrap();

        let got = store
            .get_by_ids(&id, &["r1".into(), "r3".into()])
            .await
            .unwrap();
        let mut ids: Vec<&str> = got.iter().map(|r| r.record_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["r1", "r3"]);
    }

    #[tokio::test]
    async fn memory_store_atomic_put() {
        // Regression test for the non-atomic put_dataset race (Fix 1): after
        // put_dataset completes, meta() must already reflect the new records and
        // the new last_refreshed_at together — never new records with a stale
        // (or absent) timestamp.
        let store = MemoryStore::new(1000, 300);
        let id = DatasetId::new(DataSource::Hkma, "atomic");
        register_test(&store, &id).await;

        let before = store.meta(&id).await.unwrap().unwrap();
        assert!(before.last_refreshed_at.is_none());

        store
            .put_dataset(&id, vec![make_record("r1", 1.0)])
            .await
            .unwrap();

        let after = store.meta(&id).await.unwrap().unwrap();
        assert_eq!(after.record_count, 1, "records visible immediately");
        let refreshed = after
            .last_refreshed_at
            .expect("refresh timestamp visible with records");
        assert!(
            refreshed <= Utc::now(),
            "refresh timestamp is not in the future"
        );
    }

    #[tokio::test]
    async fn record_store_trait_object() {
        // MemoryStore must be usable behind `Arc<dyn RecordStore>` — the whole
        // point of the trait is that the API layer depends on the dyn, not the
        // concrete impl.
        let store: Arc<dyn RecordStore> = Arc::new(MemoryStore::new(1000, 300));
        let id = DatasetId::new(DataSource::Hkma, "dyn");
        store
            .put_dataset(&id, vec![make_record("r1", 1.0)])
            .await
            .unwrap();
        let page = store.get_page(&id, 0, 10).await.unwrap();
        assert_eq!(page.records.len(), 1);
    }

    // ---- D-029: record_count must survive a record-cache TTL eviction ----
    //
    // Before the fix, `record_count` was derived live from the moka cache, so
    // once the TTL elapsed the entry was evicted and `/v1/sources` showed 0
    // records for every dataset until the next refresh — breaking the catalog,
    // the silence index, and `is_degraded`/`/ready`. The persisted count in the
    // registry (not subject to TTL) is now the fallback.
    #[tokio::test]
    async fn record_count_survives_cache_ttl_eviction() {
        // A 1-second TTL so the eviction happens within the test.
        let store = MemoryStore::new(100, 1);
        let id = DatasetId::new(DataSource::Hkma, "d029");
        register_test(&store, &id).await;
        store
            .put_dataset(
                &id,
                vec![
                    make_record("2026-01", 1.0),
                    make_record("2026-02", 2.0),
                    make_record("2026-03", 3.0),
                ],
            )
            .await
            .unwrap();

        // Immediately: live count is 3.
        let meta = store.meta(&id).await.unwrap().unwrap();
        assert_eq!(meta.record_count, 3);

        // Wait past the TTL so moka evicts the entry.
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        // moka evicts lazily: a `get` past the TTL returns None and removes the
        // entry. `meta()` performs that get internally, so the persisted count
        // fallback is what the catalog now reports.
        let _ = store.meta(&id).await; // touch → trigger lazy eviction

        // D-029: the catalog must still report 3 (persisted), not 0 (evicted).
        let meta_after = store.meta(&id).await.unwrap().unwrap();
        assert_eq!(
            meta_after.record_count, 3,
            "record_count must survive cache TTL eviction (D-029); got {}",
            meta_after.record_count
        );
        assert!(meta_after.last_refreshed_at.is_some());

        // And /v1/sources (list) agrees.
        let listed = store.list(None).await.unwrap();
        let entry = listed.iter().find(|m| m.dataset == "d029").unwrap();
        assert_eq!(entry.record_count, 3);
    }
}
