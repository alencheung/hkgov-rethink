//! Runtime configuration loaded from `config.toml` + environment overrides.
//!
//! Defaults are tuned for local single-node development. The fields that matter
//! for horizontal scaling (worker threads, connection pool, cache size) are all
//! exposed here so we never hardcode them — see docs/ARCHITECTURE.md for the
//! path from one node to a 100k-concurrency fleet.

use figment::providers::{Env, Format};
use figment::Figment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub api: ApiSettings,
    pub upstream: UpstreamSettings,
    pub cache: CacheSettings,
    pub store: StoreSettings,
    pub agent: AgentSettings,
    pub alerts: AlertSettings,
    pub log: LogSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    /// Bind address for the axum server.
    pub bind: String,
    /// Max concurrent in-flight requests before tower load-shedding kicks in.
    pub max_concurrency: usize,
    /// Per-request timeout, milliseconds.
    pub request_timeout_ms: u64,
    /// URL prefix for all API routes, e.g. `/v1`. Empty = no prefix.
    pub api_prefix: String,
    /// Optional API key. When set, every request must send it via the
    /// `X-API-Key` header (or `?api_key=` query). Empty = anonymous access.
    pub api_key: Option<String>,
    /// Per-IP request rate limit, requests/sec. 0 = unlimited.
    pub rate_per_sec: u32,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".to_string(),
            // Conservative for v1 single-node; raise / shard for the 100k target.
            max_concurrency: 50_000,
            request_timeout_ms: 15_000,
            api_prefix: "/v1".to_string(),
            api_key: None,
            rate_per_sec: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpstreamSettings {
    pub hkma_base_url: String,
    pub hkma_api_key: Option<String>,
    pub hkma_timeout_ms: u64,
    pub hkma_max_retries: u32,
    /// Tuned to be polite to a free public endpoint.
    pub hkma_rate_per_sec: u32,

    pub data_gov_hk_filter_url: String,
    pub data_gov_hk_archive_url: String,
}

impl Default for UpstreamSettings {
    fn default() -> Self {
        Self {
            // Verified live — see docs/DATA_SOURCES.md.
            hkma_base_url: "https://api.hkma.gov.hk/public".to_string(),
            hkma_api_key: None,
            hkma_timeout_ms: 10_000,
            hkma_max_retries: 3,
            hkma_rate_per_sec: 5,
            data_gov_hk_filter_url: "https://api.data.gov.hk/v2/filter".to_string(),
            data_gov_hk_archive_url: "https://app.data.gov.hk/v1/historical-archive/list-files"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSettings {
    /// Max entries the in-process moka cache will hold.
    pub max_entries: u64,
    /// TTL for cached records, seconds.
    pub ttl_secs: u64,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            max_entries: 200_000,
            ttl_secs: 600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogSettings {
    /// `json` or `plain`.
    pub format: String,
    /// RUSTON_LOG-style filter, e.g. `info,hkgov=debug`.
    pub filter: String,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            format: "plain".to_string(),
            filter: "info".to_string(),
        }
    }
}

/// Which backing store the API uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreSettings {
    /// `memory` (in-process moka) or `redis` (shared cluster). The redis
    /// choice requires the `redis` cargo feature at build time.
    pub backend: String,
    /// Redis URL, e.g. `redis://127.0.0.1:6379`. Ignored unless backend=redis.
    pub redis_url: String,
}

impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            backend: "memory".to_string(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
        }
    }
}

/// AI-agent layer knobs (ROADMAP v3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Whether the background agent scheduler runs. Off by default.
    pub enabled: bool,
    /// How often the agent re-runs its analysis passes, seconds.
    pub run_interval_secs: u64,
    /// LLM provider base URL. Empty = local heuristic mode (no network calls),
    /// used so the agent layer works without API keys in dev/CI.
    pub llm_base_url: String,
    /// LLM API key (optional). Read from env `HKGOV_AGENT__LLM_API_KEY`.
    pub llm_api_key: Option<String>,
    /// Model id to request.
    pub llm_model: String,
    /// What the periodic scan pass should run. Each entry maps to one detector
    /// call against one (source, dataset, field). An empty list means "run the
    /// [`default_scan_targets`] set" so out-of-the-box behavior is unchanged.
    pub scan: Vec<ScanTarget>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            run_interval_secs: 6 * 3600,
            llm_base_url: String::new(),
            llm_api_key: None,
            llm_model: "gpt-4o-mini".to_string(),
            scan: default_scan_targets(),
        }
    }
}

/// The update frequency of a series. Determines what "a normal move" means —
/// a 5% monthly move is large; a 5% annual move is noise. Detectors use this to
/// scale thresholds cadence-relatively instead of applying a flat % everywhere.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Cadence {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Biannual,
    Annual,
    /// Cadence unknown / not declared. Detectors treat this as "no scaling" —
    /// i.e. use the raw threshold. The safe default for backwards compat.
    #[default]
    Unknown,
}

impl Cadence {
    /// Typical number of reporting periods per year for this cadence. Used to
    /// convert a flat % threshold into a per-period threshold. `Unknown` → 1
    /// (no scaling). Daily/weekly are capped to avoid pathological scaling.
    pub fn periods_per_year(&self) -> f64 {
        match self {
            Cadence::Daily => 252.0,
            Cadence::Weekly => 52.0,
            Cadence::Monthly => 12.0,
            Cadence::Quarterly => 4.0,
            Cadence::Biannual => 2.0,
            Cadence::Annual => 1.0,
            Cadence::Unknown => 1.0,
        }
    }
}

/// What a cadence-aware detector compares each period against. The choice
/// determines which kinds of lie/misalignment a scan catches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    /// Consecutive periods (N vs N-1). The original `series_jump` semantics.
    /// Right for daily/monthly; misleading for seasonal quarterly/annual series.
    #[default]
    PeriodOverPeriod,
    /// Same period a year ago (N vs N-from-a-year-ago). Removes seasonality —
    /// the correct comparison for quarterly retail, tourism, fiscal lines.
    YearOverYear,
}

/// One detector invocation in the periodic scan. Replaces the hardcoded
/// `run_pass` targets in v3 — the same targets now live in config so operators
/// can widen coverage without code changes.
///
/// `detector` is one of: `series_jump`, `year_over_year`, `outlier`,
/// `seasonality`, `correlation`, `cross_source_gap`, `proxy_divergence`,
/// `benchmark_deviation`. Field/threshold semantics depend on the detector
/// (see `crates/agent/src/analysis.rs`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScanTarget {
    /// Source slug, e.g. `hkma`, `datagovhk`, `press`, `landsd`.
    pub source: String,
    /// Dataset id within the source.
    pub dataset: String,
    /// Detector name. See the module doc on `crates/agent/src/analysis.rs`.
    pub detector: String,
    /// Numeric field the detector reads. Required for `series_jump`/`outlier`/
    /// `seasonality`/`correlation`; ignored by `cross_source_gap`.
    #[serde(default)]
    pub field: Option<String>,
    /// Detector-specific threshold (e.g. % move for `series_jump`, |z| for
    /// `outlier`). When `None`, each detector applies its own documented default.
    #[serde(default)]
    pub threshold: Option<f64>,
    /// For `correlation`: the second field to correlate against `field`.
    /// Ignored by other detectors.
    #[serde(default)]
    pub field_b: Option<String>,
    /// For `cross_source_gap`/`proxy_divergence`/`benchmark_deviation`: the
    /// companion (source, dataset) forming the second side of the comparison.
    /// Ignored by other detectors.
    #[serde(default)]
    pub companion: Option<CompanionRef>,
    /// Update cadence of this series. When set (not `unknown`), cadence-aware
    /// detectors (`series_jump`, `year_over_year`) scale their thresholds so a
    /// normal-sized move for the cadence isn't flagged.
    #[serde(default)]
    pub cadence: Cadence,
    /// For `series_jump`/`year_over_year`: what each period is compared against.
    /// Defaults to `period_over_period` (the v3 behavior). Use `year_over_year`
    /// for seasonal quarterly/annual series to remove seasonality from the delta.
    #[serde(default)]
    pub comparison: Comparison,
    /// For `proxy_divergence`: the field to read on the companion dataset.
    /// Required for `proxy_divergence`; ignored otherwise.
    #[serde(default)]
    pub companion_field: Option<String>,
    /// For `proxy_divergence`: the key field shared by both datasets (e.g.
    /// `date`, `district`, `quarter`). The two series are joined on it.
    /// Defaults to `record_id`.
    #[serde(default)]
    pub join_field: Option<String>,
    /// Mark this target as experimental. Experimental detectors (`seasonality`,
    /// `correlation`) are correct by construction but haven't yet produced a
    /// standout finding on real HKGOV data — see EXAMPLES.md. Setting this only
    /// affects logging (an `experimental=true` field on the insight-scan log
    /// line); it does NOT change detection. Defaults to `false`.
    #[serde(default)]
    pub experimental: bool,
}

/// The "other side" of a `cross_source_gap` comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompanionRef {
    pub source: String,
    pub dataset: String,
}

/// The out-of-the-box scan set. Kept as a fn (not a `const`) because it allocates;
/// this is exactly the set the old hardcoded `run_pass` ran.
///
/// **Note:** the v6 detectors (`outlier`, `seasonality`, `correlation`) are NOT
/// in the defaults. `outlier` is validated (see EXAMPLES.md); `seasonality` and
/// `correlation` are experimental. Operators opt in by adding `[[agent.scan]]`
/// entries — see config.toml for examples.
pub fn default_scan_targets() -> Vec<ScanTarget> {
    vec![
        ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "series_jump".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(25.0),
            field_b: None,
            companion: None,
            cadence: Cadence::Daily,
            comparison: Comparison::PeriodOverPeriod,
            companion_field: None,
            join_field: None,
            experimental: false,
        },
        ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "series_jump".into(),
            field: Some("closing_balance".into()),
            threshold: Some(15.0),
            field_b: None,
            companion: None,
            cadence: Cadence::Daily,
            comparison: Comparison::PeriodOverPeriod,
            companion_field: None,
            join_field: None,
            experimental: false,
        },
        ScanTarget {
            source: "hkma".into(),
            dataset: "capital-market-statistics".into(),
            detector: "series_jump".into(),
            field: Some("eq_mkt_hs_index".into()),
            threshold: Some(10.0),
            field_b: None,
            companion: None,
            cadence: Cadence::Monthly,
            comparison: Comparison::PeriodOverPeriod,
            companion_field: None,
            join_field: None,
            experimental: false,
        },
        ScanTarget {
            source: "press".into(),
            dataset: "hkma-press-releases".into(),
            detector: "cross_source_gap".into(),
            field: Some("date".into()),
            threshold: None,
            field_b: None,
            companion: Some(CompanionRef {
                source: "hkma".into(),
                dataset: "daily-interbank-liquidity".into(),
            }),
            cadence: Cadence::Unknown,
            comparison: Comparison::PeriodOverPeriod,
            companion_field: None,
            join_field: None,
            experimental: false,
        },
    ]
}

/// Proactive alerting knobs (v6). Off by default — enabling requires the
/// `alerts` cargo feature at build time. When on, the agent supervisor pushes
/// insights at or above `min_severity` to every configured webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertSettings {
    /// Whether alert dispatch is active. Off by default.
    pub enabled: bool,
    /// Minimum severity that triggers a dispatch: `info`, `warning`, or
    /// `critical`. Default `warning` (so info-level insights don't spam).
    pub min_severity: String,
    /// Webhook URLs to POST each qualifying insight to (JSON body).
    pub webhooks: Vec<String>,
    /// Bearer token sent as `Authorization` on each webhook POST, if set.
    pub webhook_token: Option<String>,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_severity: "warning".into(),
            webhooks: Vec::new(),
            webhook_token: None,
        }
    }
}

impl Settings {
    /// Load settings. Order (later wins): defaults < config.toml < env.
    ///
    /// Env vars are flattened with a `HKGOV_` prefix and `__` as the separator,
    /// e.g. `HKGOV_API__BIND=0.0.0.0:9090`.
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        Figment::from(figment::providers::Serialized::defaults(Settings::default()))
            .merge(figment::providers::Toml::file("config.toml"))
            .merge(Env::prefixed("HKGOV_").split("__"))
            .extract()
    }
}
