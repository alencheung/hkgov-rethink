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
//!
//! ## Dataset coverage
//!
//! The `DATASETS` table below enumerates the **entire public HKMA Open API
//! catalog** — every dataset listed under `apidocs.hkma.gov.hk/documentation`,
//! each one probe-verified live (HTTP 200 + `header.success`). That is 151
//! datasets across 14 sections:
//!
//! - Monthly Statistical Bulletin (financial, money, banking, money-markets,
//!   efbn, er-ir, monetary-operation, ef-fc-resv-assets, gov-bond)
//! - Daily Monetary Statistics
//! - Other (Exchange Fund)
//! - Bank & SVF Related Information
//! - Financial Market Infrastructure (Debt Securities Settlement System,
//!   Trade Repository)
//!
//! A handful of datasets require an extra query parameter to return data:
//! - `lang` (`=en`) — the `bank-svf-info` family rejects requests without it.
//! - `segment` — tender results, bond pricings, SVF licensees, HKTR
//!   disclosures, etc. need a segment selector (tenor / instrument / type).
//!   Each such row carries its verified default segment; the connector sends
//!   exactly one segment so a single fetch is deterministic.
//!
//! Note on DSSI: the Debt Securities Settlement System datasets live at
//! `/public/debt-securities-settlement-system/...` — NOT under
//! `financial-market-infra/` despite the docs URL. Verified live.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

/// The verified HKMA dataset table lives in its own module so the connector
/// logic stays navigable. See `crate::hkma_datasets` for the table + row struct.
use crate::hkma_datasets::{HkmaDataset, DATASETS};

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
            .redirect(reqwest::redirect::Policy::none())
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

    /// Look up a dataset row by slug. O(n) but n is small (151) and the call is
    /// cold (once per refresh interval per dataset).
    fn dataset(&self, slug: &str) -> Option<&'static HkmaDataset> {
        DATASETS.iter().find(|d| d.slug == slug)
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
                let detail = crate::limited::read_text_limited(
                    resp,
                    "hkma",
                    crate::limited::MAX_ERROR_BYTES,
                )
                .await
                .unwrap_or_default();
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

            // Cap the body before parsing — HKMA's monetary-statistics tables
            // are large but bounded; a runaway response would otherwise OOM
            // the process (PERF-CON-01).
            let body =
                crate::limited::read_text_limited(resp, "hkma", crate::limited::MAX_DATA_BYTES)
                    .await?;
            let json: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| Error::Decode {
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
        // The table is the source of truth; project each row onto a DatasetSpec
        // lazily on first call. The projected slice lives for the process lifetime.
        ensure_specs_initialized();
        HKMA_SPECS
            .get()
            .map(Vec::as_slice)
            .expect("HKMA specs initialized")
    }

    // M1 lineage: HKMA knows its exact upstream URL + envelope shape. The
    // gateway's lineage index therefore carries a verifiable provenance pointer
    // for every HKMA dataset (the offset=0 URL; pagination is connector-
    // internal and not part of the lineage identity).
    fn upstream_url(&self, dataset: &str) -> Option<String> {
        self.dataset(dataset).map(|ds| ds.url(&self.base_url, 0))
    }

    fn upstream_format(&self, _dataset: &str) -> hkgov_store::UpstreamFormat {
        hkgov_store::UpstreamFormat::HkmaJson
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let ds = self.dataset(dataset).ok_or_else(|| {
            Error::Internal(format!("hkma: no path mapping for dataset {dataset}"))
        })?;

        let now = Utc::now();
        let mut raw_records: Vec<serde_json::Value> = Vec::new();
        let mut datasize: u64 = 0;
        let mut offset: u64 = 0;
        // Safety cap: the HKMA API pages at 1000 records. 50 pages caps a
        // single dataset fetch at 50000 records, which is well above every
        // dataset in the verified catalog (the largest is a few thousand).
        // This guards against an unbounded loop if the upstream lies about
        // `datasize` or returns more records than it reports.
        const MAX_PAGES: u32 = 50;
        for page in 0..MAX_PAGES {
            let url = ds.url(&self.base_url, offset);
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

            // On the first page, capture the declared total record count so we
            // know how far to page.
            if page == 0 {
                datasize = env.result.datasize;
            }
            let returned = env.result.records.len() as u64;
            raw_records.extend(env.result.records);

            // Stop when we've collected everything the upstream reported, or
            // when a page returns fewer than a full page (last page / empty).
            if returned == 0 || raw_records.len() as u64 >= datasize {
                break;
            }
            offset += returned;
        }

        if (raw_records.len() as u64) < datasize {
            // We hit MAX_PAGES before collecting everything. Surface it loudly
            // so the truncation is never silent.
            tracing::warn!(
                dataset,
                expected = datasize,
                fetched = raw_records.len(),
                skipped = datasize.saturating_sub(raw_records.len() as u64),
                "hkma: dataset truncated at safety cap ({} pages); increase MAX_PAGES if this dataset legitimately exceeds it",
                MAX_PAGES,
            );
        }

        let records: Vec<NormalizedRecord> = raw_records
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
            count = datasize,
            fetched = records.len(),
            "hkma: fetched dataset"
        );
        Ok(records)
    }
}

/// Lazy-built `DatasetSpec` slice projected from [`DATASETS`]. Held in a
/// `OnceLock` so the projection happens exactly once per process and the
/// connector can hand out a `&'static`-lifetime view.
static HKMA_SPECS: std::sync::OnceLock<Vec<DatasetSpec>> = std::sync::OnceLock::new();

/// Initialize the projected specs once. Called from the registry build so the
/// `&'static` lifetime in `datasets()` is sound.
pub(crate) fn ensure_specs_initialized() {
    HKMA_SPECS.get_or_init(|| {
        DATASETS
            .iter()
            .map(|d| {
                // Description is derived from the title prefix — keeps the
                // catalog self-describing without hand-authoring 151 strings.
                let desc = format!("HKMA Open API: {}", d.title);
                DatasetSpec {
                    id: d.slug,
                    title: d.title,
                    description: Some(Box::leak(desc.into_boxed_str())),
                    category: d.category,
                    tags: d.tags,
                    cadence: d.cadence,
                    refresh_interval_secs: d.refresh_interval_secs,
                }
            })
            .collect()
    });
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
    // D-012: when the HKMA catalog was widened from 5 datasets to the full 151,
    // every new dataset fell through to the hash fallback below, producing
    // opaque `id-<hash>` record ids. That broke two things: (a) evidence
    // pointers in insights became unreadable, and (b) `cross_source_gap`
    // joins press release dates against data record_ids, so hash ids meant
    // every press date looked "unexplained". A natural date key per record
    // fixes both. The dataset-specific map covers the legacy slugs and the
    // exact period keys the detectors were authored against; the generic
    // fallback then picks up the other ~150 datasets from any date-like field
    // they carry, before hashing.
    let candidates: &[&str] = match dataset {
        "capital-market-statistics" | "residential-mortgage-survey" => &["end_of_month"],
        "daily-interbank-liquidity" | "daily-figures-interbank-liquidity" => {
            &["date", "end_of_date"]
        }
        _ => &[],
    };
    let mut date_base: Option<String> = None;
    for key in candidates {
        if let Some(RecordValue::Str(s)) = fields.get(*key) {
            date_base = Some(s.clone());
            break;
        }
        if let Some(RecordValue::Int(i)) = fields.get(*key) {
            date_base = Some(i.to_string());
            break;
        }
    }
    // Generic fallback: scan for any of the common HKMA date/period field
    // names, in priority order. Almost every HKMA dataset exposes its period
    // as one of these columns. This keeps record ids human-readable and
    // (where the field is a true calendar date) joinable by cross_source_gap.
    if date_base.is_none() {
        const GENERIC_DATE_FIELDS: &[&str] = &[
            "end_of_date",
            "end_of_month",
            "end_of_quarter",
            "end_of_year",
            "date",
            "year_month",
            "quarter",
            "year",
        ];
        for key in GENERIC_DATE_FIELDS {
            if let Some(RecordValue::Str(s)) = fields.get(*key) {
                date_base = Some(s.clone());
                break;
            }
            if let Some(RecordValue::Int(i)) = fields.get(*key) {
                date_base = Some(i.to_string());
                break;
            }
        }
    }

    if let Some(date) = date_base {
        // D-013: multi-dimensional datasets (keyed by period + a dimension such
        // as currency, sector, country, …) previously all collapsed onto the
        // bare date as their record_id. Two rows sharing a date but differing
        // on the dimension got the SAME id, which silently overwrote each
        // other in PgStore (PK collision) and muddied dedup in MemoryStore.
        // Append the first non-empty dimension value found so each row is
        // uniquely identifiable.
        const DIMENSION_FIELDS: &[&str] = &[
            "currency",
            "sector",
            "type",
            "category",
            "country",
            "instrument",
            "tenor",
            "rating",
            "issuer_type",
            "component",
            "sub_type",
            "breakdown",
        ];
        for key in DIMENSION_FIELDS {
            if let Some(RecordValue::Str(s)) = fields.get(*key) {
                if !s.is_empty() {
                    return format!("{date}|{s}");
                }
            }
        }
        return date;
    }

    // Deterministic fallback when no date/period field is present. Delegates to
    // the shared version-stable FNV-1a derivation so this and every other
    // connector share one implementation (previously HKMA carried its own copy).
    crate::ids::synthetic_record_id(fields)
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

    // ---- D-012: generic date-key fallback so widened datasets get natural ids ----
    //
    // Before this fix, every dataset not in the two-row explicit map fell through
    // to the hash fallback, producing opaque `id-<hash>` ids. That broke
    // cross_source_gap (which joins press dates against data record_ids) and made
    // evidence pointers unreadable. The generic fallback now picks up any common
    // HKMA date/period field, so the ~150 datasets added in the catalog widening
    // keep human-readable, joinable ids.

    #[test]
    fn record_id_uses_end_of_date_for_daily_interbank() {
        let mut fields = BTreeMap::new();
        fields.insert("end_of_date".into(), RecordValue::Str("2026-06-18".into()));
        fields.insert("hibor_overnight".into(), RecordValue::Float(2.93));
        let id = record_id_for("daily-figures-interbank-liquidity", &fields);
        assert_eq!(
            id, "2026-06-18",
            "daily interbank id must be its date, not a hash"
        );
    }

    #[test]
    fn record_id_generic_fallback_picks_up_unknown_dataset_date_field() {
        // A dataset NOT in the explicit map should still get a natural id from
        // any date-like field it carries, rather than a hash.
        let mut fields = BTreeMap::new();
        fields.insert("end_of_quarter".into(), RecordValue::Str("2026-Q2".into()));
        fields.insert("some_metric".into(), RecordValue::Float(1.0));
        let id = record_id_for("some-new-bulletin-dataset", &fields);
        assert_eq!(
            id, "2026-Q2",
            "unknown dataset with a period field should use it"
        );
    }

    #[test]
    fn record_id_hash_fallback_only_when_no_date_field() {
        // When no date/period field is present, the deterministic hash still applies.
        let mut fields = BTreeMap::new();
        fields.insert("bank_name".into(), RecordValue::Str("ACME".into()));
        fields.insert("branch_count".into(), RecordValue::Int(42));
        let id = record_id_for("banks-branch-locator", &fields);
        assert!(id.starts_with("id-"), "no date field -> hash id, got: {id}");
        assert!(id.len() > "id-".len(), "hash id must carry a digest");
    }

    #[test]
    fn hibor_tag_present_on_interbank_datasets() {
        // D-012: the dashboard + flagship narrative rely on `?tag=hibor`
        // resolving. The tag was dropped in the catalog widening and restored.
        let hibor_tagged: Vec<_> = DATASETS
            .iter()
            .filter(|d| d.tags.contains(&"hibor"))
            .collect();
        assert!(
            !hibor_tagged.is_empty(),
            "at least one dataset must carry the hibor tag"
        );
        assert!(
            hibor_tagged
                .iter()
                .any(|d| d.slug == "daily-figures-interbank-liquidity"),
            "the daily interbank feed must be hibor-tagged"
        );
    }

    #[test]
    fn dataset_table_is_well_formed() {
        // Regression guard: the catalog must stay exhaustive and unique.
        assert_eq!(
            DATASETS.len(),
            151,
            "HKMA dataset count drifted from the verified 151"
        );
        // Every slug unique.
        let mut seen = std::collections::HashSet::new();
        for d in DATASETS {
            assert!(seen.insert(d.slug), "duplicate HKMA slug: {}", d.slug);
        }
        // Every path non-empty + starts with a known section.
        for d in DATASETS {
            assert!(!d.path.is_empty(), "empty path for {}", d.slug);
            assert!(
                d.path.starts_with("market-data-and-statistics/")
                    || d.path.starts_with("bank-svf-info/")
                    || d.path.starts_with("debt-securities-settlement-system/")
                    || d.path.starts_with("financial-market-infra/"),
                "unexpected path prefix for {}: {}",
                d.slug,
                d.path
            );
        }
    }

    #[test]
    fn url_builder_adds_segment_and_lang() {
        let plain = DATASETS
            .iter()
            .find(|d| d.slug == "capital-market-statistics")
            .unwrap();
        assert_eq!(
            plain.url("https://api.hkma.gov.hk/public", 0),
            "https://api.hkma.gov.hk/public/market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics?pagesize=1000&offset=0"
        );

        let seg = DATASETS
            .iter()
            .find(|d| d.slug == "efbn-tender-results-efb")
            .unwrap();
        assert_eq!(
            seg.url("https://api.hkma.gov.hk/public", 0),
            "https://api.hkma.gov.hk/public/market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-tender-results-efb?pagesize=1000&offset=0&segment=28day"
        );

        let lang_seg = DATASETS
            .iter()
            .find(|d| d.slug == "register-svf-licensees")
            .unwrap();
        assert_eq!(
            lang_seg.url("https://api.hkma.gov.hk/public", 0),
            "https://api.hkma.gov.hk/public/bank-svf-info/register-svf-licensees?pagesize=1000&offset=0&segment=SVFLic&lang=en"
        );

        // Offset advances the record window for follow-up pages.
        assert_eq!(
            plain.url("https://api.hkma.gov.hk/public", 2000),
            "https://api.hkma.gov.hk/public/market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics?pagesize=1000&offset=2000"
        );
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
