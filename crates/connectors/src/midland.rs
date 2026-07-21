//! Midland Realty (美聯物業) connector — 銀主盤 (foreclosure) listings.
//!
//! Source: `www.midland.com.hk` + its JSON API host `data.midland.com.hk`
//! (both fronted by the `hkgov-proxy` Worker — CloudFront WAF geo-blocks
//! non-HK IPs).
//!
//! The Midland listing site is a Next.js SPA whose listing grid hydrates via
//! XHR — but unlike typical SPAs, the data XHR endpoint
//! `data.midland.com.hk/search/v2/properties` returns full structured JSON.
//! We skip rendering the SPA entirely and call the API directly.
//!
//! ## Auth
//!
//! The data API requires `Authorization: Bearer <JWT>`. The JWT is a
//! build-embedded bootstrap token (`runtimeConfig.BUILD_TOKEN` in the page's
//! `__NEXT_DATA__`), issued in 2020 with no expiry — same for every visitor.
//! The connector fetches the SPA shell once to extract that token, then uses
//! it for the search API.
//!
//! ## Dataset
//!
//! `midland-bank-listings` — 銀主盤 (bank-owned / foreclosure) listings.
//! We hit the search API with `category=foreclosure&tx_type=S` to scope to
//! the foreclosure pool. Each record is one listing:
//!   - `record_id` = listing `id` (e.g. `M350591670`)
//!   - `fields`: `estate_name`, `region`, `subregion`, `address`, `tx_type`,
//!     `sale_price_hkd`, `rent_hkd`, `net_area_sqft`, `build_area_sqft`,
//!     `bedroom`, `is_foreclosure`, `source_url`.
//!
//! The connector paginates through all foreclosure results on each refresh.

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

const DATASET_ID: &str = "midland-bank-listings";
/// The Midland saved-search hash for the 銀主盤 category. The site serves
/// this as the canonical 銀主盤 link in its own nav (verified July 2026).
const FORECLOSURE_HASH: &str = "3b9d6de8";

static DATASETS: OnceLock<Vec<DatasetSpec>> = OnceLock::new();

fn datasets() -> &'static [DatasetSpec] {
    DATASETS.get_or_init(|| {
        vec![DatasetSpec {
            id: DATASET_ID,
            title: "Midland 銀主盤 — Foreclosure Listings",
            description: Some(
                "Midland Realty (美聯物業) 銀主盤 (bank-owned / foreclosure) \
                 listings, pulled from the data.midland.com.hk search API \
                 that backs the Midland SPA. Each record is one listing \
                 (active 銀主盤 stock) with estate name, region, address, \
                 asking price, area, and bedroom count. record_id = Midland \
                 listing id (e.g. M350591670). Refreshed every 6h — the \
                 foreclosure pool rotates slowly.",
            ),
            category: Category::Property,
            tags: &["midland", "銀主盤", "foreclosure", "bank-owned", "美聯"],
            cadence: Cadence::Daily,
            refresh_interval_secs: 6 * 3600,
        }]
    })
}

/// The SPA shell URL we fetch once to extract the build token.
const LISTING_PAGE_URL: &str =
    "https://www.midland.com.hk/zh-hk/list/buy/%E6%90%9C%E5%B0%8B-H-3b9d6de8";

/// One page of the search API — 24 results max per call (verified).
const SEARCH_PAGE_SIZE: u32 = 24;
/// Safety cap on pagination. ~50 銀主盤 in the pool today; cap at 500 to
/// avoid runaway loops if the API ever returns a misbehaving `count`.
const MAX_PAGES: u32 = 25;

pub struct MidlandConnector {
    client: reqwest::Client,
    enabled: bool,
    upstream: UpstreamSettings,
}

impl MidlandConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(60_000))
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

    /// Fetch the SPA shell and pull out the `BUILD_TOKEN` JWT embedded in
    /// `__NEXT_DATA__.runtimeConfig.BUILD_TOKEN`. Cached cheaply — the token
    /// is a build-time constant.
    async fn fetch_build_token(&self) -> Result<String> {
        let html = worker_fetch(&self.client, &self.upstream, LISTING_PAGE_URL, &[]).await?;
        extract_build_token(&html).ok_or_else(|| Error::Decode {
            origin: "midland",
            backtrace: serde::de::Error::custom(
                "no runtimeConfig.BUILD_TOKEN in __NEXT_DATA__ — page shape changed?",
            ),
        })
    }

    /// Hit the search API for one page of foreclosure results.
    async fn fetch_search_page(&self, auth_token: &str, page: u32) -> Result<SearchResponse> {
        let url = format!(
            "https://data.midland.com.hk/search/v2/properties?q={FORECLOSURE_HASH}&ad=true&lang=zh-hk&currency=HKD&unit=feet&search_behavior=normal&tx_type=S&category=foreclosure&limit={SEARCH_PAGE_SIZE}&page={page}"
        );
        let auth_value = format!("Bearer {auth_token}");
        let body = worker_fetch(
            &self.client,
            &self.upstream,
            &url,
            &[("authorization", auth_value.as_str())],
        )
        .await?;
        let resp: SearchResponse = serde_json::from_str(&body).map_err(|e| Error::Decode {
            origin: "midland",
            backtrace: serde::de::Error::custom(format!("search response decode: {e}")),
        })?;
        Ok(resp)
    }
}

#[async_trait]
impl Connector for MidlandConnector {
    fn source(&self) -> DataSource {
        DataSource::Midland
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
                "midland: Worker proxy not configured (set HKGOV_UPSTREAM__PROXY_URL + \
                 the CF Access service-token fields)"
                    .into(),
            ));
        }
        if dataset != DATASET_ID {
            return Err(Error::Internal(format!(
                "midland: unknown dataset {dataset}"
            )));
        }
        let token = self.fetch_build_token().await?;
        let now = Utc::now();
        let mut all_records = Vec::new();
        let mut total_reported: u64 = 0;
        let mut pages_fetched: u32 = 0;
        let mut page = 1u32;
        loop {
            let resp = self.fetch_search_page(&token, page).await?;
            total_reported = total_reported.max(resp.count.unwrap_or(0));
            let result_count = resp.result.len();
            for item in resp.result.into_iter() {
                if let Some(rec) = item.into_record(now) {
                    all_records.push(rec);
                }
            }
            pages_fetched += 1;
            // Stop when we've collected everything (or hit the safety cap,
            // or the page was empty — defensive against bad counts).
            let collected = (page * SEARCH_PAGE_SIZE) as u64;
            if collected >= total_reported || page >= MAX_PAGES || result_count == 0 {
                break;
            }
            page += 1;
        }
        tracing::info!(
            dataset,
            pages = pages_fetched,
            total_reported,
            collected = all_records.len(),
            "midland: paginated foreclosure listings"
        );
        Ok(all_records)
    }
}

// ---- typed JSON shape of the search API response ----

#[derive(Debug, Deserialize)]
struct SearchResponse {
    /// Total number of matching listings across all pages.
    #[serde(default)]
    count: Option<u64>,
    #[serde(default)]
    result: Vec<ListingItem>,
}

#[derive(Debug, Deserialize)]
struct ListingItem {
    /// Midland listing id, e.g. `"M350591670"`.
    #[serde(default)]
    id: Option<String>,
    /// Estate / building name (Chinese).
    #[serde(default)]
    estate: Option<Nest>,
    #[serde(default)]
    region: Option<Nest>,
    #[serde(default)]
    subregion: Option<Nest>,
    /// Free-form address line.
    #[serde(default, rename = "full_address")]
    full_address: Option<String>,
    /// Build area in sqft.
    #[serde(default, rename = "area")]
    area: Option<f64>,
    /// Net (saleable) area in sqft.
    #[serde(default, rename = "net_area")]
    net_area: Option<f64>,
    /// Asking price in HKD.
    #[serde(default)]
    price: Option<f64>,
    #[serde(default, rename = "price_hkd")]
    price_hkd: Option<f64>,
    #[serde(default)]
    rent: Option<f64>,
    #[serde(default, rename = "rent_hkd")]
    rent_hkd: Option<f64>,
    /// Price per net sqft (derived, useful for cross-listing comparison).
    #[serde(default, rename = "price_over_net_area")]
    price_over_net_area: Option<f64>,
    /// Bedroom count (number or label like "開放式").
    #[serde(default)]
    bedroom: Option<serde_json::Value>,
    /// Transaction type, e.g. `["S"]` for sale.
    #[serde(default, rename = "tx_type")]
    tx_type: Vec<String>,
    /// Tags include `foreclosure` / `bank` for 銀主盤.
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Nest {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

impl ListingItem {
    fn into_record(self, now: chrono::DateTime<Utc>) -> Option<NormalizedRecord> {
        let id = self.id.clone()?; // no id → can't dedupe, skip
        let mut f = BTreeMap::new();
        if let Some(n) = self.estate.as_ref().and_then(|e| e.name.as_deref()) {
            f.insert("estate_name".into(), RecordValue::Str(n.to_string()));
        }
        if let Some(n) = self.region.as_ref().and_then(|r| r.name.as_deref()) {
            f.insert("region".into(), RecordValue::Str(n.to_string()));
        }
        if let Some(n) = self.subregion.as_ref().and_then(|s| s.name.as_deref()) {
            f.insert("subregion".into(), RecordValue::Str(n.to_string()));
        }
        if let Some(a) = self.full_address {
            f.insert("address".into(), RecordValue::Str(a));
        }
        if let Some(v) = self.area {
            f.insert("build_area_sqft".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.net_area {
            f.insert("net_area_sqft".into(), RecordValue::Float(v));
        }
        // Prefer the explicit HKD field; fall back to the generic price.
        let price = self.price_hkd.or(self.price);
        if let Some(p) = price {
            f.insert("sale_price_hkd".into(), RecordValue::Float(p));
        }
        let rent = self.rent_hkd.or(self.rent);
        if let Some(r) = rent {
            f.insert("rent_hkd".into(), RecordValue::Float(r));
        }
        if let Some(ppf) = self.price_over_net_area {
            f.insert("price_per_net_sqft".into(), RecordValue::Float(ppf));
        }
        // bedroom may be a number or a string label — keep as-is via JSON.
        if let Some(b) = self.bedroom {
            let v = match b {
                serde_json::Value::Number(n) => n
                    .as_f64()
                    .map(RecordValue::Float)
                    .unwrap_or(RecordValue::Null),
                serde_json::Value::String(s) => RecordValue::Str(s),
                other => RecordValue::Str(other.to_string()),
            };
            if !matches!(v, RecordValue::Null) {
                f.insert("bedroom".into(), v);
            }
        }
        if !self.tx_type.is_empty() {
            f.insert("tx_type".into(), RecordValue::Str(self.tx_type.join(",")));
        }
        // The 銀主盤 flag — we know this is foreclosure because we queried
        // category=foreclosure, but record the explicit tag when present.
        let is_foreclosure = self
            .tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case("foreclosure") || t.contains("銀主"));
        f.insert("is_foreclosure".into(), RecordValue::Bool(is_foreclosure));
        if !self.tags.is_empty() {
            f.insert("tags".into(), RecordValue::Str(self.tags.join(",")));
        }
        f.insert(
            "source_url".into(),
            RecordValue::Str(LISTING_PAGE_URL.to_string()),
        );
        Some(NormalizedRecord {
            source: DataSource::Midland,
            dataset: DATASET_ID.into(),
            record_id: id,
            fields: f,
            fetched_at: now,
        })
    }
}

// ---- __NEXT_DATA__ token extraction ----

/// Extract `runtimeConfig.BUILD_TOKEN` from the page's `__NEXT_DATA__` JSON.
/// Returns None if the script tag, JSON, or token field is missing.
fn extract_build_token(html: &str) -> Option<String> {
    let marker = r#"id="__NEXT_DATA__""#;
    let start = html.find(marker)?;
    let after_marker = &html[start..];
    let json_start = after_marker.find('>')? + 1;
    let json_body = &after_marker[json_start..];
    let json_end = json_body.find("</script>")?;
    let json_str = json_body[..json_end].trim();
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let tok = v
        .get("runtimeConfig")?
        .get("BUILD_TOKEN")?
        .as_str()?
        .to_string();
    if tok.is_empty() {
        None
    } else {
        Some(tok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_build_token_from_next_data() {
        let html = r#"<html><head>
            <script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{}},"runtimeConfig":{"BUILD_TOKEN":"eyJabc123.BUILD.day"}}</script>
        </head></html>"#;
        assert_eq!(
            extract_build_token(html).as_deref(),
            Some("eyJabc123.BUILD.day")
        );
    }

    #[test]
    fn returns_none_when_token_missing() {
        let html = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{}}}</script>"#;
        assert!(extract_build_token(html).is_none());
    }

    #[test]
    fn parses_real_search_response_sample() {
        // Trimmed version of the live API response shape.
        let raw = r#"{
            "count": 42,
            "actual_count": 47,
            "filter": {"category": "foreclosure"},
            "result": [
                {
                    "id": "M350591670",
                    "estate": {"id": "E000014416", "name": "溱柏"},
                    "region": {"id": "30", "name": "新界"},
                    "subregion": {"id": "3014", "name": "元朗"},
                    "area": 820,
                    "net_area": 660,
                    "price_hkd": 5200000,
                    "price_over_net_area": 7878,
                    "bedroom": 2,
                    "tx_type": ["S"],
                    "tags": ["foreclosure", "bank"]
                }
            ]
        }"#;
        let resp: SearchResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.count, Some(42));
        assert_eq!(resp.result.len(), 1);
        let now = Utc::now();
        let rec = resp
            .result
            .into_iter()
            .next()
            .unwrap()
            .into_record(now)
            .unwrap();
        assert_eq!(rec.record_id, "M350591670");
        assert_eq!(
            rec.fields.get("estate_name"),
            Some(&RecordValue::Str("溱柏".into()))
        );
        assert_eq!(
            rec.fields.get("region"),
            Some(&RecordValue::Str("新界".into()))
        );
        assert_eq!(
            rec.fields.get("net_area_sqft"),
            Some(&RecordValue::Float(660.0))
        );
        assert_eq!(
            rec.fields.get("sale_price_hkd"),
            Some(&RecordValue::Float(5_200_000.0))
        );
        assert_eq!(
            rec.fields.get("is_foreclosure"),
            Some(&RecordValue::Bool(true))
        );
        assert_eq!(rec.fields.get("bedroom"), Some(&RecordValue::Float(2.0)));
    }

    #[test]
    fn skips_listing_without_id() {
        let item: ListingItem =
            serde_json::from_str(r#"{"estate":{"name":"No-ID Estate"},"price_hkd":1000000}"#)
                .unwrap();
        let now = Utc::now();
        let rec = item.into_record(now);
        assert!(rec.is_none(), "listings without an id are dropped");
    }

    #[test]
    fn handles_string_bedroom_label() {
        let item: ListingItem =
            serde_json::from_str(r#"{"id":"M1","bedroom":"開放式","tx_type":["S"]}"#).unwrap();
        let now = Utc::now();
        let rec = item.into_record(now).unwrap();
        assert_eq!(
            rec.fields.get("bedroom"),
            Some(&RecordValue::Str("開放式".into()))
        );
    }
}
