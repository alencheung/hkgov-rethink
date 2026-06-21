//! Ingestion scheduler.
//!
//! Owns the loop that keeps the [`RecordStore`] warm. For each (source,
//! dataset) registered in the connector [`Registry`] it kicks off a refresh
//! task at the cadence the connector declared.
//!
//! Design notes:
//! - Each dataset gets its own task. A slow/failed dataset never blocks others.
//! - Failures are logged and retried on the next tick — we never panic the
//!   supervisor.
//! - Initial refresh is sequential-per-source so we don't hammer HKMA at boot;
//!   steady-state refreshes are naturally staggered by their different cadences.

use hkgov_common::{DataSource, Result};
use hkgov_connectors::registry::Registry;
use hkgov_store::{DatasetId, MemoryStore, RecordStore};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

pub struct IngestSupervisor {
    handles: Vec<JoinHandle<()>>,
}

impl IngestSupervisor {
    /// Spawn the supervisor. Returns immediately; refresh tasks run in the
    /// background for the lifetime of the runtime.
    pub fn spawn(registry: Arc<Registry>, store: Arc<MemoryStore>) -> Self {
        let mut handles = Vec::new();

        // First, register metadata for every dataset so `/sources` is correct
        // even before the first fetch returns.
        for (source, spec) in registry.all_datasets() {
            let connector = registry.lookup(source).expect("source present");
            let store = store.clone();
            let spec_id = spec.id.to_string();
            let title = spec.title.to_string();
            let description = spec.description.map(|s| s.to_string());
            let interval = spec.refresh_interval_secs;

            let handle = tokio::spawn(async move {
                let id = DatasetId::new(source, spec_id.clone());
                store
                    .register(id.clone(), title, description, interval)
                    .await;

                // Initial warm.
                refresh_once(source, &connector, &store, &spec_id).await;

                let mut ticker = tokio::time::interval(Duration::from_secs(interval.max(30)));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                ticker.tick().await; // skip immediate (already warmed)
                loop {
                    ticker.tick().await;
                    refresh_once(source, &connector, &store, &spec_id).await;
                }
            });
            handles.push(handle);
        }

        IngestSupervisor { handles }
    }

    /// Best-effort shutdown.
    pub fn abort_all(&self) {
        for h in &self.handles {
            h.abort();
        }
    }
}

async fn refresh_once(
    source: DataSource,
    connector: &Arc<dyn hkgov_connectors::Connector>,
    store: &Arc<MemoryStore>,
    dataset: &str,
) {
    let id = DatasetId::new(source, dataset);
    match connector.fetch(dataset).await {
        Ok(records) => {
            let count = records.len();
            if let Err(e) = store.put_dataset(&id, records).await {
                tracing::warn!(source = %source, dataset, error = %e, "ingest: store put failed");
            } else {
                tracing::info!(source = %source, dataset, count, "ingest: refreshed");
            }
        }
        Err(e) => {
            tracing::warn!(source = %source, dataset, error = %e, "ingest: fetch failed");
        }
    }
}

/// Pull a single dataset once. Used by API on-demand refresh and tests.
pub async fn fetch_once(
    source: DataSource,
    connector: &Arc<dyn hkgov_connectors::Connector>,
    store: &Arc<MemoryStore>,
    dataset: &str,
) -> Result<usize> {
    let records = connector.fetch(dataset).await?;
    let count = records.len();
    let id = DatasetId::new(source, dataset);
    store.put_dataset(&id, records).await?;
    Ok(count)
}
