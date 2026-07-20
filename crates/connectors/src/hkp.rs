//! Hong Kong Property / 香港置業 (hkp.com.hk) connector — price index,
//! economic indicators, and 12-month Land Registry summary.
//!
//! Source: `www.hkp.com.hk` (verified live via the `hkgov-proxy` Worker —
//! CloudFront WAF geo-blocks non-HK IPs, returning HTTP 403 from our US egress).
//!
//! Three datasets, two upstream pages:
//!
//! 1. `hkp-price-index-monthly` ← market-insight page `__NEXT_DATA__` → `mrIndex[]`
//!    The 二手樓價指數 — HK/KLN/NT price indices, transaction counts, per-sqft
//!    prices/rents, weekly + monthly % changes. ~355 monthly points from 1997.
//!
//! 2. `hkp-economic-indicators-monthly` ← market-insight `economicIndicators[]`
//!    Mortgage rate, rental yield, real savings rate, HSI, USD index,
//!    unemployment, affordability ratio. ~354 monthly points from 1997.
//!
//! 3. `hkp-land-registry-summary-monthly` ← market-insight `langRegRecords[]`
//!    + 12-month page HTML tables. Latest-month breakdown by property class
//!    (firsthand_private, secondhand_private, firsthand_hos, industrial,
//!    commercial, shop) with `{number, amount, number_chg, amount_chg}`.
//!
//! Both pages are Next.js SSR — the full data is embedded in an
//! `<script id="__NEXT_DATA__" type="application/json">{...}</script>` island
//! in the HTML body. We extract that JSON with a regex and deserialize into
//! typed structs. No browser rendering needed.

use crate::{worker_fetch, Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

const PRICE_INDEX_ID: &str = "hkp-price-index-monthly";
const ECON_INDICATORS_ID: &str = "hkp-economic-indicators-monthly";
const LAND_REGISTRY_SUMMARY_ID: &str = "hkp-land-registry-summary-monthly";

static DATASETS: OnceLock<Vec<DatasetSpec>> = OnceLock::new();

fn datasets() -> &'static [DatasetSpec] {
    DATASETS.get_or_init(|| {
        vec![
            DatasetSpec {
                id: PRICE_INDEX_ID,
                title: "HKP 二手樓價指數 — Monthly Price Index".into(),
                description: Some(
                    "Hong Kong Property (香港置業) monthly secondary-market \
                     price index — the 二手樓價指數. Covers HK/KLN/NT regions, \
                     transaction counts (total / firsthand / secondhand), \
                     per-sqft price + rent (gross + net), and weekly + monthly \
                     % changes. ~355 monthly points from 1997. record_id = \
                     YYYY-MM (ISO month)."
                        .into(),
                ),
                category: Category::Property,
                tags: &["hkp", "price-index", "二手樓價指數", "香港置業"],
                cadence: Cadence::Monthly,
                refresh_interval_secs: 6 * 3600,
            },
            DatasetSpec {
                id: ECON_INDICATORS_ID,
                title: "HKP Economic Indicators — Monthly".into(),
                description: Some(
                    "Hong Kong Property (香港置業) monthly economic indicators \
                     that contextualize the property market: mortgage interest \
                     rate, rental yield, real savings rate, Hang Seng Index, \
                     USD index, unemployment rate, affordability ratio (price \
                     + rental). ~354 monthly points from 1997."
                        .into(),
                ),
                category: Category::Property,
                tags: &["hkp", "economic-indicators", "affordability", "mortgage-rate"],
                cadence: Cadence::Monthly,
                refresh_interval_secs: 6 * 3600,
            },
            DatasetSpec {
                id: LAND_REGISTRY_SUMMARY_ID,
                title: "HKP Land Registry Summary — Latest Month by Class".into(),
                description: Some(
                    "Hong Kong Property (香港置業) summary of Land Registry \
                     registrations, broken down by property class (firsthand \
                     private, secondhand private, firsthand HOS, industrial, \
                     commercial, shop). Each class carries number of \
                     registrations, total amount (HK$ 億), and % change vs \
                     previous month. record_id = YYYY-MM."
                        .into(),
                ),
                category: Category::Property,
                tags: &["hkp", "land-registry", "transaction-volume", "by-class"],
                cadence: Cadence::Monthly,
                refresh_interval_secs: 6 * 3600,
            },
        ]
    })
}

const MARKET_INSIGHT_URL: &str = "https://www.hkp.com.hk/zh-hk/market-insight";
const TWELVE_MONTH_URL: &str = "https://www.hkp.com.hk/land-registry-record/12months.html";

pub struct HkpConnector {
    client: reqwest::Client,
    /// When false (proxy not configured), `datasets()` returns empty and
    /// `fetch()` returns a clear error. Lets the connector exist in the
    /// registry without 502-ing every refresh when no proxy is set.
    enabled: bool,
    upstream: UpstreamSettings,
}

impl HkpConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(45_000))
            .gzip(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .pool_max_idle_per_host(4)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            client,
            enabled: settings.proxy_configured(),
            upstream: settings.clone(),
        })
    }

    async fn fetch_market_insight(&self) -> Result<MarketInsightData> {
        let html = worker_fetch(&self.client, &self.upstream, MARKET_INSIGHT_URL, &[]).await?;
        let json = extract_next_data(&html)?;
        let parsed: NextData = serde_json::from_str(&json).map_err(|e| Error::Decode {
            origin: "hkp",
            backtrace: serde::de::Error::custom(format!("__NEXT_DATA__ decode: {e}")),
        })?;
        Ok(parsed.props.page_props)
    }

    async fn fetch_12month_html(&self) -> Result<String> {
        worker_fetch(&self.client, &self.upstream, TWELVE_MONTH_URL, &[]).await
    }
}

#[async_trait]
impl Connector for HkpConnector {
    fn source(&self) -> DataSource {
        DataSource::Hkp
    }

    fn datasets(&self) -> &[DatasetSpec] {
        if self.enabled {
            datasets()
        } else {
            &[]
        }
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        if !self.enabled {
            return Err(Error::Internal(
                "hkp: Worker proxy not configured (set HKGOV_UPSTREAM__PROXY_URL + \
                 the CF Access service-token fields)"
                    .into(),
            ));
        }
        let now = Utc::now();
        match dataset {
            PRICE_INDEX_ID => {
                let mi = self.fetch_market_insight().await?;
                let recs: Vec<NormalizedRecord> = mi
                    .mr_index
                    .into_iter()
                    .filter_map(|p| p.into_record(now))
                    .collect();
                tracing::info!(dataset, points = recs.len(), "hkp: parsed price index");
                Ok(recs)
            }
            ECON_INDICATORS_ID => {
                let mi = self.fetch_market_insight().await?;
                let recs: Vec<NormalizedRecord> = mi
                    .economic_indicators
                    .into_iter()
                    .filter_map(|p| p.into_record(now))
                    .collect();
                tracing::info!(dataset, points = recs.len(), "hkp: parsed economic indicators");
                Ok(recs)
            }
            LAND_REGISTRY_SUMMARY_ID => {
                let mi = self.fetch_market_insight().await?;
                let mut recs: Vec<NormalizedRecord> = mi
                    .lang_reg_records
                    .into_iter()
                    .filter_map(|r| r.into_record(now))
                    .collect();
                // Augment with the 12-month history page if the latest-month
                // form is missing any data — the 12-mo page gives us monthly
                // totals going back 12 months, useful for trend detection.
                if let Ok(html12) = self.fetch_12month_html().await {
                    recs.extend(parse_12mo_totals(&html12, now));
                }
                tracing::info!(dataset, points = recs.len(), "hkp: parsed land registry summary");
                Ok(recs)
            }
            other => Err(Error::Internal(format!(
                "hkp: unknown dataset {other}"
            ))),
        }
    }
}

// ---- Next.js __NEXT_DATA__ extraction ----

/// Pull the JSON inside `<script id="__NEXT_DATA__" type="application/json">…</script>`.
/// Returns the raw JSON string (caller deserializes into typed structs).
fn extract_next_data(html: &str) -> Result<String> {
    let marker = r#"id="__NEXT_DATA__""#;
    let start_idx = html.find(marker).ok_or_else(|| {
        Error::Decode {
            origin: "hkp",
            backtrace: serde::de::Error::custom(
                "no __NEXT_DATA__ script tag in body — page shape changed?",
            ),
        }
    })?;
    let after_marker = &html[start_idx..];
    let json_start = after_marker.find('>').ok_or_else(|| Error::Decode {
        origin: "hkp",
        backtrace: serde::de::Error::custom("__NEXT_DATA__ tag has no '>'"),
    })?;
    let json_body_start = after_marker[json_start + 1..].as_ptr() as usize - html.as_ptr() as usize;
    let json_body = &html[json_body_start..];
    let json_end = json_body.find("</script>").ok_or_else(|| Error::Decode {
        origin: "hkp",
        backtrace: serde::de::Error::custom("__NEXT_DATA__ has no </script>"),
    })?;
    Ok(json_body[..json_end].trim().to_string())
}

// ---- Typed __NEXT_DATA__ shape (only the fields we read) ----

#[derive(Debug, Deserialize)]
struct NextData {
    props: NextProps,
}

#[derive(Debug, Deserialize)]
struct NextProps {
    #[serde(rename = "pageProps")]
    page_props: MarketInsightData,
}

#[derive(Debug, Default, Deserialize)]
struct MarketInsightData {
    #[serde(default, rename = "mrIndex")]
    mr_index: Vec<MrIndexPoint>,
    #[serde(default, rename = "economicIndicators")]
    economic_indicators: Vec<EconIndicatorPoint>,
    #[serde(default, rename = "langRegRecords")]
    lang_reg_records: Vec<LangRegRecord>,
}

/// One row of the 二手樓價指數 series. Most fields are optional because the
/// latest point carries weekly + monthly % fields that historical points
/// don't (and vice versa).
#[derive(Debug, Deserialize)]
struct MrIndexPoint {
    /// ISO timestamp, e.g. `"1997-01-01T00:00:00.000Z"`. We truncate to YYYY-MM.
    date: String,
    #[serde(default)]
    mr_index: Option<f64>,
    #[serde(default, rename = "mr_index_hk")]
    mr_index_hk: Option<f64>,
    #[serde(default, rename = "mr_index_kln")]
    mr_index_kln: Option<f64>,
    #[serde(default, rename = "mr_index_nt")]
    mr_index_nt: Option<f64>,
    #[serde(default, rename = "tx_count")]
    tx_count: Option<i64>,
    #[serde(default, rename = "tx_count_hk")]
    tx_count_hk: Option<i64>,
    #[serde(default, rename = "tx_count_kln")]
    tx_count_kln: Option<i64>,
    #[serde(default, rename = "tx_count_nt")]
    tx_count_nt: Option<i64>,
    #[serde(default, rename = "net_ft_price")]
    net_ft_price: Option<f64>,
    #[serde(default, rename = "net_ft_price_hk")]
    net_ft_price_hk: Option<f64>,
    #[serde(default, rename = "net_ft_price_kln")]
    net_ft_price_kln: Option<f64>,
    #[serde(default, rename = "net_ft_price_nt")]
    net_ft_price_nt: Option<f64>,
    #[serde(default, rename = "ft_price")]
    ft_price: Option<f64>,
    #[serde(default, rename = "ft_rent")]
    ft_rent: Option<f64>,
    #[serde(default, rename = "net_ft_rent")]
    net_ft_rent: Option<f64>,
    #[serde(default, rename = "monthly_perc")]
    monthly_perc: Option<f64>,
    #[serde(default, rename = "monthly_perc_hk")]
    monthly_perc_hk: Option<f64>,
    #[serde(default, rename = "monthly_perc_kln")]
    monthly_perc_kln: Option<f64>,
    #[serde(default, rename = "monthly_perc_nt")]
    monthly_perc_nt: Option<f64>,
}

impl MrIndexPoint {
    fn into_record(self, now: chrono::DateTime<Utc>) -> Option<NormalizedRecord> {
        let month = iso_ts_to_month(&self.date)?;
        let mut f = BTreeMap::new();
        if let Some(v) = self.mr_index {
            f.insert("mr_index".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.mr_index_hk {
            f.insert("mr_index_hk".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.mr_index_kln {
            f.insert("mr_index_kln".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.mr_index_nt {
            f.insert("mr_index_nt".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.tx_count {
            f.insert("tx_count".into(), RecordValue::Int(v));
        }
        if let Some(v) = self.tx_count_hk {
            f.insert("tx_count_hk".into(), RecordValue::Int(v));
        }
        if let Some(v) = self.tx_count_kln {
            f.insert("tx_count_kln".into(), RecordValue::Int(v));
        }
        if let Some(v) = self.tx_count_nt {
            f.insert("tx_count_nt".into(), RecordValue::Int(v));
        }
        for (k, v) in [
            ("net_ft_price", self.net_ft_price),
            ("net_ft_price_hk", self.net_ft_price_hk),
            ("net_ft_price_kln", self.net_ft_price_kln),
            ("net_ft_price_nt", self.net_ft_price_nt),
            ("ft_price", self.ft_price),
            ("ft_rent", self.ft_rent),
            ("net_ft_rent", self.net_ft_rent),
            ("monthly_perc", self.monthly_perc),
            ("monthly_perc_hk", self.monthly_perc_hk),
            ("monthly_perc_kln", self.monthly_perc_kln),
            ("monthly_perc_nt", self.monthly_perc_nt),
        ] {
            if let Some(n) = v {
                f.insert(k.into(), RecordValue::Float(n));
            }
        }
        if f.is_empty() {
            return None;
        }
        Some(NormalizedRecord {
            source: DataSource::Hkp,
            dataset: PRICE_INDEX_ID.into(),
            record_id: month,
            fields: f,
            fetched_at: now,
        })
    }
}

/// One row of the economic-indicators series.
#[derive(Debug, Deserialize)]
struct EconIndicatorPoint {
    #[serde(rename = "Mortgage_Interest_Rate", default)]
    mortgage_interest_rate: Option<f64>,
    #[serde(rename = "Rental_Yield", default)]
    rental_yield: Option<f64>,
    #[serde(rename = "Real_Saving_Interest_Rate", default)]
    real_saving_interest_rate: Option<f64>,
    #[serde(rename = "Hang_Seng_Index", default)]
    hang_seng_index: Option<f64>,
    #[serde(rename = "US_Dollar_Index", default)]
    us_dollar_index: Option<f64>,
    #[serde(rename = "Unemployment_Rate", default)]
    unemployment_rate: Option<f64>,
    #[serde(rename = "Affordability_Ratio", default)]
    affordability_ratio: Option<f64>,
    #[serde(rename = "Rental_Affordability_Ratio", default)]
    rental_affordability_ratio: Option<f64>,
    #[serde(rename = "House_Price_to_Income_Ratio", default)]
    house_price_to_income_ratio: Option<f64>,
    date: String,
}

impl EconIndicatorPoint {
    fn into_record(self, now: chrono::DateTime<Utc>) -> Option<NormalizedRecord> {
        let month = iso_ts_to_month(&self.date)?;
        let mut f = BTreeMap::new();
        for (k, v) in [
            ("mortgage_interest_rate", self.mortgage_interest_rate),
            ("rental_yield", self.rental_yield),
            ("real_saving_interest_rate", self.real_saving_interest_rate),
            ("hang_seng_index", self.hang_seng_index),
            ("us_dollar_index", self.us_dollar_index),
            ("unemployment_rate", self.unemployment_rate),
            ("affordability_ratio", self.affordability_ratio),
            ("rental_affordability_ratio", self.rental_affordability_ratio),
            ("house_price_to_income_ratio", self.house_price_to_income_ratio),
        ] {
            if let Some(n) = v {
                f.insert(k.into(), RecordValue::Float(n));
            }
        }
        if f.is_empty() {
            return None;
        }
        Some(NormalizedRecord {
            source: DataSource::Hkp,
            dataset: ECON_INDICATORS_ID.into(),
            record_id: month,
            fields: f,
            fetched_at: now,
        })
    }
}

/// One month of Land Registry registrations, broken down by property class.
#[derive(Debug, Deserialize)]
struct LangRegRecord {
    #[serde(default, rename = "firsthand_private")]
    firsthand_private: Option<ClassBreakdown>,
    #[serde(default, rename = "secondhand_private")]
    secondhand_private: Option<ClassBreakdown>,
    #[serde(default, rename = "firsthand_hos")]
    firsthand_hos: Option<ClassBreakdown>,
    #[serde(default, rename = "industrial")]
    industrial: Option<ClassBreakdown>,
    #[serde(default, rename = "commercial")]
    commercial: Option<ClassBreakdown>,
    #[serde(default, rename = "shop")]
    shop: Option<ClassBreakdown>,
}

#[derive(Debug, Deserialize)]
struct ClassBreakdown {
    #[serde(default)]
    number: Option<f64>,
    #[serde(default)]
    amount: Option<f64>,
    #[serde(default, rename = "number_chg")]
    number_chg: Option<f64>,
    #[serde(default, rename = "amount_chg")]
    amount_chg: Option<f64>,
}

impl LangRegRecord {
    /// The market-insight `langRegRecords[]` carries only the latest month,
    /// no date field — we infer the month from "previous month" (data is
    /// published mid-month for the prior month).
    fn into_record(self, now: chrono::DateTime<Utc>) -> Option<NormalizedRecord> {
        // Best-effort: derive the month from `now` minus one month. The HKP
        // page reflects "current published month" which is last month's data.
        let prev = now
            .checked_sub_signed(chrono::Duration::days(30))
            .unwrap_or(now);
        let month = prev.format("%Y-%m").to_string();
        let mut f = BTreeMap::new();
        for (class_prefix, bd) in [
            ("firsthand_private", self.firsthand_private),
            ("secondhand_private", self.secondhand_private),
            ("firsthand_hos", self.firsthand_hos),
            ("industrial", self.industrial),
            ("commercial", self.commercial),
            ("shop", self.shop),
        ] {
            let Some(bd) = bd else { continue };
            if let Some(n) = bd.number {
                f.insert(format!("{class_prefix}_number"), RecordValue::Float(n));
            }
            if let Some(a) = bd.amount {
                f.insert(format!("{class_prefix}_amount"), RecordValue::Float(a));
            }
            if let Some(c) = bd.number_chg {
                f.insert(format!("{class_prefix}_number_chg"), RecordValue::Float(c));
            }
            if let Some(c) = bd.amount_chg {
                f.insert(format!("{class_prefix}_amount_chg"), RecordValue::Float(c));
            }
        }
        if f.is_empty() {
            return None;
        }
        Some(NormalizedRecord {
            source: DataSource::Hkp,
            dataset: LAND_REGISTRY_SUMMARY_ID.into(),
            record_id: month,
            fields: f,
            fetched_at: now,
        })
    }
}

/// Parse the 12-month HTML table for territory-wide monthly registration
/// totals. The page has 3 tables; table[0] carries the rolling 12-month
/// breakdown (整體物業註冊宗數 + 整體物業註冊金額 by month). We extract
/// the month headers + the 整體 totals row, producing one record per month
/// with `overall_units` and `overall_amount_hkd_bn`.
fn parse_12mo_totals(html: &str, now: chrono::DateTime<Utc>) -> Vec<NormalizedRecord> {
    // Find the months header row — it carries values like "2025 年 8月", "9月", …
    // Each cell is either "YYYY 年 MM月" (year-boundary month) or just "MM月".
    let mut months: Vec<(i32, u32)> = Vec::new();
    // Walk the body and find runs of "YYYY 年" then collect the next 12 month cells.
    let lower = html.to_lowercase();
    // Anchor on the first <table>; we only need the first table for totals.
    let table_start = match lower.find("<table") {
        Some(s) => s,
        None => return Vec::new(),
    };
    let table_end = lower[table_start..]
        .find("</table>")
        .map(|e| table_start + e)
        .unwrap_or(html.len());
    let table = &html[table_start..table_end];
    // Collect all <th> and <td> cell texts in order.
    let cells: Vec<String> = collect_cells(table);
    // Walk the cells: when we see "… 年 …月" or "… 月", record the month.
    let mut current_year: Option<i32> = None;
    for cell in &cells {
        let c = cell.trim();
        if c.contains("年") {
            // "2025 年" or "2025 年 8月" — pull the year (and month if present).
            if let Some(y) = parse_first_int(c) {
                current_year = Some(y as i32);
            }
            if let Some(m) = extract_month_number(c) {
                if let Some(y) = current_year {
                    months.push((y, m));
                }
            }
        } else if let Some(m) = extract_month_number(c) {
            if let Some(y) = current_year {
                months.push((y, m));
            }
        }
    }
    // Now find the 整體物業註冊宗數 row — its cells are the month-by-month
    // totals in the same column order as the header.
    let mut units_by_month: BTreeMap<(i32, u32), i64> = BTreeMap::new();
    let mut amount_by_month: BTreeMap<(i32, u32), f64> = BTreeMap::new();
    // Find the units row: a <tr> whose first cell is 整體物業註冊宗數.
    if let Some(units_row) = find_row_starting_with(table, "整體物業註冊宗數") {
        let row_cells: Vec<String> = collect_cells(units_row);
        for (i, val) in row_cells.iter().enumerate() {
            if i == 0 {
                continue;
            }
            if let Some(m) = months.get(i - 1) {
                if let Some(n) = parse_first_int(val) {
                    units_by_month.insert(*m, n);
                }
            }
        }
    }
    if let Some(amount_row) = find_row_starting_with(table, "整體物業註冊金額") {
        let row_cells: Vec<String> = collect_cells(amount_row);
        for (i, val) in row_cells.iter().enumerate() {
            if i == 0 {
                continue;
            }
            if let Some(m) = months.get(i - 1) {
                if let Some(n) = val
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.')
                    .collect::<String>()
                    .parse::<f64>()
                    .ok()
                {
                    amount_by_month.insert(*m, n);
                }
            }
        }
    }
    // Emit one record per month we found data for.
    let mut out = Vec::new();
    for ((y, m), units) in units_by_month {
        let mut f = BTreeMap::new();
        f.insert("overall_units".into(), RecordValue::Int(units));
        if let Some(amt) = amount_by_month.get(&(y, m)) {
            f.insert("overall_amount_hkd_bn".into(), RecordValue::Float(*amt));
        }
        out.push(NormalizedRecord {
            source: DataSource::Hkp,
            dataset: LAND_REGISTRY_SUMMARY_ID.into(),
            record_id: format!("{y:04}-{m:02}"),
            fields: f,
            fetched_at: now,
        });
    }
    out
}

// ---- tiny helpers ----

/// `"1997-01-01T00:00:00.000Z"` → `"1997-01"`.
fn iso_ts_to_month(ts: &str) -> Option<String> {
    // Take the first 7 chars if they look like YYYY-MM.
    let bytes = ts.as_bytes();
    if bytes.len() >= 7 && bytes[4] == b'-' {
        let s = std::str::from_utf8(&bytes[..7]).ok()?;
        if s.chars().filter(|c| *c == '-').count() == 1 {
            return Some(s.to_string());
        }
    }
    None
}

/// Pull the month number out of a cell like "8月" or "2025 年 8月" or "12月".
fn extract_month_number(s: &str) -> Option<u32> {
    let idx = s.find("月")?;
    let before = &s[..idx];
    // Walk back from 月 to the first non-digit.
    let digits_end = before.len();
    let mut digits_start = digits_end;
    for (i, ch) in before.char_indices().rev() {
        if ch.is_ascii_digit() {
            digits_start = i;
        } else {
            // Allow one space between digits and 月.
            if ch == ' ' {
                continue;
            }
            break;
        }
    }
    let digits = &before[digits_start..digits_end];
    digits.parse::<u32>().ok().filter(|m| *m >= 1 && *m <= 12)
}

fn parse_first_int(s: &str) -> Option<i64> {
    let cleaned: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '.')
        .filter(|c| *c != ',')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned
        .split('.')
        .next()
        .unwrap_or(&cleaned)
        .parse()
        .ok()
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_cells(html: &str) -> Vec<String> {
    let lower = html.to_lowercase();
    let mut out = Vec::new();
    let mut rest = 0;
    while let Some(rel) = lower[rest..]
        .find("<th")
        .or_else(|| lower[rest..].find("<td"))
        .map(|p| rest + p)
    {
        let after_open = match html[rel..].find('>') {
            Some(p) => rel + p + 1,
            None => break,
        };
        let close_rel = match lower[after_open..]
            .find("</th>")
            .or_else(|| lower[after_open..].find("</td>"))
        {
            Some(c) => c,
            None => break,
        };
        let close = after_open + close_rel;
        out.push(strip_tags(&html[after_open..close]));
        rest = close + 5;
    }
    out
}

/// Find a `<tr>…</tr>` whose first non-empty cell starts with `prefix`.
fn find_row_starting_with<'a>(html: &'a str, prefix: &str) -> Option<&'a str> {
    let lower = html.to_lowercase();
    let mut rest = 0;
    while let Some(rel) = lower[rest..].find("<tr") {
        let tr_start = rest + rel;
        let after_open = html[tr_start..].find('>').map(|p| tr_start + p + 1)?;
        let close = lower[after_open..]
            .find("</tr>")
            .map(|c| after_open + c)?;
        let row = &html[after_open..close];
        let first_cell = collect_cells(row).into_iter().next().unwrap_or_default();
        if first_cell.starts_with(prefix) {
            return Some(row);
        }
        rest = close + 5;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_ts_to_month_truncates() {
        assert_eq!(iso_ts_to_month("1997-01-01T00:00:00.000Z"), Some("1997-01".into()));
        assert_eq!(iso_ts_to_month("2026-07-20T00:00:00.000Z"), Some("2026-07".into()));
        assert_eq!(iso_ts_to_month("nonsense"), None);
    }

    #[test]
    fn extracts_next_data_json_from_html() {
        let html = r#"<html><head>
            <script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"mrIndex":[]}}}</script>
        </head></html>"#;
        let json = extract_next_data(html).unwrap();
        let parsed: NextData = serde_json::from_str(&json).unwrap();
        assert!(parsed.props.page_props.mr_index.is_empty());
    }

    #[test]
    fn parses_mr_index_point_into_record() {
        let raw = r#"{"date":"2026-07-20T00:00:00.000Z","weekly":true,"mr_index":148.7,"mr_index_hk":156.8,"mr_index_kln":156.6,"mr_index_nt":134,"monthly_perc":0.3,"tx_count":2580,"tx_count_hk":454,"net_ft_price":14749.0}"#;
        let p: MrIndexPoint = serde_json::from_str(raw).unwrap();
        let now = Utc::now();
        let rec = p.into_record(now).unwrap();
        assert_eq!(rec.record_id, "2026-07");
        assert_eq!(rec.fields.get("mr_index"), Some(&RecordValue::Float(148.7)));
        assert_eq!(rec.fields.get("tx_count"), Some(&RecordValue::Int(2580)));
    }

    #[test]
    fn parses_economic_indicators_point() {
        let raw = r#"{"Mortgage_Interest_Rate":8.75,"Rental_Yield":4.4,"Hang_Seng_Index":13321.79,"date":"1997-01-01T00:00:00.000Z"}"#;
        let p: EconIndicatorPoint = serde_json::from_str(raw).unwrap();
        let now = Utc::now();
        let rec = p.into_record(now).unwrap();
        assert_eq!(rec.record_id, "1997-01");
        assert_eq!(
            rec.fields.get("mortgage_interest_rate"),
            Some(&RecordValue::Float(8.75))
        );
    }

    #[test]
    fn parses_land_registry_breakdown() {
        let raw = r#"{"firsthand_private":{"number":536,"amount":94.24,"number_chg":-63.8,"amount_chg":-50.3},"secondhand_private":{"number":2580,"amount":174.63}}"#;
        let r: LangRegRecord = serde_json::from_str(raw).unwrap();
        let now = Utc::now();
        let rec = r.into_record(now).unwrap();
        // record_id is the prior month — just check it parses as YYYY-MM.
        assert!(rec.record_id.len() == 7);
        let n = rec.fields.get("firsthand_private_number").and_then(|v| match v {
            RecordValue::Float(f) => Some(*f as i64),
            _ => None,
        });
        assert_eq!(n, Some(536));
        let c = rec.fields.get("firsthand_private_number_chg").and_then(|v| match v {
            RecordValue::Float(f) => Some(*f),
            _ => None,
        });
        assert_eq!(c, Some(-63.8));
    }

    #[test]
    fn extracts_month_number_from_cell() {
        assert_eq!(extract_month_number("8月"), Some(8));
        assert_eq!(extract_month_number("2025 年 8月"), Some(8));
        assert_eq!(extract_month_number("12月"), Some(12));
        assert_eq!(extract_month_number("(截至17日)"), None);
        assert_eq!(extract_month_number("宗數"), None);
    }

    #[test]
    fn parses_12mo_table_totals() {
        // Miniature of the live first table.
        let html = r#"<table><thead><tr>
            <th>&nbsp;</th>
            <th>2025 年 8月</th><th>9月</th><th>10月</th>
        </tr></thead><tbody>
            <tr><td>整體物業註冊宗數</td><td>6,462</td><td>6,870</td><td>7,190</td></tr>
            <tr><td>整體物業註冊金額 (億元)</td><td>477.8</td><td>534.8</td><td>579.0</td></tr>
        </tbody></table>"#;
        let now = Utc::now();
        let recs = parse_12mo_totals(html, now);
        assert_eq!(recs.len(), 3);
        // Sorted by month (record_id).
        assert_eq!(recs[0].record_id, "2025-08");
        assert_eq!(
            recs[0].fields.get("overall_units"),
            Some(&RecordValue::Int(6462))
        );
        assert_eq!(
            recs[0].fields.get("overall_amount_hkd_bn"),
            Some(&RecordValue::Float(477.8))
        );
        assert_eq!(recs[2].record_id, "2025-10");
    }
}
