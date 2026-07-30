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
/// Version-stable synthetic record-id derivation (shared FNV-1a). Used by any
/// connector whose upstream exposes no natural primary key, so persisted ids
/// stay stable across Rust/compiler versions.
pub mod ids;
pub mod immigration;
pub mod landregistry;
pub mod landsd;
/// Size-limited response reads — bounds peak memory regardless of upstream
/// behavior (PERF-CON-01). Every connector should read bodies through here.
pub mod limited;
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

/// Per-hop deadline for a single Worker-proxy round-trip. Sized to sit *on top
/// of* the reqwest client's own timeout so a hung upstream can't pin a Worker
/// connection indefinitely (RES-CON-01).
const WORKER_HOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Reject `upstream_url` values that are not public http(s) endpoints. This is
/// client-side defense-in-depth against SSRF: the Worker enforces a host
/// allow-list too, but validating here means a future config-driven or
/// user-influenced URL can never turn the proxy hop into an internal relay
/// (SEC-CON-02). Blocks loopback, private (RFC1918), and link-local addresses.
pub(crate) fn assert_public_upstream(upstream_url: &str) -> Result<()> {
    let parsed = url::Url::parse(upstream_url)
        .map_err(|e| Error::BadRequest(format!("invalid upstream url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(Error::BadRequest(format!(
                "upstream url must be http(s), got {other}"
            )))
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::BadRequest("upstream url has no host".into()))?;
    // IP-literal hosts get a direct address check; named hosts are left to DNS
    // resolution (the Worker allow-list is the hard boundary for names).
    if let Some(ip) = parsed.host().and_then(|h| match h {
        url::Host::Ipv4(v4) => Some(std::net::IpAddr::V4(v4)),
        url::Host::Ipv6(v6) => Some(std::net::IpAddr::V6(v6)),
        _ => None,
    }) {
        if ip.is_loopback() || ip.is_unspecified() || is_private_or_link_local(&ip) {
            return Err(Error::BadRequest(format!(
                "refusing non-public upstream host {host}"
            )));
        }
    }
    Ok(())
}

/// True for RFC1918 / shared / link-local IPv4, and unique-local / link-local
/// IPv6. (std doesn't expose `is_private`/`is_link_local` on both families
/// uniformly across versions, so check explicitly.)
fn is_private_or_link_local(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_link_local() || v4.is_broadcast() || v4.is_documentation()
        }
        std::net::IpAddr::V6(v6) => {
            // fc00::/7 unique-local, fe80::/10 link-local.
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
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
/// `extra_headers` carries optional per-host request headers to forward to the
/// upstream target. These travel as the value of a single `X-Upstream-Auth`
/// request header on the Worker hop — **never** as query parameters. Placing
/// bearer JWTs (Midland's `BUILD_TOKEN`, HKP's `userToken`) in the query string
/// leaks them into edge/access logs, browser history, and the `Referer` header
/// (SEC-CON-01); headers do not. The Worker reads `X-Upstream-Auth` and
/// forwards it upstream as `Authorization`.
///
/// Only the first `extra_headers` entry is forwarded (callers today pass exactly
/// one `Authorization` value); a second entry is a programmer error and is
/// ignored with a debug log.
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
    // Defense-in-depth against SSRF: reject non-http(s) schemes and non-public
    // (loopback / private / link-local) hosts before handing the URL to the
    // Worker. The Worker ALSO enforces an allow-list, but validating client-side
    // means a future caller-config-driven URL can never turn this into an open
    // proxy (SEC-CON-02).
    assert_public_upstream(upstream_url)?;

    // Build the Worker URL: <proxy>/fetch?url=<encoded>. No secrets in the
    // query string — auth travels in the `X-Upstream-Auth` header below.
    let worker_url = format!(
        "{}/fetch?url={}",
        proxy_url.trim_end_matches('/'),
        url::form_urlencoded::byte_serialize(upstream_url.as_bytes()).collect::<String>()
    );
    // The first extra header is the Authorization value for the upstream target.
    // We forward it via X-Upstream-Auth so it stays out of logs.
    let upstream_auth = extra_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.to_string());
    let mut req = client
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
        );
    if let Some(auth) = upstream_auth {
        req = req.header("X-Upstream-Auth", auth);
    }
    // Per-hop deadline on top of the client's overall timeout: a hung upstream
    // must not pin a Worker connection indefinitely (RES-CON-01).
    let resp = tokio::time::timeout(WORKER_HOP_TIMEOUT, req.send())
        .await
        .map_err(|_| Error::Upstream {
            origin: "worker",
            status: 0,
            detail: "worker hop timed out".into(),
        })?
        .map_err(|e| Error::Upstream {
            origin: "worker",
            status: 0,
            detail: format!("transport: {e}"),
        })?;
    let status = resp.status().as_u16();
    if status >= 400 {
        // Worker itself returned an error (e.g. 403 host-not-allowed, 502
        // upstream network error). Map it cleanly. Cap the error body so a
        // pathological Worker error page can't OOM us.
        let body =
            crate::limited::read_text_limited(resp, "worker", crate::limited::MAX_ERROR_BYTES)
                .await
                .unwrap_or_default();
        return Err(Error::Upstream {
            origin: "worker",
            status,
            detail: body,
        });
    }
    // Cap the Worker envelope body — it carries the full upstream payload, so
    // without a cap a runaway upstream would OOM the process here (PERF-CON-01).
    let env_json =
        crate::limited::read_text_limited(resp, "worker", crate::limited::MAX_DATA_BYTES).await?;
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
