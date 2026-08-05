//! Land Registry (土地註冊處) connector — property transactions.
//!
//! Source: the JSON statistics files published at
//! `https://www.landreg.gov.hk/datagovhk/` (verified live):
//! - `consideration_YYYY.json` — number of S&P agreements by price band, one
//!   column per month (Jan..Dec). This is the headline 樓市成交 series.
//! - `YYYYMM_data.json` — all-instruments monthly statistics, including the
//!   Primary Sales vs Secondary Sales split for residential units. This covers
//!   二手房 (secondary-market transactions).
//!
//! Both are **plain JSON files** (no queryable API). We fetch the current
//! year's file(s) and parse them client-side.
//!
//! Quirks handled here (verified against the live files):
//! - Counts are strings with thousands commas (`"1,186"`) → parsed to ints.
//! - The consideration file is wide (one row per price band, columns = months);
//!   we transpose it into one record per month with a `total_units` field (the
//!   sum across price bands) so `series_jump` has a clean monthly series.
//! - The all-instruments file uses a `Description` string to name each row
//!   (e.g. `"Number of Secondary Sales for ASP Residential Building Units"`);
//!   we filter for the Primary/Secondary Sales rows.

use crate::{strip_bom, Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use std::collections::BTreeMap;
use std::time::Duration;

const DATASETS: &[DatasetSpec] = &[
    DatasetSpec {
        id: "monthly-transactions",
        title: "Monthly Property Transactions by Price Band",
        description: Some(
            "Land Registry monthly number of sale & purchase agreements for \
             building units, broken down by consideration (price) band. \
             Aggregated to a territory-wide monthly total — the headline \
             property-transaction volume series.",
        ),
        category: Category::Property,
        tags: &["land-registry", "property", "transactions", "consideration"],
        cadence: Cadence::Monthly,
        refresh_interval_secs: 60 * 60,
    },
    DatasetSpec {
        id: "monthly-primary-secondary",
        title: "Monthly Primary vs Secondary Market Sales",
        description: Some(
            "Land Registry monthly breakdown of residential sales into the \
             primary (new-build) and secondary (existing-home) markets. The \
             secondary-market series covers 二手房 transactions.",
        ),
        category: Category::Property,
        tags: &[
            "land-registry",
            "property",
            "secondary-market",
            "primary-market",
        ],
        cadence: Cadence::Monthly,
        refresh_interval_secs: 60 * 60,
    },
];

const BASE_URL: &str = "https://www.landreg.gov.hk/datagovhk";

pub struct LandRegistryConnector {
    client: reqwest::Client,
}

impl LandRegistryConnector {
    pub fn new(_settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(30_000))
            .gzip(true)
            .redirect(reqwest::redirect::Policy::none())
            .pool_max_idle_per_host(16)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self { client })
    }

    /// Fetch the current year's consideration file as parsed JSON rows.
    async fn fetch_consideration(&self) -> Result<Vec<serde_json::Value>> {
        let year = Utc::now().format("%Y").to_string();
        let url = format!("{BASE_URL}/consideration_{year}.json");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "landregistry",
                status: 0,
                detail: format!("transport: {e}"),
            })?
            .error_for_status()
            .map_err(|e| Error::Upstream {
                origin: "landregistry",
                status: e.status().map(|s| s.as_u16()).unwrap_or(0),
                detail: format!("http: {e}"),
            })?;
        // Cap the body before parsing — without a bound a runaway or
        // malformed JSON file could OOM the process (PERF-CON-01).
        let body =
            crate::limited::read_text_limited(resp, "landregistry", crate::limited::MAX_DATA_BYTES)
                .await?;
        // The Land Registry publishes these JSON files with a UTF-8 BOM
        // prefix. serde_json's `.json()` does not strip it, so the decode
        // fails on byte 0xEF before parsing even starts. Read as text and
        // strip the BOM, then parse.
        let body = strip_bom(&body);
        let parsed: serde_json::Value = serde_json::from_str(body).map_err(|e| Error::Decode {
            origin: "landregistry",
            backtrace: serde::de::Error::custom(format!("consideration decode: {e}")),
        })?;
        // The file is a JSON array of row objects.
        Ok(parsed.as_array().cloned().unwrap_or_default())
    }

    /// Fetch the all-instruments file for the current month (or prior month if
    /// the current month's file isn't published yet).
    async fn fetch_all_instruments(&self) -> Result<Vec<serde_json::Value>> {
        let now = Utc::now();
        // Try the current month, then fall back to the prior month. The
        // prior-month fallback exists because the current-month file is
        // published mid-month, so early in a month it may not yet exist (404).
        let mut last_status: u16 = 0;
        for offset in [0, 1] {
            let d = now
                .checked_sub_signed(chrono::Duration::days(offset * 31))
                .unwrap_or(now);
            let ym = d.format("%Y%m").to_string();
            let url = format!("{BASE_URL}/{ym}_data.json");
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| Error::Upstream {
                    origin: "landregistry",
                    status: 0,
                    detail: format!("transport: {e}"),
                })?;
            if !resp.status().is_success() {
                // Record the status so we can build a meaningful error if BOTH
                // months fail. A 404 on the current month is expected early in
                // the month (file not published yet) → fall through to the
                // prior-month attempt. But if the prior month ALSO fails, we
                // must NOT return Ok(empty) — that would make the ingest
                // supervisor call put_dataset([]), atomically REPLACING the
                // live dataset with nothing (silent data wipe). Return Err so
                // the supervisor skips the put and preserves prior data
                // (D-037 / C-2).
                last_status = resp.status().as_u16();
                continue;
            }
            // Cap the instruments body before parsing (PERF-CON-01).
            let body = crate::limited::read_text_limited(
                resp,
                "landregistry",
                crate::limited::MAX_DATA_BYTES,
            )
            .await?;
            // Same BOM issue as fetch_consideration — see that method.
            let body = strip_bom(&body);
            let parsed: serde_json::Value =
                serde_json::from_str(body).map_err(|e| Error::Decode {
                    origin: "landregistry",
                    backtrace: serde::de::Error::custom(format!("instruments decode: {e}")),
                })?;
            return Ok(parsed.as_array().cloned().unwrap_or_default());
        }
        // Both months failed (e.g. upstream 5xx, or the file moved). Returning
        // an error here — rather than Ok(empty) — ensures the ingest supervisor
        // logs "fetch failed" and leaves the cached dataset intact, instead of
        // silently replacing it with zero records.
        Err(Error::Upstream {
            origin: "landregistry",
            status: last_status,
            detail: format!(
                "all-instruments file unavailable for both current and prior month (last status {last_status})"
            ),
        })
    }
}

#[async_trait]
impl Connector for LandRegistryConnector {
    fn source(&self) -> DataSource {
        DataSource::LandRegistry
    }

    fn datasets(&self) -> &[DatasetSpec] {
        DATASETS
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let now = Utc::now();
        let year = now.format("%Y").to_string();
        match dataset {
            "monthly-transactions" => {
                let rows = self.fetch_consideration().await?;
                Ok(aggregate_consideration(&rows, &year, now))
            }
            "monthly-primary-secondary" => {
                let rows = self.fetch_all_instruments().await?;
                Ok(extract_primary_secondary(&rows, now))
            }
            other => Err(Error::Internal(format!(
                "landregistry: no dataset mapping for {other}"
            ))),
        }
    }
}

/// Parse the consideration file (wide: price-band rows, month columns) into
/// one record per month with `total_units` = sum across all price bands.
fn aggregate_consideration(
    rows: &[serde_json::Value],
    year: &str,
    now: chrono::DateTime<Utc>,
) -> Vec<NormalizedRecord> {
    use std::collections::HashMap;
    // Month-name → total count accumulator.
    let mut by_month: HashMap<String, i64> = HashMap::new();
    for row in rows {
        let Some(obj) = row.as_object() else {
            continue;
        };
        for (key, val) in obj {
            // Skip the non-month "Range of Consideration" label column.
            let month_num = month_name_to_num(key);
            let Some(m) = month_num else {
                continue;
            };
            let count = parse_count(val.as_str());
            *by_month.entry(m).or_insert(0) += count;
        }
    }
    let mut out: Vec<NormalizedRecord> = by_month
        .into_iter()
        .map(|(month, total)| {
            let mut fields = BTreeMap::new();
            fields.insert("total_units".into(), RecordValue::Int(total));
            NormalizedRecord {
                source: DataSource::LandRegistry,
                dataset: "monthly-transactions".into(),
                record_id: format!("{year}-{month}"),
                fields,
                fetched_at: now,
            }
        })
        .collect();
    out.sort_by(|a, b| a.record_id.cmp(&b.record_id));
    out
}

/// Extract the Primary Sales and Secondary Sales rows from the all-instruments
/// file, producing one record per month with `primary_sales` and
/// `secondary_sales` fields.
fn extract_primary_secondary(
    rows: &[serde_json::Value],
    now: chrono::DateTime<Utc>,
) -> Vec<NormalizedRecord> {
    let mut primary: Option<i64> = None;
    let mut secondary: Option<i64> = None;
    let mut year_val: Option<i64> = None;
    let mut month_val: Option<i64> = None;
    for row in rows {
        let Some(obj) = row.as_object() else {
            continue;
        };
        // Year/Month are top-level numeric fields on the row (not parsed from
        // Description). Read them from any row that carries them.
        if let Some(y) = obj.get("Year").and_then(|v| v.as_i64()) {
            year_val = Some(y);
        }
        if let Some(m) = obj.get("Month").and_then(|v| v.as_i64()) {
            month_val = Some(m);
        }
        let desc = obj
            .get("Description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let units = obj.get("Units").and_then(|v| v.as_str());
        if desc.contains("Primary Sales") {
            primary = Some(parse_count_opt(units));
        } else if desc.contains("Secondary Sales") {
            secondary = Some(parse_count_opt(units));
        }
    }
    let Some(y) = year_val else {
        return Vec::new();
    };
    let m = month_val.unwrap_or(1);
    let mut fields = BTreeMap::new();
    if let Some(p) = primary {
        fields.insert("primary_sales".into(), RecordValue::Int(p));
    }
    if let Some(s) = secondary {
        fields.insert("secondary_sales".into(), RecordValue::Int(s));
    }
    vec![NormalizedRecord {
        source: DataSource::LandRegistry,
        dataset: "monthly-primary-secondary".into(),
        record_id: format!("{y:04}-{m:02}"),
        fields,
        fetched_at: now,
    }]
}

/// Map a month-name column header ("Jan".."Dec") to a zero-padded month number.
fn month_name_to_num(name: &str) -> Option<String> {
    let n = match name.trim() {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    Some(format!("{n:02}"))
}

/// Parse a count string with optional thousands commas ("1,186" → 1186).
fn parse_count(s: Option<&str>) -> i64 {
    s.and_then(|v| v.trim().replace(',', "").parse::<i64>().ok())
        .unwrap_or(0)
}

fn parse_count_opt(s: Option<&str>) -> i64 {
    parse_count(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_count_with_commas() {
        assert_eq!(parse_count(Some("1,186")), 1186);
        assert_eq!(parse_count(Some("42")), 42);
        assert_eq!(parse_count(Some("")), 0);
        assert_eq!(parse_count(None), 0);
    }

    // Regression: the Land Registry publishes its JSON files with a UTF-8 BOM
    // prefix. serde_json rejects BOM-prefixed input, so `.json()` silently
    // failed every fetch — Land Registry showed 0 records and its absence kept
    // the agent's scan-readiness wait perpetually timing out at 180s. The
    // fetchers now read as text and strip a leading BOM before parsing.
    #[test]
    fn bom_prefixed_json_parses_after_strip() {
        // "\u{feff}" is the BOM; serde_json alone chokes on it.
        let with_bom = "\u{feff}[{\"x\":1}]";
        let stripped = with_bom.strip_prefix('\u{feff}').unwrap_or(with_bom);
        let parsed: serde_json::Value =
            serde_json::from_str(stripped).expect("must parse after BOM strip");
        assert_eq!(parsed[0]["x"], 1, "content after BOM is intact");
        // And confirm the un-stripped form fails — this is the bug we fixed.
        assert!(
            serde_json::from_str::<serde_json::Value>(with_bom).is_err(),
            "serde_json must reject BOM-prefixed input (guards the regression)"
        );
    }

    #[test]
    fn month_name_maps_to_number() {
        assert_eq!(month_name_to_num("Jan").as_deref(), Some("01"));
        assert_eq!(month_name_to_num("Dec").as_deref(), Some("12"));
        assert!(month_name_to_num("Range of Consideration").is_none());
    }

    #[test]
    fn aggregates_consideration_into_monthly_totals() {
        // Two price-band rows, each with Jan + Feb counts.
        let rows = serde_json::json!([
            {"Range of Consideration ($ million)": "Less than 2", "Jan": "100", "Feb": "200"},
            {"Range of Consideration ($ million)": "2 to less than 3", "Jan": "1,000", "Feb": "1,500"}
        ]);
        let now = Utc::now();
        let recs = aggregate_consideration(rows.as_array().unwrap(), "2026", now);
        assert_eq!(recs.len(), 2, "one record per month");
        // Sorted by record_id: Feb < Jan lexicographically? No: "2026-01" < "2026-02".
        let jan = recs.iter().find(|r| r.record_id == "2026-01").unwrap();
        assert_eq!(
            jan.fields.get("total_units"),
            Some(&RecordValue::Int(1100)),
            "Jan total = 100 + 1000"
        );
        let feb = recs.iter().find(|r| r.record_id == "2026-02").unwrap();
        assert_eq!(
            feb.fields.get("total_units"),
            Some(&RecordValue::Int(1700)),
            "Feb total = 200 + 1500"
        );
    }

    #[test]
    fn extracts_primary_secondary_from_instruments() {
        let rows = serde_json::json!([
            {"Year": 2026, "Month": 5, "Description": "Year", "Units": "", "Consideration (nearest $ million)": ""},
            {"Description": "Number of Primary Sales for ASP Residential Building Units", "Units": "1,200"},
            {"Description": "Number of Secondary Sales for ASP Residential Building Units", "Units": "3,400"}
        ]);
        let now = Utc::now();
        let recs = extract_primary_secondary(rows.as_array().unwrap(), now);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].record_id, "2026-05");
        assert_eq!(
            recs[0].fields.get("primary_sales"),
            Some(&RecordValue::Int(1200))
        );
        assert_eq!(
            recs[0].fields.get("secondary_sales"),
            Some(&RecordValue::Int(3400))
        );
    }
}
