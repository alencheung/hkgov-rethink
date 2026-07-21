//! Dataset lineage — M1 (Open Data Gateway Foundation).
//!
//! A sidecar store tracking where each dataset came from and whether its
//! contents have changed: the upstream URL, the wire format, a content hash
//! over the records, and the fetch timestamp.
//!
//! content_sha256 is computed over the canonical (sorted) record slice using
//! the same NaN/Inf-safe approach as cite.rs: finite floats use serde_json
//! shortest-round-trip form; non-finite floats get distinct tags.

use crate::DatasetId;
use chrono::{DateTime, Utc};
use hkgov_common::NormalizedRecord;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamFormat {
    HkmaJson,
    JsonArray,
    JsonApi,
    Csv,
    HtmlNextData,
    HtmlTable,
    Feed,
    #[default]
    Unknown,
}

impl UpstreamFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HkmaJson => "hkma_json",
            Self::JsonArray => "json_array",
            Self::JsonApi => "json_api",
            Self::Csv => "csv",
            Self::HtmlNextData => "html_next_data",
            Self::HtmlTable => "html_table",
            Self::Feed => "feed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetLineage {
    pub source: hkgov_common::DataSource,
    pub dataset: String,
    pub upstream_url: String,
    pub upstream_format: UpstreamFormat,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub content_sha256: String,
    pub last_fetched_at: DateTime<Utc>,
    pub record_count_at_fetch: usize,
}

/// Serde default for `schema_version`. Referenced via the `default = "..."`
/// attribute on the field; the `#[allow]` silences the false-positive
/// dead-code lint (serde's attribute isn't visible to the lint).
#[allow(dead_code)]
fn default_schema_version() -> String {
    "1".to_string()
}

#[allow(clippy::too_many_arguments)]
pub fn lineage_from(
    dataset_id: &DatasetId,
    upstream_url: impl Into<String>,
    upstream_format: UpstreamFormat,
    schema_version: impl Into<String>,
    records: &[NormalizedRecord],
    now: DateTime<Utc>,
) -> DatasetLineage {
    DatasetLineage {
        source: dataset_id.source,
        dataset: dataset_id.dataset.clone(),
        upstream_url: upstream_url.into(),
        upstream_format,
        schema_version: schema_version.into(),
        content_sha256: content_hash(records),
        last_fetched_at: now,
        record_count_at_fetch: records.len(),
    }
}

/// SHA-256 over a canonical (sorted by record_id) record slice. NaN/Inf-safe.
pub fn content_hash(records: &[NormalizedRecord]) -> String {
    let mut hasher = Sha256::new();
    let mut canonical: Vec<&NormalizedRecord> = records.iter().collect();
    canonical.sort_by(|a, b| a.record_id.cmp(&b.record_id));
    for rec in canonical {
        hasher.update(rec.record_id.as_bytes());
        hasher.update(b"\x01");
        for (k, v) in &rec.fields {
            hasher.update(k.as_bytes());
            hasher.update(b"\x00");
            hasher.update(canonical_record_value(v).as_bytes());
            hasher.update(b"\x00");
        }
        hasher.update(b"\x01");
    }
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

fn canonical_record_value(v: &hkgov_common::RecordValue) -> String {
    use hkgov_common::RecordValue;
    match v {
        RecordValue::Null => "null".to_string(),
        RecordValue::Bool(b) => b.to_string(),
        RecordValue::Int(i) => i.to_string(),
        RecordValue::Float(f) => canonical_f64(*f),
        RecordValue::Str(s) => s.clone(),
    }
}

fn canonical_f64(f: f64) -> String {
    if f.is_nan() {
        "\"__nan__\"".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "\"__inf__\"".into()
        } else {
            "\"__ninf__\"".into()
        }
    } else {
        serde_json::to_string(&f).unwrap_or_else(|_| "null".into())
    }
}

#[derive(Debug, Default, Clone)]
pub struct LineageStore {
    inner: Arc<RwLock<HashMap<DatasetId, DatasetLineage>>>,
}

impl LineageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn record(&self, lineage: DatasetLineage) {
        let id = DatasetId::new(lineage.source, lineage.dataset.clone());
        let mut inner = self.inner.write().await;
        inner.insert(id, lineage);
    }

    pub async fn get(&self, id: &DatasetId) -> Option<DatasetLineage> {
        let inner = self.inner.read().await;
        inner.get(id).cloned()
    }

    pub async fn list(&self, source: Option<hkgov_common::DataSource>) -> Vec<DatasetLineage> {
        let inner = self.inner.read().await;
        let mut out: Vec<DatasetLineage> = inner
            .iter()
            .filter(|(id, _)| source.is_none_or(|s| s == id.source))
            .map(|(_, v)| v.clone())
            .collect();
        out.sort_by(|a, b| {
            a.source
                .to_string()
                .cmp(&b.source.to_string())
                .then_with(|| a.dataset.cmp(&b.dataset))
        });
        out
    }

    pub async fn snapshot(&self) -> Vec<DatasetLineage> {
        let inner = self.inner.read().await;
        let mut out: Vec<DatasetLineage> = inner.values().cloned().collect();
        out.sort_by(|a, b| {
            a.source
                .to_string()
                .cmp(&b.source.to_string())
                .then_with(|| a.dataset.cmp(&b.dataset))
        });
        out
    }

    pub async fn restore(&self, snapshot: Vec<DatasetLineage>) {
        let mut inner = self.inner.write().await;
        inner.clear();
        for lineage in snapshot {
            let id = DatasetId::new(lineage.source, lineage.dataset.clone());
            inner.insert(id, lineage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use hkgov_common::{DataSource, NormalizedRecord, RecordValue};
    use std::collections::BTreeMap;

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

    fn float_rec(id: &str, val: f64) -> NormalizedRecord {
        make_record(id, val)
    }

    #[test]
    fn content_hash_is_order_independent() {
        let a = vec![make_record("r1", 1.0), make_record("r2", 2.0)];
        let b = vec![make_record("r2", 2.0), make_record("r1", 1.0)];
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn content_hash_detects_value_drift() {
        let original = vec![make_record("r1", 1.0)];
        let revised = vec![make_record("r1", 2.5)];
        assert_ne!(content_hash(&original), content_hash(&revised));
    }

    #[test]
    fn content_hash_distinguishes_nan_from_null() {
        let mut nan_fields = BTreeMap::new();
        nan_fields.insert("value".into(), RecordValue::Float(f64::NAN));
        let nan_rec = NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "test".into(),
            record_id: "r1".into(),
            fields: nan_fields,
            fetched_at: Utc::now(),
        };
        let mut null_fields = BTreeMap::new();
        null_fields.insert("value".into(), RecordValue::Null);
        let null_rec = NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "test".into(),
            record_id: "r1".into(),
            fields: null_fields,
            fetched_at: Utc::now(),
        };
        assert_ne!(content_hash(&[nan_rec]), content_hash(&[null_rec]));
    }

    #[test]
    fn content_hash_distinguishes_inf_signs() {
        let pos = vec![float_rec("r1", f64::INFINITY)];
        let neg = vec![float_rec("r1", f64::NEG_INFINITY)];
        assert_ne!(content_hash(&pos), content_hash(&neg));
    }

    #[tokio::test]
    async fn lineage_store_record_and_get() {
        let store = LineageStore::new();
        let id = DatasetId::new(DataSource::Hkma, "ds");
        let lineage = lineage_from(
            &id,
            "https://example.test/api",
            UpstreamFormat::HkmaJson,
            "1",
            &[make_record("r1", 1.0)],
            Utc::now(),
        );
        store.record(lineage.clone()).await;
        let got = store.get(&id).await.expect("lineage recorded");
        assert_eq!(got.upstream_url, "https://example.test/api");
        assert_eq!(got.upstream_format, UpstreamFormat::HkmaJson);
        assert_eq!(got.record_count_at_fetch, 1);
        assert!(!got.content_sha256.is_empty());
    }

    #[tokio::test]
    async fn lineage_store_list_filtered_by_source() {
        let store = LineageStore::new();
        let id_a = DatasetId::new(DataSource::Hkma, "ds-a");
        let id_b = DatasetId::new(DataSource::Rvd, "ds-b");
        store
            .record(lineage_from(
                &id_a,
                "https://a",
                UpstreamFormat::HkmaJson,
                "1",
                &[make_record("r1", 1.0)],
                Utc::now(),
            ))
            .await;
        store
            .record(lineage_from(
                &id_b,
                "https://b",
                UpstreamFormat::Csv,
                "1",
                &[make_record("r1", 1.0)],
                Utc::now(),
            ))
            .await;

        let all = store.list(None).await;
        assert_eq!(all.len(), 2);

        let hkma_only = store.list(Some(DataSource::Hkma)).await;
        assert_eq!(hkma_only.len(), 1);
        assert_eq!(hkma_only[0].source, DataSource::Hkma);
    }

    #[tokio::test]
    async fn lineage_snapshot_round_trip() {
        let store = LineageStore::new();
        let id = DatasetId::new(DataSource::Hkma, "ds");
        store
            .record(lineage_from(
                &id,
                "https://example",
                UpstreamFormat::HkmaJson,
                "1",
                &[make_record("r1", 1.0)],
                Utc::now(),
            ))
            .await;
        let snap = store.snapshot().await;
        assert_eq!(snap.len(), 1);

        let restored = LineageStore::new();
        restored.restore(snap).await;
        let got = restored.get(&id).await.expect("restored");
        assert_eq!(got.upstream_url, "https://example");
    }
}
