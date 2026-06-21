//! Postgres-backed `RecordStore` — the persistent cold/historical tier
//! (ROADMAP v4). Used for unbounded historical reads alongside the hot cache.
//!
//! Schema (created lazily on connect):
//! ```sql
//! CREATE TABLE IF NOT EXISTS hkgov_records (
//!   source    TEXT NOT NULL,
//!   dataset   TEXT NOT NULL,
//!   record_id TEXT NOT NULL,
//!   fields    JSONB NOT NULL,
//!   fetched_at TIMESTAMPTZ NOT NULL,
//!   PRIMARY KEY (source, dataset, record_id)
//! );
//! CREATE TABLE IF NOT EXISTS hkgov_dataset_meta (
//!   source TEXT NOT NULL,
//!   dataset TEXT NOT NULL,
//!   last_refreshed_at TIMESTAMPTZ,
//!   PRIMARY KEY (source, dataset)
//! );
//! ```
//!
//! `put_dataset` upserts all rows for a dataset in a single transaction;
//! `get_page` reads from the table with OFFSET/LIMIT. Behind the `pg` feature
//! so the default build needs no Postgres server.

use crate::{DatasetId, RecordPage, RecordStore};
use async_trait::async_trait;
use hkgov_common::{DataSource, DatasetMeta, Error, NormalizedRecord, Result};
use tokio::sync::Mutex;
use tokio_postgres::Client;

pub struct PgStore {
    // tokio_postgres::Client::transaction() requires &mut self, so we guard the
    // connection with an async mutex. Contention is low: writes happen on the
    // ingest scheduler cadence, reads are the hot path.
    client: Mutex<Client>,
}

impl PgStore {
    pub async fn connect(url: &str) -> Result<Self> {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .map_err(|e| Error::Store(format!("pg connect: {e}")))?;

        // Drive the connection in the background.
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!(error = %e, "pg connection error");
            }
        });

        let s = Self {
            client: Mutex::new(client),
        };
        s.ensure_schema().await?;
        Ok(s)
    }

    async fn ensure_schema(&self) -> Result<()> {
        let client = self.client.lock().await;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS hkgov_records (
                    source     TEXT NOT NULL,
                    dataset    TEXT NOT NULL,
                    record_id  TEXT NOT NULL,
                    fields     JSONB NOT NULL,
                    fetched_at TIMESTAMPTZ NOT NULL,
                    PRIMARY KEY (source, dataset, record_id)
                 );
                 CREATE TABLE IF NOT EXISTS hkgov_dataset_meta (
                    source TEXT NOT NULL,
                    dataset TEXT NOT NULL,
                    last_refreshed_at TIMESTAMPTZ,
                    PRIMARY KEY (source, dataset)
                 );",
            )
            .await
            .map_err(|e| Error::Store(format!("pg schema: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl RecordStore for PgStore {
    async fn put_dataset(
        &self,
        dataset_id: &DatasetId,
        records: Vec<NormalizedRecord>,
    ) -> Result<()> {
        let mut client = self.client.lock().await;
        let tx = client
            .transaction()
            .await
            .map_err(|e| Error::Store(format!("pg tx begin: {e}")))?;

        // Replace: delete then insert, atomically.
        tx.execute(
            "DELETE FROM hkgov_records WHERE source = $1 AND dataset = $2",
            &[&dataset_id.source.as_str(), &dataset_id.dataset],
        )
        .await
        .map_err(|e| Error::Store(format!("pg delete: {e}")))?;

        let stmt = tx
            .prepare(
                "INSERT INTO hkgov_records (source, dataset, record_id, fields, fetched_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (source, dataset, record_id) DO UPDATE SET fields = EXCLUDED.fields, fetched_at = EXCLUDED.fetched_at",
            )
            .await
            .map_err(|e| Error::Store(format!("pg prepare: {e}")))?;

        let now = chrono::Utc::now();
        for r in &records {
            let fields_json = serde_json::to_value(&r.fields)
                .map_err(|e| Error::Store(format!("serialize fields: {e}")))?;
            tx.execute(
                &stmt,
                &[
                    &dataset_id.source.as_str(),
                    &dataset_id.dataset,
                    &r.record_id,
                    &fields_json,
                    &now,
                ],
            )
            .await
            .map_err(|e| Error::Store(format!("pg insert: {e}")))?;
        }

        tx.execute(
            "INSERT INTO hkgov_dataset_meta (source, dataset, last_refreshed_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (source, dataset) DO UPDATE SET last_refreshed_at = EXCLUDED.last_refreshed_at",
            &[&dataset_id.source.as_str(), &dataset_id.dataset, &now],
        )
        .await
        .map_err(|e| Error::Store(format!("pg meta upsert: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("pg commit: {e}")))?;
        tracing::debug!(
            source = %dataset_id.source,
            dataset = %dataset_id.dataset,
            count = records.len(),
            "pg: dataset stored"
        );
        Ok(())
    }

    async fn get_page(
        &self,
        dataset_id: &DatasetId,
        offset: usize,
        limit: usize,
    ) -> Result<RecordPage> {
        let limit = limit.clamp(1, 500) as i64;
        let offset = offset as i64;
        let client = self.client.lock().await;
        let rows = client
            .query(
                "SELECT record_id, fields, fetched_at FROM hkgov_records
                 WHERE source = $1 AND dataset = $2
                 ORDER BY record_id
                 LIMIT $3 OFFSET $4",
                &[
                    &dataset_id.source.as_str(),
                    &dataset_id.dataset,
                    &limit,
                    &offset,
                ],
            )
            .await
            .map_err(|e| Error::Store(format!("pg select: {e}")))?;

        let total: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM hkgov_records WHERE source = $1 AND dataset = $2",
                &[&dataset_id.source.as_str(), &dataset_id.dataset],
            )
            .await
            .map_err(|e| Error::Store(format!("pg count: {e}")))?
            .get(0);
        drop(client);

        let records = rows
            .into_iter()
            .map(|row| {
                let record_id: String = row.get(0);
                let fields_json: serde_json::Value = row.get(1);
                let fetched_at: chrono::DateTime<chrono::Utc> = row.get(2);
                let fields = serde_json::from_value(fields_json).unwrap_or_default();
                NormalizedRecord {
                    source: dataset_id.source,
                    dataset: dataset_id.dataset.clone(),
                    record_id,
                    fields,
                    fetched_at,
                }
            })
            .collect();

        Ok(RecordPage {
            source: dataset_id.source,
            dataset: dataset_id.dataset.clone(),
            total: total as usize,
            offset: offset as usize,
            limit: limit as usize,
            records,
        })
    }

    async fn meta(&self, dataset_id: &DatasetId) -> Result<Option<DatasetMeta>> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                "SELECT last_refreshed_at,
                        (SELECT COUNT(*) FROM hkgov_records r
                         WHERE r.source = m.source AND r.dataset = m.dataset)
                 FROM hkgov_dataset_meta m
                 WHERE m.source = $1 AND m.dataset = $2",
                &[&dataset_id.source.as_str(), &dataset_id.dataset],
            )
            .await
            .map_err(|e| Error::Store(format!("pg meta: {e}")))?;
        match row {
            None => Ok(None),
            Some(row) => {
                let last: Option<chrono::DateTime<chrono::Utc>> = row.get(0);
                let count: i64 = row.get(1);
                Ok(Some(DatasetMeta {
                    source: dataset_id.source,
                    dataset: dataset_id.dataset.clone(),
                    title: dataset_id.dataset.clone(),
                    description: None,
                    refresh_interval_secs: 0,
                    last_refreshed_at: last,
                    record_count: count as usize,
                }))
            }
        }
    }

    async fn list(&self, source: Option<DataSource>) -> Result<Vec<DatasetMeta>> {
        let client = self.client.lock().await;
        let rows = if let Some(src) = source {
            client
                .query(
                    "SELECT source, dataset, last_refreshed_at,
                            (SELECT COUNT(*) FROM hkgov_records r
                             WHERE r.source = m.source AND r.dataset = m.dataset)
                     FROM hkgov_dataset_meta m WHERE m.source = $1",
                    &[&src.as_str()],
                )
                .await
                .map_err(|e| Error::Store(format!("pg list: {e}")))?
        } else {
            client
                .query(
                    "SELECT source, dataset, last_refreshed_at,
                            (SELECT COUNT(*) FROM hkgov_records r
                             WHERE r.source = m.source AND r.dataset = m.dataset)
                     FROM hkgov_dataset_meta m",
                    &[],
                )
                .await
                .map_err(|e| Error::Store(format!("pg list: {e}")))?
        };
        let mut out = Vec::new();
        for row in rows {
            let src_str: String = row.get(0);
            let dataset: String = row.get(1);
            let last: Option<chrono::DateTime<chrono::Utc>> = row.get(2);
            let count: i64 = row.get(3);
            out.push(DatasetMeta {
                source: DataSource::parse(&src_str).unwrap_or(DataSource::Hkma),
                dataset,
                title: String::new(),
                description: None,
                refresh_interval_secs: 0,
                last_refreshed_at: last,
                record_count: count as usize,
            });
        }
        Ok(out)
    }
}
