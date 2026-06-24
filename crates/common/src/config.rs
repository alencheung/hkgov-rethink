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
    /// Static reference data served from read-only endpoints (e.g. the curated
    /// "related market players" directory behind `/v1/market-players`). An empty
    /// list means "serve the [`default_market_players`] set" so out-of-the-box
    /// behavior is unchanged — the same empty-means-defaults contract as
    /// `agent.scan`.
    pub reference: ReferenceSettings,
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

    /// Parse a cadence slug (case-insensitive). Mirrors `DataSource::parse`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "quarterly" => Some(Self::Quarterly),
            "biannual" | "semiannual" => Some(Self::Biannual),
            "annual" | "yearly" => Some(Self::Annual),
            "unknown" | "" => Some(Self::Unknown),
            _ => None,
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
    /// For `threshold_crossing`: the direction to watch. `"above"` = fire when
    /// the value exceeds `threshold`; `"below"` = fire when it drops under.
    /// Any other value (or unset) defaults to `"above"`. Kept as a string (not
    /// the agent crate's `CrossDirection` enum) to avoid a common→agent dep.
    #[serde(default)]
    pub direction: Option<String>,
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
    // D-012: the HKMA catalog was widened to the full 151 datasets and the
    // legacy `daily-interbank-liquidity` slug was renamed to
    // `daily-figures-interbank-liquidity` (it is the same Daily Figures of
    // Interbank Liquidity feed, same `hibor_overnight`/`closing_balance`
    // fields). The three targets below previously pointed at the old slug, so
    // the HIBOR spike + balance-swing + cross-source-gap detectors silently
    // scanned an empty dataset and produced nothing — only the capital-market
    // series_jump target fired. Updating the slug restores the flagship
    // HIBOR detection surface (the project's headline feature) and the
    // cross_source_gap input to the Silence Index.
    vec![
        ScanTarget {
            source: "hkma".into(),
            dataset: "daily-figures-interbank-liquidity".into(),
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
            direction: None,
        },
        ScanTarget {
            source: "hkma".into(),
            dataset: "daily-figures-interbank-liquidity".into(),
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
            direction: None,
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
            direction: None,
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
                dataset: "daily-figures-interbank-liquidity".into(),
            }),
            cadence: Cadence::Unknown,
            comparison: Comparison::PeriodOverPeriod,
            companion_field: None,
            join_field: None,
            experimental: false,
            direction: None,
        },
    ]
}

/// Curated reference data served from read-only endpoints. Mirrors the
/// `agent.scan` shape: a `Vec` of structured records loaded from
/// `[[reference.*]]` TOML tables, with a shipped default set so the platform
/// works with zero config and operators override by editing `config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReferenceSettings {
    /// The per-department "related market players" directory surfaced on the
    /// Licences page. Empty = serve [`default_market_players`].
    pub market_players: Vec<MarketPlayerGroup>,
}

/// A ranked set of private-sector players for one license-issuing department.
///
/// `dept` is a short stable code (e.g. `"HKMA"`, `"IA"`, `"SFC"`) that joins to
/// the dashboard's `LIC_DEPARTMENTS` entries — not the human-readable
/// department name, so the join is robust to copy edits. `category` tags the
/// business stream so the same data can be sliced by domain elsewhere.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketPlayerGroup {
    /// Stable department code joining to the dashboard directory (e.g. `HKMA`).
    pub dept: String,
    /// Business stream the players operate in (e.g. `monetary`, `livability`).
    #[serde(default)]
    pub category: crate::model::Category,
    /// The ranked players, ordered most-significant first.
    pub players: Vec<PlayerEntry>,
}

/// One market player: name + a short evidence-backed note (+ optional link).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerEntry {
    /// The player's common name (e.g. `HSBC`, `Sun Hung Kai Properties`).
    pub name: String,
    /// One-line note — why they rank, with a figure where possible
    /// (e.g. `Largest HK bank, ~HK$10.5T assets`).
    pub note: String,
    /// Optional deep link (company site, register entry, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The out-of-the-box market-players directory. Covers the seven license-issuing
/// departments that have a clear, nameable private market (banking, insurance,
/// securities, telecoms, transport, travel, food & beverage). Departments with
/// no natural named market (EPD, FSD, HKPF, EMSD, AFCD, LCSD, SWD, C&ED, CR,
/// IRD, LD, MD, LLB) ship no group — the dashboard renders no panel for them.
///
/// Rankings are directional, compiled from 2024–2025 public sources (KPMG HK
/// Banking Report, Insurance Authority stats, HKTDC research, HSI filings).
/// Operators can override or extend any group via `[[reference.market_player]]`
/// in `config.toml`.
pub fn default_market_players() -> Vec<MarketPlayerGroup> {
    use crate::model::Category;
    vec![
        // ── HKMA: the three-tier banking system + SVF ──────────────────────────
        MarketPlayerGroup {
            dept: "HKMA".into(),
            category: Category::Monetary,
            players: vec![
                pe("HSBC", "Largest HK bank by assets, ~HK$10.5T (KPMG 2024)"),
                pe("Bank of China (Hong Kong)", "#2 by assets, ~HK$3.9T; RMB clearing bank"),
                pe("Hang Seng Bank", "#3 by assets, ~HK$1.8T; HSBC-group domestic leader"),
                pe("Standard Chartered (HK)", "Major note-issuing bank"),
                pe("ICBC (Asia)", "Mainland-backed universal bank"),
                pe("DBS Bank (HK)", "Leading foreign bank"),
                pe("Citibank (HK)", "Major retail + corporate bank"),
                pe("Bank of East Asia", "Largest locally-rooted bank"),
                pe("Virtual banks (ZA Bank, Mox, livi)", "Eight licensed virtual banks"),
                pe("Octopus Cards", "Leading SVF licensee"),
            ],
        },
        // ── Insurance Authority: long-term + general insurance ─────────────────
        MarketPlayerGroup {
            dept: "IA".into(),
            category: Category::Monetary,
            players: vec![
                pe("AIA Group", "#1 life insurer, HK$87.1B premium (2025)"),
                pe("Prudential plc", "#2 life insurer, HK$65.3B premium"),
                pe("HSBC Life (International)", "#3 by premium"),
                pe("Manulife (International)", "Top-tier life insurer"),
                pe("China Life Insurance (Overseas)", "Major mainland-backed insurer"),
                pe("AXA Hong Kong & Macau", "Leading general + life insurer"),
                pe("FWD Group", "Fast-growing pan-Asian insurer"),
                pe("BOC Group Life Assurance", "Mainland-backed life insurer"),
                pe("Chubb (HK)", "Leading general insurer"),
                pe("AIG Hong Kong", "Major general insurer"),
            ],
        },
        // ── SFC: licensed corporations (Type 1–9 regulated activities) ─────────
        MarketPlayerGroup {
            dept: "SFC".into(),
            category: Category::Fiscal,
            players: vec![
                pe("HSBC Global Asset Management", "Largest AUM base in HK"),
                pe("AIA Investments", "Major insurance-linked asset manager"),
                pe("Hang Seng Investment Management", "Index + fund leader"),
                pe("Value Partners", "Largest home-grown asset manager"),
                pe("BOCI International / CCB International", "Mainland-backed IBs"),
                pe("Goldman Sachs Asia", "Leading global IB"),
                pe("Morgan Stanley Asia", "Major global IB"),
                pe("UBS AG Hong Kong", "Wealth management leader"),
                pe("Futu Securities / Tiger Brokers", "Retail brokerage disruptors"),
                pe("China Cheng Xin (Asia)", "Leading credit-rating agency"),
            ],
        },
        // ── OFCA: telecoms + broadcasting carriers ─────────────────────────────
        MarketPlayerGroup {
            dept: "OFCA".into(),
            category: Category::Livability,
            players: vec![
                pe("PCCW / HKT", "Largest telecom operator, ~US$5.2B revenue"),
                pe("Hutchison Telecom HK ('3')", "Major mobile operator (CK Hutchison)"),
                pe("SmarTone Telecommunications", "Major mobile operator (HKBN)"),
                pe("HKBN", "Major broadband + enterprise telecom"),
                pe("China Mobile Hong Kong", "Mainland-backed mobile operator"),
                pe("CITIC Telecom International", "~US$1.2B revenue; carrier services"),
                pe("Now TV (PCCW Media)", "Leading pay-TV operator"),
                pe("Television Broadcasts (TVB)", "Free-to-air leader"),
                pe("HK01 / Orange News", "Top digital news operator"),
                pe("Hong Kong Cable Television", "Pay-TV + broadband"),
            ],
        },
        // ── Transport Dept: vehicle / PSV / operator licensing ─────────────────
        MarketPlayerGroup {
            dept: "TD".into(),
            category: Category::Livability,
            players: vec![
                pe("MTR Corporation", "Rail operator + property; HSI constituent"),
                pe("Kowloon Motor Bus (KMB)", "Largest franchised bus operator"),
                pe("Citybus / Cityflyer", "Franchised bus operator (HK Island)"),
                pe("Long Win Bus", "Franchised bus operator (N.T./Lantau)"),
                pe("New World First Bus", "Franchised bus operator"),
                pe("New Lantao Bus", "Lantau franchised operator"),
                pe("Hong Kong Tramways", "Tram operator (also a data provider here)"),
                pe("Peak Tramways", "Tourist tram operator"),
                pe("Star Ferry", "Iconic harbour ferry operator"),
                pe("Taxi fleets (urban / NT / Lantau)", "Franchised PSV permit holders"),
            ],
        },
        // ── Travel Industry Authority: agents / guides / escorts ───────────────
        MarketPlayerGroup {
            dept: "TIA".into(),
            category: Category::Trade,
            players: vec![
                pe("China Travel Service (HK)", "Largest outbound/inbound operator"),
                pe("Wing On Travel", "Major outbound operator"),
                pe("Hong Thai Travel Services", "Major outbound operator"),
                pe("EGL Tours", "Leading package-tour operator"),
                pe("Morning Star Travel", "Major inbound + outbound"),
                pe("Ying Tung Travel", "Outbound specialist"),
                pe("Sunflower Travel", "Package-tour operator"),
                pe("Priceline / Booking HK", "Online travel platform"),
                pe("Trip.com (HK)", "Leading online travel platform"),
                pe("Klook (HK)", "Tours & experiences platform"),
            ],
        },
        // ── FEHD: food business / restaurant licensing ─────────────────────────
        MarketPlayerGroup {
            dept: "FEHD".into(),
            category: Category::Population,
            players: vec![
                pe("Maxim's Caterers", "Largest F&B operator (HK)"),
                pe("Café de Coral", "Largest QSR chain"),
                pe("Everyone's 25 (Fairwood)", "Major QSR chain"),
                pe("McDonald's Hong Kong", "Leading global QSR"),
                pe("Starbucks Hong Kong (Maxim's JV)", "Leading coffee chain"),
                pe("Pacific Coffee", "Major coffee chain"),
                pe("Yoshinoya Hong Kong", "Leading Japanese QSR"),
                pe("Sushi Revolution / Genki Sushi", "Major Japanese chains"),
                pe("Habitat (Tsui Wah Group)", "Cha chaan teng chain"),
                pe("The Chairman / Ho Lee Fook", "Acclaimed independent restaurants"),
            ],
        },
    ]
}

/// Shorthand constructor for a [`PlayerEntry`] without a URL — keeps the
/// `default_market_players` table readable above.
fn pe(name: &str, note: &str) -> PlayerEntry {
    PlayerEntry {
        name: name.into(),
        note: note.into(),
        url: None,
    }
}

/// Proactive alerting knobs (v6, email added v9). Off by default — enabling
/// requires the `alerts` cargo feature at build time. When on, the agent
/// supervisor pushes insights at or above `min_severity` to every configured
/// sink (webhooks + optional email).
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
    /// Email gateway config — the universal push channel. All fields required
    /// when any is set; the dispatcher builds one EmailSink from them.
    pub email_api_url: Option<String>,
    pub email_api_token: Option<String>,
    pub email_to: Option<String>,
    pub email_from: Option<String>,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_severity: "warning".into(),
            webhooks: Vec::new(),
            webhook_token: None,
            email_api_url: None,
            email_api_token: None,
            email_to: None,
            email_from: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- D-012: default scan targets must reference LIVE datasets ------------
    //
    // When the HKMA catalog was widened to 151 datasets, the legacy
    // `daily-interbank-liquidity` slug was renamed to
    // `daily-figures-interbank-liquidity` but the default scan targets kept
    // pointing at the old name. Three of four flagship detectors silently
    // scanned an empty dataset and produced nothing — only the capital-market
    // target fired, so the project's headline HIBOR detection was dead. These
    // guards lock the slug against any future rename-driven regression and
    // assert the field/cadence contract each detector needs.

    #[test]
    fn default_scan_targets_reference_live_datasets() {
        let scan = default_scan_targets();
        assert!(!scan.is_empty(), "defaults must be non-empty");

        // No target may point at the dead legacy slug.
        for t in &scan {
            assert_ne!(
                t.dataset, "daily-interbank-liquidity",
                "D-012 regression: scan target still references the dead slug"
            );
            if let Some(c) = &t.companion {
                assert_ne!(
                    c.dataset, "daily-interbank-liquidity",
                    "D-012 regression: companion still references the dead slug"
                );
            }
        }
    }

    #[test]
    fn default_scan_targets_cover_hibor_detection() {
        // The project's flagship: HIBOR series_jump on the live daily feed.
        let scan = default_scan_targets();
        let has_hibor = scan.iter().any(|t| {
            t.dataset == "daily-figures-interbank-liquidity"
                && t.detector == "series_jump"
                && t.field.as_deref() == Some("hibor_overnight")
        });
        assert!(
            has_hibor,
            "defaults must include hibor_overnight series_jump on the live feed"
        );
    }

    #[test]
    fn default_scan_targets_include_cross_source_gap_companion() {
        // The Silence Index is built from cross_source_gap; its companion must
        // be the live interbank dataset (whose record_ids are dates).
        let scan = default_scan_targets();
        let gap = scan
            .iter()
            .find(|t| t.detector == "cross_source_gap")
            .expect("defaults must include a cross_source_gap target");
        let companion = gap
            .companion
            .as_ref()
            .expect("cross_source_gap needs a companion");
        assert_eq!(companion.source, "hkma");
        assert_eq!(
            companion.dataset, "daily-figures-interbank-liquidity",
            "cross_source_gap companion must be the live (date-keyed) dataset"
        );
    }
}
