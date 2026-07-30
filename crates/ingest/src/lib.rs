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
use hkgov_store::{lineage_from, DatasetId, DatasetLineage, MemoryStore, RecordStore};
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
            let category = spec.category;
            let tags: Vec<String> = spec.tags.iter().map(|t| (*t).to_string()).collect();
            let cadence = spec.cadence;

            let handle = tokio::spawn(async move {
                let id = DatasetId::new(source, spec_id.clone());
                store
                    .register(
                        id.clone(),
                        title,
                        description,
                        interval,
                        category,
                        tags,
                        cadence,
                    )
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
            // SEC-CON-03: bound untrusted upstream free-text before it reaches
            // the store / API / LLM layer. Connectors store scraped addresses,
            // agent names, and price hints verbatim; a malicious source could
            // otherwise inject multi-MB strings (memory amplification + a
            // prompt-injection vector for the framing layer).
            let records = sanitize_records(records);
            let count = records.len();
            // M1: record lineage before the records move into put_dataset. The
            // content hash borrows the slice; the URL/format come from the
            // connector's optional lineage accessors (None/Unknown when the
            // connector doesn't track them — lineage is still recorded with
            // the hash + count, just without a verifiable URL).
            let lineage = build_lineage(source, dataset, connector.as_ref(), &records);
            if let Err(e) = store.put_dataset(&id, records).await {
                tracing::warn!(source = %source, dataset, error = %e, "ingest: store put failed");
            } else {
                // Record lineage only after the publish succeeded — a failed
                // put must not leave a lineage pointing at data that isn't
                // actually cached.
                store.record_lineage(lineage).await;
                tracing::info!(source = %source, dataset, count, "ingest: refreshed");
            }
        }
        Err(e) => {
            tracing::warn!(source = %source, dataset, error = %e, "ingest: fetch failed");
        }
    }
}

/// Bound the length of every `Str` field across a batch of records to
/// [`hkgov_common::MAX_FIELD_BYTES`]. Applied at the ingest chokepoint so every
/// source (HKMA, data.gov.hk, the property-portal scrapers, …) inherits the
/// cap uniformly (SEC-CON-03).
fn sanitize_records(
    mut records: Vec<hkgov_common::NormalizedRecord>,
) -> Vec<hkgov_common::NormalizedRecord> {
    use hkgov_common::{RecordValue, MAX_FIELD_BYTES};
    let cap = MAX_FIELD_BYTES;
    let dirty = records.iter().any(|r| {
        r.fields
            .values()
            .any(|v| matches!(v, RecordValue::Str(s) if s.len() > cap))
    });
    if !dirty {
        return records; // fast path: almost no record carries an oversized field
    }
    for rec in records.iter_mut() {
        for v in rec.fields.values_mut() {
            *v = std::mem::replace(v, RecordValue::Null).cap_str(cap);
        }
    }
    records
}

/// Build the lineage record for a fetch (M1). Borrows the records so the hash
/// can be computed before they move into `put_dataset`. Connectors that don't
/// implement `upstream_url`/`upstream_format` get a `None`/`Unknown` lineage —
/// the hash + count are still recorded, so drift detection works even without
/// a verifiable URL.
fn build_lineage(
    source: DataSource,
    dataset: &str,
    connector: &dyn hkgov_connectors::Connector,
    records: &[hkgov_common::NormalizedRecord],
) -> DatasetLineage {
    let id = DatasetId::new(source, dataset);
    let url = connector
        .upstream_url(dataset)
        .unwrap_or_else(|| format!("unknown:{source}/{dataset}"));
    let format = connector.upstream_format(dataset);
    // lineage_from computes the content hash internally (canonical slice,
    // order-independent + NaN-safe, mirroring cite.rs).
    lineage_from(&id, url, format, "1", records, chrono::Utc::now())
}

/// Pull a single dataset once. Used by API on-demand refresh and tests.
///
/// QUAL-ING-01: this previously wrote records WITHOUT recording lineage, so an
/// on-demand refresh left the lineage index pointing at stale data and the
/// content-hash drift detector couldn't see the refresh. It now builds +
/// records lineage exactly like the scheduled `refresh_once` path, so
/// provenance stays consistent between the two entry points.
pub async fn fetch_once(
    source: DataSource,
    connector: &Arc<dyn hkgov_connectors::Connector>,
    store: &Arc<MemoryStore>,
    dataset: &str,
) -> Result<usize> {
    let records = connector.fetch(dataset).await?;
    // SEC-CON-03: bound untrusted upstream free-text (same as refresh_once).
    let records = sanitize_records(records);
    let count = records.len();
    let id = DatasetId::new(source, dataset);
    // Build lineage before the records move into put_dataset (the hash borrows
    // the slice), then record it only after the publish succeeded — mirroring
    // refresh_once so both paths leave identical provenance.
    let lineage = build_lineage(source, dataset, connector.as_ref(), &records);
    store.put_dataset(&id, records).await?;
    store.record_lineage(lineage).await;
    Ok(count)
}
