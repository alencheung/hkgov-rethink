//! data.gov.hk connector.
//!
//! Targets the platform's *own* APIs (NOT CKAN — `data.gov.hk/api/3/action/*`
//! returns 404). Two endpoints are used:
//!
//! - **Filter API** (`api.data.gov.hk/v2/filter`): query a single resource by its
//!   CSV/JSON URL. Returns a bare JSON array of row objects.
//! - **Historical archive** (`app.data.gov.hk/v1/historical-archive/list-files`):
//!   list files for a provider between two dates.
//!
//! Both verified live — see docs/DATA_SOURCES.md.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings};
use std::collections::BTreeMap;
use std::time::Duration;

/// A resource exposed via the v2 filter API. Each one is a real data.gov.hk
/// resource URL — adding a dataset is adding an entry here plus a slug below.
#[derive(Debug, Clone)]
struct FilterResource {
    /// data.gov.hk resource URL (CSV/JSON/Excel). Queried verbatim.
    resource_url: &'static str,
    /// Which field (if any) uniquely identifies a row.
    id_field: Option<&'static str>,
}

fn resources() -> &'static [(&'static str, FilterResource)] {
    &[
        (
            "money-lenders-licensees",
            FilterResource {
                resource_url: "http://www.cr.gov.hk/datagovhk/psi/ml_licensees.csv",
                id_field: Some("MLR_No"),
            },
        ),
        // Add more verified resources here. Each must be probed live before
        // being registered — the v2 filter API rejects unregistered URLs with
        // {"code":"422","message":"Not a valid resource"}.
    ]
}

fn specs() -> &'static [DatasetSpec] {
    &[DatasetSpec {
        id: "money-lenders-licensees",
        title: "Money Lenders Licensees (Companies Registry)",
        description: Some("List of licensed money lenders, published by the Companies Registry."),
        refresh_interval_secs: 24 * 3600,
    }]
}

pub struct DataGovHkConnector {
    filter_url: String,
    archive_url: String,
    client: reqwest::Client,
}

impl DataGovHkConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(20_000))
            .gzip(true)
            .pool_max_idle_per_host(32)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            filter_url: settings.data_gov_hk_filter_url.clone(),
            archive_url: settings.data_gov_hk_archive_url.clone(),
            client,
        })
    }

    fn resource_for(&self, dataset: &str) -> Option<&FilterResource> {
        resources()
            .iter()
            .find(|(id, _)| *id == dataset)
            .map(|(_, r)| r)
    }

    /// Build the `q` parameter value for the v2 filter API.
    fn build_query(&self, resource_url: &str) -> serde_json::Value {
        serde_json::json!({
            "resource": resource_url,
            "section": 1,
            "format": "json"
        })
    }

    async fn fetch_filter(&self, resource_url: &str) -> Result<Vec<serde_json::Value>> {
        let q = self.build_query(resource_url);
        // The v2 filter API expects q as a JSON string in the query.
        let resp = self
            .client
            .get(&self.filter_url)
            .query(&[("q", q.to_string())])
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "datagovhk",
                status: 0,
                detail: format!("transport: {e}"),
            })?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "datagovhk",
                status,
                detail,
            });
        }

        // The filter API returns either a bare JSON array or an error object
        // like {"code":"422","message":"..."}. Handle both.
        let text = resp.text().await.map_err(|e| Error::Decode {
            origin: "datagovhk",
            backtrace: serde::de::Error::custom(format!("body read: {e}")),
        })?;
        let trimmed = text.trim_start();
        if trimmed.starts_with('[') {
            serde_json::from_str::<Vec<serde_json::Value>>(&text).map_err(|e| Error::Decode {
                origin: "datagovhk",
                backtrace: e,
            })
        } else {
            // Try to surface the platform error message.
            let detail = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("message").cloned())
                .map(|m| m.to_string())
                .unwrap_or_else(|| text.chars().take(200).collect());
            Err(Error::Upstream {
                origin: "datagovhk",
                status,
                detail,
            })
        }
    }

    /// Historical archive file listing. Returns the raw JSON for now; useful for
    /// the agent layer to discover what changed day-to-day.
    #[allow(dead_code)]
    pub async fn list_archive(
        &self,
        provider: &str,
        start: &str,
        end: &str,
    ) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(&self.archive_url)
            .query(&[
                ("start", start),
                ("end", end),
                ("provider", provider),
                ("max", "100"),
            ])
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "datagovhk",
                status: 0,
                detail: format!("transport: {e}"),
            })?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "datagovhk",
                status,
                detail,
            });
        }
        resp.json().await.map_err(|e| Error::Decode {
            origin: "datagovhk",
            backtrace: serde::de::Error::custom(e.to_string()),
        })
    }
}

#[async_trait]
impl Connector for DataGovHkConnector {
    fn source(&self) -> DataSource {
        DataSource::DataGovHk
    }

    fn datasets(&self) -> &[DatasetSpec] {
        specs()
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let resource = self.resource_for(dataset).ok_or_else(|| {
            Error::Internal(format!(
                "datagovhk: no resource mapping for dataset {dataset}"
            ))
        })?;
        let rows = self.fetch_filter(resource.resource_url).await?;
        let now = Utc::now();
        let records: Vec<NormalizedRecord> = rows
            .into_iter()
            .map(|raw| {
                let fields = normalize_row(&raw);
                let record_id = record_id_for(dataset, resource.id_field, &fields);
                NormalizedRecord {
                    source: DataSource::DataGovHk,
                    dataset: dataset.to_string(),
                    record_id,
                    fields,
                    fetched_at: now,
                }
            })
            .collect();
        tracing::info!(dataset, count = records.len(), "datagovhk: fetched dataset");
        Ok(records)
    }
}

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
        other => RecordValue::Str(other.to_string()),
    }
}

fn record_id_for(
    _dataset: &str,
    id_field: Option<&str>,
    fields: &BTreeMap<String, RecordValue>,
) -> String {
    if let Some(field) = id_field {
        if let Some(v) = fields.get(field) {
            return match v {
                RecordValue::Str(s) => s.clone(),
                RecordValue::Int(i) => i.to_string(),
                other => format!("{other:?}"),
            };
        }
    }
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (k, v) in fields {
        k.hash(&mut h);
        format!("{v:?}").hash(&mut h);
    }
    format!("id-{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_object_row() {
        let raw: serde_json::Value = serde_json::json!({
            "MLR_No": 6384,
            "Name_Eng": "VCREDIT Finance Limited",
            "Expiry_Date": "27-Apr-27"
        });
        let fields = normalize_row(&raw);
        assert_eq!(fields.get("MLR_No"), Some(&RecordValue::Int(6384)));
        assert_eq!(
            fields.get("Name_Eng"),
            Some(&RecordValue::Str("VCREDIT Finance Limited".into()))
        );
    }

    #[test]
    fn record_id_uses_configured_field() {
        let mut fields = BTreeMap::new();
        fields.insert("MLR_No".into(), RecordValue::Int(6384));
        let id = record_id_for("money-lenders-licensees", Some("MLR_No"), &fields);
        assert_eq!(id, "6384");
    }

    #[test]
    fn record_id_falls_back_to_hash_when_no_id_field() {
        let mut fields = BTreeMap::new();
        fields.insert("foo".into(), RecordValue::Str("bar".into()));
        let id = record_id_for("x", None, &fields);
        assert!(id.starts_with("id-"));
    }
}
