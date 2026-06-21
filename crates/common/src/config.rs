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
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            run_interval_secs: 6 * 3600,
            llm_base_url: String::new(),
            llm_api_key: None,
            llm_model: "gpt-4o-mini".to_string(),
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
