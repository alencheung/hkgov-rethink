//! Hong Kong Monetary Authority (HKMA) Open API connector.
//!
//! Verified live against `https://api.hkma.gov.hk/public/...`. Upstream returns
//! a stable envelope:
//!
//! ```jsonc
//! {
//!   "header": { "success": true, "err_code": "0000", "err_msg": "..." },
//!   "result": { "datasize": 3, "records": [ { ...row... }, ... ] }
//! }
//! ```
//!
//! Fields per record are dataset-specific and sparse (many `null`s), so we keep
//! them as `serde_json::Value` and normalize into [`RecordValue`] cells. That
//! keeps this connector resilient when HKMA adds new columns.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

/// The subset of datasets we expose in v1. All slugs are real HKMA
/// documentation paths under `market-data-and-statistics/monthly-statistical-bulletin`.
const DATASETS: &[DatasetSpec] = &[
    DatasetSpec {
        id: "capital-market-statistics",
        title: "Capital Market Statistics (T0103)",
        description: Some("Monthly capital market statistics from the HKMA Monthly Statistical Bulletin, Section Financial."),
        refresh_interval_secs: 6 * 3600,
    },
    DatasetSpec {
        id: "daily-interbank-liquidity",
        title: "Daily Interbank Liquidity Figures",
        description: Some("Daily figures of the discount window and liquidity adjustment window."),
        refresh_interval_secs: 3600,
    },
];

pub struct HkmaConnector {
    base_url: String,
    max_retries: u32,
    client: reqwest::Client,
}

impl HkmaConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_millis(settings.hkma_timeout_ms))
            .gzip(true)
            .pool_max_idle_per_host(32)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")));

        if let Some(key) = settings.hkma_api_key.as_deref() {
            let mut headers = reqwest::header::HeaderMap::new();
            if let Ok(v) = reqwest::header::HeaderValue::from_str(key) {
                headers.insert("X-API-KEY", v);
            }
            builder = builder.default_headers(headers);
        }

        let client = builder
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;

        Ok(Self {
            base_url: settings.hkma_base_url.trim_end_matches('/').to_string(),
            max_retries: settings.hkma_max_retries,
            client,
        })
    }

    /// Build the path for a known dataset slug. Kept explicit (not a lookup
    /// table) so the path can be read at a glance.
    fn path_for(&self, dataset: &str) -> Option<String> {
        match dataset {
            "capital-market-statistics" => Some(format!(
                "{}/market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics",
                self.base_url
            )),
            "daily-interbank-liquidity" => Some(format!(
                "{}/market-data-and-statistics/daily-figures-interbank-liquidity",
                self.base_url
            )),
            _ => None,
        }
    }

    /// Single GET with bounded exponential backoff. Retries are safe: HKMA
    /// endpoints are idempotent reads.
    async fn get_with_retry(&self, url: &str) -> Result<serde_json::Value> {
        let mut last_err: Option<Error> = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_millis(200 * (1u64 << (attempt.min(6))));
                tokio::time::sleep(backoff).await;
            }

            tracing::debug!(attempt, url, "hkma request");
            let req = self.client.get(url);
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(Error::Upstream {
                        origin: "hkma",
                        status: 0,
                        detail: format!("transport: {e}"),
                    });
                    continue;
                }
            };

            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                let detail = resp.text().await.unwrap_or_default();
                last_err = Some(Error::Upstream {
                    origin: "hkma",
                    status,
                    detail,
                });
                // 4xx other than 429 won't fix themselves; stop early.
                if (400..500).contains(&status) && status != 429 {
                    break;
                }
                continue;
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| Error::Decode {
                origin: "hkma",
                backtrace: serde::de::Error::custom(e.to_string()),
            })?;

            return Ok(json);
        }
        Err(last_err.unwrap_or_else(|| Error::Upstream {
            origin: "hkma",
            status: 0,
            detail: "exhausted retries".to_string(),
        }))
    }
}

/// HKMA response envelope — see module docs.
#[derive(Debug, Deserialize)]
struct HkmaEnvelope {
    header: HkmaHeader,
    result: HkmaResult,
}

#[derive(Debug, Deserialize)]
struct HkmaHeader {
    success: bool,
    #[serde(default)]
    err_code: String,
    #[serde(default)]
    err_msg: String,
}

#[derive(Debug, Deserialize)]
struct HkmaResult {
    #[serde(default)]
    datasize: u64,
    #[serde(default)]
    records: Vec<serde_json::Value>,
}

#[async_trait]
impl Connector for HkmaConnector {
    fn source(&self) -> DataSource {
        DataSource::Hkma
    }

    fn datasets(&self) -> &[DatasetSpec] {
        DATASETS
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let base = self.path_for(dataset).ok_or_else(|| {
            Error::Internal(format!("hkma: no path mapping for dataset {dataset}"))
        })?;
        // pagesize=1000 is the documented HKMA maximum.
        let url = format!("{base}?pagesize=1000");

        let json = self.get_with_retry(&url).await?;
        let env: HkmaEnvelope = serde_json::from_value(json).map_err(|e| Error::Decode {
            origin: "hkma",
            backtrace: e,
        })?;

        if !env.header.success {
            return Err(Error::Upstream {
                origin: "hkma",
                status: 200,
                detail: format!("{}: {}", env.header.err_code, env.header.err_msg),
            });
        }

        let now = Utc::now();
        let records = env
            .result
            .records
            .into_iter()
            .map(|raw| {
                let fields = normalize_row(&raw);
                let record_id = record_id_for(dataset, &fields);
                NormalizedRecord {
                    source: DataSource::Hkma,
                    dataset: dataset.to_string(),
                    record_id,
                    fields,
                    fetched_at: now,
                }
            })
            .collect();

        tracing::info!(
            dataset,
            count = env.result.datasize,
            "hkma: fetched dataset"
        );
        Ok(records)
    }
}

/// Convert a raw JSON object into our [`RecordValue`] map.
fn normalize_row(raw: &serde_json::Value) -> BTreeMap<String, RecordValue> {
    let Some(obj) = raw.as_object() else {
        return BTreeMap::new();
    };
    obj.iter()
        .map(|(k, v)| (k.clone(), json_to_value(v)))
        .collect()
}

fn json_to_value(v: &serde_json::Value) -> RecordValue {
    match v {
        serde_json::Value::Null => RecordValue::Null,
        serde_json::Value::Bool(b) => RecordValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                RecordValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                RecordValue::Float(f)
            } else {
                RecordValue::Null
            }
        }
        serde_json::Value::String(s) => RecordValue::Str(s.clone()),
        // Flatten arrays/objects to their compact JSON string. Keeps the cell
        // scalar-friendly for downstream serialization and AI ingestion.
        other => RecordValue::Str(other.to_string()),
    }
}

/// Derive a stable per-record id. For monthly statistics HKMA keys on
/// `end_of_month`; daily ones on `date`. Fall back to a hash so we always have
/// *something* stable.
fn record_id_for(dataset: &str, fields: &BTreeMap<String, RecordValue>) -> String {
    let candidates: &[&str] = match dataset {
        "capital-market-statistics" => &["end_of_month"],
        "daily-interbank-liquidity" => &["date", "end_of_date"],
        _ => &[],
    };
    for key in candidates {
        if let Some(RecordValue::Str(s)) = fields.get(*key) {
            return s.clone();
        }
        if let Some(RecordValue::Int(i)) = fields.get(*key) {
            return i.to_string();
        }
    }
    // Deterministic fallback.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (k, v) in fields {
        k.hash(&mut h);
        format!("{v:?}").hash(&mut h);
    }
    format!("id-{:016x}", h.finish())
}

/// Public test helper: expose normalization so unit tests can assert against
/// fixture payloads without going to the network.
pub(crate) fn _test_normalize(raw: &serde_json::Value) -> BTreeMap<String, RecordValue> {
    normalize_row(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the shape of a real HKMA capital-market-statistics record
    /// (captured live, trimmed for the test).
    const SAMPLE: &str = r#"{
        "header": {"success": true, "err_code": "0000", "err_msg": "No error found"},
        "result": {
            "datasize": 1,
            "records": [
                {
                    "end_of_month": "2026-05",
                    "hkd_drmkt_outstand_efbn": 1354062,
                    "hkd_drmkt_outstand_odrinst": null,
                    "eq_mkt_hs_index": 25182.39,
                    "eq_mkt_ttl_stock_cap": 47078571.017408
                }
            ]
        }
    }"#;

    #[test]
    fn parses_envelope_and_normalizes_row() {
        let v: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        let env: HkmaEnvelope = serde_json::from_value(v).unwrap();
        assert!(env.header.success);
        assert_eq!(env.result.datasize, 1);
        let row = &env.result.records[0];
        let fields = _test_normalize(row);
        assert_eq!(
            fields.get("end_of_month"),
            Some(&RecordValue::Str("2026-05".into()))
        );
        assert_eq!(
            fields.get("hkd_drmkt_outstand_efbn"),
            Some(&RecordValue::Int(1354062))
        );
        assert_eq!(
            fields.get("hkd_drmkt_outstand_odrinst"),
            Some(&RecordValue::Null)
        );
        // float preserved
        match fields.get("eq_mkt_hs_index") {
            Some(RecordValue::Float(f)) => assert!((f - 25182.39).abs() < 1e-6),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn record_id_prefers_end_of_month() {
        let mut fields = BTreeMap::new();
        fields.insert("end_of_month".into(), RecordValue::Str("2026-05".into()));
        let id = record_id_for("capital-market-statistics", &fields);
        assert_eq!(id, "2026-05");
    }

    #[cfg(feature = "live")]
    #[tokio::test]
    async fn live_fetch_capital_market() {
        use hkgov_common::Settings;
        let s = Settings::default();
        let c = HkmaConnector::new(&s.upstream).unwrap();
        let records = c.fetch("capital-market-statistics").await.unwrap();
        assert!(!records.is_empty(), "expected live records");
        assert!(records.iter().all(|r| r.source == DataSource::Hkma));
    }
}
