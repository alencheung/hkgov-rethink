//! Press release connector.
//!
//! Sources government press releases — the raw material the agent layer will
//! cross-reference against the statistical data to surface divergences.
//!
//! v2 implements the **HKMA press releases API** (verified live):
//! `GET api.hkma.gov.hk/public/press-releases?lang=en&pagesize=N`
//! → `{header, result:{records:[{title, link, date}]}}`.
//!
//! ISD/info.gov.hk scraping and news.gov.hk RSS are stubbed for later — they
//! need HTML/RSS parsing (ROADMAP follow-up).

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

const DATASETS: &[DatasetSpec] = &[DatasetSpec {
    id: "hkma-press-releases",
    title: "HKMA Press Releases",
    description: Some(
        "Press releases issued by the Hong Kong Monetary Authority — the \
         narrative the agent layer cross-references against HKMA statistics.",
    ),
    refresh_interval_secs: 30 * 60,
}];

pub struct PressConnector {
    hkma_base_url: String,
    client: reqwest::Client,
}

impl PressConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(settings.hkma_timeout_ms))
            .gzip(true)
            .pool_max_idle_per_host(16)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            hkma_base_url: settings.hkma_base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    fn url_for(&self, dataset: &str) -> Option<String> {
        match dataset {
            "hkma-press-releases" => Some(format!("{}/press-releases", self.hkma_base_url)),
            _ => None,
        }
    }
}

/// HKMA press release record shape.
#[derive(Debug, Deserialize)]
struct PressRecord {
    #[serde(default)]
    title: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    date: String,
}

#[derive(Debug, Deserialize)]
struct PressResult {
    #[serde(default)]
    records: Vec<PressRecord>,
}

#[derive(Debug, Deserialize)]
struct PressEnvelope {
    header: PressHeader,
    result: PressResult,
}

#[derive(Debug, Deserialize)]
struct PressHeader {
    success: bool,
    #[serde(default)]
    err_code: String,
    #[serde(default)]
    err_msg: String,
}

#[async_trait]
impl Connector for PressConnector {
    fn source(&self) -> DataSource {
        DataSource::Press
    }

    fn datasets(&self) -> &[DatasetSpec] {
        DATASETS
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let base = self.url_for(dataset).ok_or_else(|| {
            Error::Internal(format!("press: no URL mapping for dataset {dataset}"))
        })?;
        let url = format!("{base}?lang=en&pagesize=200");

        let resp = self.client.get(&url).send().await.map_err(|e| {
            Error::Upstream {
                origin: "press",
                status: 0,
                detail: format!("transport: {e}"),
            }
        })?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "press",
                status,
                detail,
            });
        }
        let json: serde_json::Value = resp.json().await.map_err(|e| Error::Decode {
            origin: "press",
            backtrace: serde::de::Error::custom(e.to_string()),
        })?;
        let env: PressEnvelope =
            serde_json::from_value(json).map_err(|e| Error::Decode {
                origin: "press",
                backtrace: e,
            })?;
        if !env.header.success {
            return Err(Error::Upstream {
                origin: "press",
                status: 200,
                detail: format!("{}: {}", env.header.err_code, env.header.err_msg),
            });
        }

        let now = Utc::now();
        let records: Vec<NormalizedRecord> = env
            .result
            .records
            .into_iter()
            .map(|r| {
                let mut fields = BTreeMap::new();
                fields.insert("title".into(), RecordValue::Str(r.title));
                fields.insert("link".into(), RecordValue::Str(r.link));
                fields.insert("date".into(), RecordValue::Str(r.date.clone()));
                let record_id = r.date;
                NormalizedRecord {
                    source: DataSource::Press,
                    dataset: dataset.to_string(),
                    record_id,
                    fields,
                    fetched_at: now,
                }
            })
            .collect();
        tracing::info!(dataset, count = records.len(), "press: fetched dataset");
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_press_envelope() {
        let raw = r#"{
            "header": {"success": true, "err_code": "0000", "err_msg": "No error found"},
            "result": {"records": [
                {"title": "Test release", "link": "https://example.com/x", "date": "2026-06-18"}
            ]}
        }"#;
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        let env: PressEnvelope = serde_json::from_value(v).unwrap();
        assert!(env.header.success);
        assert_eq!(env.result.records.len(), 1);
        assert_eq!(env.result.records[0].date, "2026-06-18");
    }
}
