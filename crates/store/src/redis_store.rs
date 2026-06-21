//! Redis-backed `RecordStore` — the multi-node cache tier (ROADMAP v2).
//!
//! Enables a fleet of stateless API nodes to share one hot cache. Records are
//! stored as JSON under `hkgov:{source}:{dataset}` keys, with a separate
//! `hkgov:meta:{source}:{dataset}` key for metadata. TTLs mirror the in-memory
//! store so caches self-expire between refresh ticks.
//!
//! This is behind the `redis` feature so the default build (and CI) need no
//! Redis server. The API binary selects the store at startup based on config.

use crate::{DatasetId, RecordPage, RecordStore};
use async_trait::async_trait;
use hkgov_common::{DataSource, DatasetMeta, Error, NormalizedRecord, Result};
use redis::AsyncCommands;

/// Connection-string-based Redis store. Uses a single multiplexed connection
/// (`redis::aio::MultiplexedConnection`), which is cheap to clone and shares
/// one socket across all callers — sufficient for the v2 single-cluster tier.
pub struct RedisStore {
    conn: redis::aio::MultiplexedConnection,
    ttl_secs: u64,
}

impl RedisStore {
    pub async fn connect(url: &str, ttl_secs: u64) -> Result<Self> {
        let conn = redis::Client::open(url)
            .map_err(|e| Error::Store(format!("redis connect: {e}")))?
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| Error::Store(format!("redis connect: {e}")))?;
        Ok(Self { conn, ttl_secs })
    }

    fn records_key(id: &DatasetId) -> String {
        format!("hkgov:{}:{}:records", id.source, id.dataset)
    }

    fn meta_key(id: &DatasetId) -> String {
        format!("hkgov:{}:{}:meta", id.source, id.dataset)
    }

    fn index_key() -> &'static str {
        "hkgov:_index"
    }
}

#[async_trait]
impl RecordStore for RedisStore {
    async fn put_dataset(
        &self,
        dataset_id: &DatasetId,
        records: Vec<NormalizedRecord>,
    ) -> Result<()> {
        let payload =
            serde_json::to_vec(&records).map_err(|e| Error::Store(format!("serialize: {e}")))?;
        let rkey = Self::records_key(dataset_id);
        let mkey = Self::meta_key(dataset_id);
        let now = chrono::Utc::now();
        let ttl = self.ttl_secs;

        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.set_ex(&rkey, payload, ttl);
        // Meta is a best-effort JSON blob; we only track refresh time here.
        // Static registry (title/description/cadence) is held in-process by the
        // MemoryStore registry and merged at read time by the API.
        pipe.set_ex(
            &mkey,
            serde_json::to_vec(&DatasetMeta {
                source: dataset_id.source,
                dataset: dataset_id.dataset.clone(),
                title: String::new(),
                description: None,
                refresh_interval_secs: 0,
                last_refreshed_at: Some(now),
                record_count: records.len(),
            })
            .unwrap_or_default(),
            ttl,
        );
        pipe.sadd(
            Self::index_key(),
            format!("{}:{}", dataset_id.source, dataset_id.dataset),
        );

        let mut conn = self.conn.clone();
        pipe.query_async::<()>(&mut conn)
            .await
            .map_err(|e| Error::Store(format!("redis put: {e}")))?;
        tracing::debug!(
            source = %dataset_id.source,
            dataset = %dataset_id.dataset,
            count = records.len(),
            "redis: dataset stored"
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
        let rkey = Self::records_key(dataset_id);
        let mut conn = self.conn.clone();
        let bytes: Vec<u8> = conn
            .get::<&str, Option<Vec<u8>>>(&rkey)
            .await
            .map_err(|e| Error::Store(format!("redis get: {e}")))?
            .ok_or_else(|| {
                Error::Store(format!(
                    "no records cached for {}/{}",
                    dataset_id.source, dataset_id.dataset
                ))
            })?;
        let records: Vec<NormalizedRecord> = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Store(format!("deserialize: {e}")))?;
        let total = records.len();
        let page = records.into_iter().skip(offset).take(limit).collect();
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
        let mkey = Self::meta_key(dataset_id);
        let mut conn = self.conn.clone();
        let bytes: Option<Vec<u8>> = conn
            .get::<&str, Option<Vec<u8>>>(&mkey)
            .await
            .map_err(|e| Error::Store(format!("redis meta: {e}")))?;
        match bytes {
            None => Ok(None),
            Some(b) => {
                let mut m: DatasetMeta = serde_json::from_slice(&b)
                    .map_err(|e| Error::Store(format!("deserialize meta: {e}")))?;
                if m.title.is_empty() {
                    m.title = dataset_id.dataset.clone();
                }
                Ok(Some(m))
            }
        }
    }

    async fn list(&self, source: Option<DataSource>) -> Result<Vec<DatasetMeta>> {
        let mut conn = self.conn.clone();
        let members: Vec<String> = conn
            .smembers(Self::index_key())
            .await
            .map_err(|e| Error::Store(format!("redis list: {e}")))?;
        let mut out = Vec::new();
        for m in members {
            let parts: Vec<&str> = m.splitn(2, ':').collect();
            if parts.len() != 2 {
                continue;
            }
            let ds = DataSource::parse(parts[0]).unwrap_or(DataSource::Hkma);
            if source.is_some_and(|s| s != ds) {
                continue;
            }
            let id = DatasetId::new(ds, parts[1]);
            if let Ok(Some(meta)) = self.meta(&id).await {
                out.push(meta);
            }
        }
        Ok(out)
    }
}
