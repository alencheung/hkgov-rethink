//! LandsD / CSDI geospatial connector.
//!
//! The government-only `api.portal.hkmapservice.gov.hk` is excluded (see
//! docs/DATA_SOURCES.md). Instead, this connector surfaces the **catalog of
//! available open LandsD datasets** via the data.gov.hk historical archive —
//! real, verified data (523 LandsD files were listed live at build time).
//!
//! This gives the agent layer a registry of geospatial datasets to draw on
//! without needing credentials for the restricted map tile API.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

const DATASETS: &[DatasetSpec] = &[DatasetSpec {
    id: "landsd-catalog",
    title: "LandsD Open Dataset Catalog",
    description: Some(
        "Catalog of open LandsD/CSDI geospatial datasets published via the \
         data.gov.hk historical archive. Each record is one available file.",
    ),
    category: Category::Property,
    tags: &["geospatial", "landsd", "csdi", "catalog"],
    cadence: Cadence::Daily,
    refresh_interval_secs: 24 * 3600,
}];

pub struct LandsDConnector {
    archive_url: String,
    client: reqwest::Client,
}

impl LandsDConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(20_000))
            .gzip(true)
            .pool_max_idle_per_host(16)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            archive_url: settings.data_gov_hk_archive_url.clone(),
            client,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ArchiveResponse {
    #[serde(rename = "file-count", default)]
    file_count: u64,
    #[serde(default)]
    files: Vec<ArchiveFile>,
}

#[derive(Debug, Deserialize)]
struct ArchiveFile {
    #[serde(rename = "dataset-id", default)]
    dataset_id: String,
    #[serde(rename = "dataset-name-en", default)]
    dataset_name_en: String,
    #[serde(default, rename = "format")]
    format: String,
    #[serde(rename = "provider-id", default)]
    provider_id: String,
    #[serde(rename = "category-id", default)]
    category_id: String,
}

#[async_trait]
impl Connector for LandsDConnector {
    fn source(&self) -> DataSource {
        DataSource::LandsD
    }

    fn datasets(&self) -> &[DatasetSpec] {
        DATASETS
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        if dataset != "landsd-catalog" {
            return Err(Error::Internal(format!(
                "landsd: unknown dataset {dataset}"
            )));
        }
        // 30-day window ending *yesterday* — the archive rejects `end` later
        // than yesterday. Keeps the catalog fresh and bounded.
        let yesterday = chrono::Local::now()
            .checked_sub_days(chrono::Days::new(1))
            .unwrap_or_else(chrono::Local::now);
        let end = yesterday.format("%Y%m%d").to_string();
        let start = yesterday
            .checked_sub_days(chrono::Days::new(30))
            .map(|d| d.format("%Y%m%d").to_string())
            .unwrap_or_else(|| end.clone());

        let url = format!(
            "{}?start={}&end={}&provider=hk-landsd&max=1000",
            self.archive_url, start, end
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "landsd",
                status: 0,
                detail: format!("transport: {e}"),
            })?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "landsd",
                status,
                detail,
            });
        }
        let arch: ArchiveResponse = resp.json().await.map_err(|e| Error::Decode {
            origin: "landsd",
            backtrace: serde::de::Error::custom(e.to_string()),
        })?;

        let now = Utc::now();
        let records: Vec<NormalizedRecord> = arch
            .files
            .into_iter()
            .map(|f| {
                let record_id = f.dataset_id.clone();
                let mut fields = BTreeMap::new();
                fields.insert("dataset_id".into(), RecordValue::Str(f.dataset_id));
                fields.insert("name_en".into(), RecordValue::Str(f.dataset_name_en));
                fields.insert("format".into(), RecordValue::Str(f.format));
                fields.insert("provider_id".into(), RecordValue::Str(f.provider_id));
                fields.insert("category_id".into(), RecordValue::Str(f.category_id));
                NormalizedRecord {
                    source: DataSource::LandsD,
                    dataset: dataset.to_string(),
                    record_id,
                    fields,
                    fetched_at: now,
                }
            })
            .collect();
        tracing::info!(
            dataset,
            file_count = arch.file_count,
            returned = records.len(),
            "landsd: fetched catalog"
        );
        Ok(records)
    }
}
