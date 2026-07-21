//! Connectors to Hong Kong Government public data sources.
//!
//! Each connector is responsible for ONE upstream family and knows how to turn
//! its raw payloads into [`NormalizedRecord`]s. The ingest pipeline orchestrates
//! them; the serving API never calls a connector directly.
//!
//! Implemented connectors (all verified live — see docs/DATA_SOURCES.md):
//! - **HKMA** (`api.hkma.gov.hk`) — monetary & market statistics, press.
//! - **data.gov.hk** (`api.data.gov.hk/v2/filter`) — cross-departmental data.
//! - **Press** (`api.hkma.gov.hk/public/press-releases`) — HKMA press releases.
//! - **LandsD/CSDI** (data.gov.hk historical archive) — geospatial catalog.
//! - **Immigration** (`immd.gov.hk`) — daily border-crossing traffic.
//! - **RVD** (`rvd.gov.hk`) — monthly property price/rental indices.
//! - **Land Registry** (`landreg.gov.hk`) — monthly property transactions.
//!
//! Commercial property portals (v3) — direct or via the `hkgov-proxy` Worker:
//! - **Chung Sen** (`chungsen.com.hk`) — 筍盤推介 / 銀主獨家 auction listings.
//! - **AA Property** (`aaproperty.com.hk`) — open auction lot list.
//! - **HKP** (`hkp.com.hk`) — 二手樓價指數 + 12-month Land Registry stats.
//! - **Midland** (`midland.com.hk`) — 銀主盤 (foreclosure) listings.

pub mod aaproperty;
pub mod chungsen;
pub mod datagovhk;
pub mod hkma;
/// The verified HKMA dataset table (internal — used by `hkma`).
mod hkma_datasets;
pub mod hkp;
pub mod immigration;
pub mod landregistry;
pub mod landsd;
pub mod midland;
pub mod press;
pub mod property_canon;
pub mod registry;
pub mod resilience;
pub mod rvd;

use async_trait::async_trait;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, Result, UpstreamSettings,
};
use serde::Deserialize;

/// Strip a leading UTF-8 BOM if present. HK gov feeds occasionally ship one
/// (the Land Registry JSON feed did — see commit "strip UTF-8 BOM"). serde_json
/// rejects a leading BOM, so callers must strip it before parsing text bodies.
pub(crate) fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Fetch one URL through the `hkgov-proxy` Cloudflare Worker (fronting the
/// geo-blocked commercial portals). Shared by the Hkp + Midland connectors.
///
/// The Worker wraps the upstream body in a JSON envelope:
/// ```jsonc
/// { "status": 200, "ct": "...", "size": 12345, "body": "<raw upstream bytes>" }
/// ```
/// This helper unwraps the `body` field. Non-2xx Worker envelopes (Worker
/// problem) map to `Error::Upstream`. A 2xx envelope carrying a non-2xx
/// upstream status (Worker OK, upstream 4xx) is returned as the body with the
/// upstream status preserved in the `Error`'s `detail` field — callers
/// typically handle this as a soft-fail (circuit-breaker counts it).
///
/// `extra_headers` carries optional per-host request headers forwarded through
/// the Worker as `?header_<name>=<value>` query params. Midland's API needs
/// `Authorization: Bearer <BUILD_TOKEN>`; HKP's data API needs the same shape.
pub(crate) async fn worker_fetch(
    client: &reqwest::Client,
    settings: &UpstreamSettings,
    upstream_url: &str,
    extra_headers: &[(&str, &str)],
) -> Result<String> {
    let proxy_url = settings.proxy_url.as_deref().ok_or_else(|| {
        Error::Internal(
            "worker_fetch: upstream.proxy_url not configured — set \
             HKGOV_UPSTREAM__PROXY_URL + the CF Access service-token fields"
                .into(),
        )
    })?;
    // Build the Worker URL: <proxy>/fetch?url=<encoded>&header_<name>=<val>&...
    // Build the query string out-of-line so the `url::UrlQuery` borrow is
    // dropped before the .await — holding it across the await makes the
    // resulting future !Send, which breaks the Connector trait's `Send` bound.
    let mut query: Vec<(String, String)> = Vec::with_capacity(1 + extra_headers.len());
    query.push(("url".to_string(), upstream_url.to_string()));
    for (name, value) in extra_headers {
        query.push((format!("header_{name}"), (*value).to_string()));
    }
    let worker_url = format!(
        "{}/fetch?{}",
        proxy_url.trim_end_matches('/'),
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .finish()
    );
    let resp = client
        .get(&worker_url)
        .header(
            "CF-Access-Client-Id",
            settings.proxy_cf_access_client_id.as_deref().unwrap_or(""),
        )
        .header(
            "CF-Access-Client-Secret",
            settings
                .proxy_cf_access_client_secret
                .as_deref()
                .unwrap_or(""),
        )
        .send()
        .await
        .map_err(|e| Error::Upstream {
            origin: "worker",
            status: 0,
            detail: format!("transport: {e}"),
        })?;
    let status = resp.status().as_u16();
    if status >= 400 {
        // Worker itself returned an error (e.g. 403 host-not-allowed, 502
        // upstream network error). Map it cleanly.
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Upstream {
            origin: "worker",
            status,
            detail: body,
        });
    }
    let env_json = resp.text().await.map_err(|e| Error::Upstream {
        origin: "worker",
        status: 0,
        detail: format!("body read: {e}"),
    })?;
    let env: WorkerEnvelope = serde_json::from_str(&env_json).map_err(|e| Error::Decode {
        origin: "worker",
        backtrace: serde::de::Error::custom(format!("envelope decode: {e}")),
    })?;
    // If the upstream URL itself returned non-2xx, surface that as an Upstream
    // error — callers (circuit breaker) will count it.
    if env.status >= 400 {
        return Err(Error::Upstream {
            origin: "worker",
            status: env.status,
            detail: format!(
                "upstream {} returned {}",
                upstream_url,
                env.body.chars().take(200).collect::<String>()
            ),
        });
    }
    Ok(env.body)
}

/// The JSON envelope the `hkgov-proxy` Worker returns on success.
#[derive(Debug, Deserialize)]
struct WorkerEnvelope {
    status: u16,
    #[serde(default)]
    #[allow(dead_code)]
    ct: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    size: usize,
    body: String,
}

/// What every connector must do. Implementations are constructed once at startup
/// and shared (via `Arc`) across the ingestion scheduler and reload fan-out.
#[async_trait]
pub trait Connector: Send + Sync + 'static {
    /// Which [`DataSource`] family this connector handles.
    fn source(&self) -> DataSource;

    /// Datasets this connector can fetch. Stable identifiers — HKMA uses its
    /// documentation slugs (e.g. `capital-market-statistics`).
    fn datasets(&self) -> &[DatasetSpec];

    /// Fetch one dataset's records. Large datasets should be paged upstream and
    /// streamed back; the caller decides how big a batch to cache.
    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>>;

    /// The upstream URL the records for `dataset` are fetched from (M1 lineage).
    /// `None` by default (connectors that don't track their URL); connectors
    /// that do override this so the gateway's lineage index carries a verifiable
    /// provenance pointer. Non-breaking: existing connectors keep compiling.
    fn upstream_url(&self, _dataset: &str) -> Option<String> {
        None
    }

    /// The wire format the connector decodes (M1 lineage). `Unknown` by default;
    /// connectors that know their format override this so the lineage record
    /// documents the upstream shape. Non-breaking.
    fn upstream_format(&self, _dataset: &str) -> hkgov_store::UpstreamFormat {
        hkgov_store::UpstreamFormat::Unknown
    }
}

/// Static description of a dataset a connector exposes.
#[derive(Debug, Clone)]
pub struct DatasetSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub description: Option<&'static str>,
    /// Domain category — the primary browse dimension. Required (no default) so
    /// every dataset is categorized at compile time. See `hkgov_common::Category`.
    pub category: Category,
    /// Free-form cross-cutting tags. Empty slice when none apply.
    pub tags: &'static [&'static str],
    /// Declared update cadence — drives cadence-aware detectors (v7) and is
    /// surfaced as a filter on `/sources`. `Unknown` when not declared.
    pub cadence: Cadence,
    /// How often the ingest scheduler should refresh this dataset, seconds.
    pub refresh_interval_secs: u64,
}
