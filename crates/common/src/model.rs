//! Normalized data model.
//!
//! HKGOV data sources disagree wildly about shapes: HKMA returns
//! `{header, result:{records:[{...}]}}`, data.gov.hk filter returns a bare JSON
//! array, press releases are HTML/RSS. We collapse them onto one shape so the
//! store, the API, and (later) the AI-agent layer only ever speak one dialect.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::config::Cadence;

/// The set of upstream sources we ingest from. New connectors extend this enum.
///
/// Note: `LandsD` covers the *public* open map tile APIs hosted on data.gov.hk
/// and the CSDI portal. The gov-only `api.portal.hkmapservice.gov.hk` is
/// intentionally excluded — see docs/DATA_SOURCES.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataSource {
    Hkma,
    DataGovHk,
    Press,
    LandsD,
}

impl DataSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataSource::Hkma => "hkma",
            DataSource::DataGovHk => "datagovhk",
            DataSource::Press => "press",
            DataSource::LandsD => "landsd",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "hkma" => Some(Self::Hkma),
            "datagovhk" | "data.gov.hk" => Some(Self::DataGovHk),
            "press" | "isd" | "info.gov.hk" => Some(Self::Press),
            "landsd" | "csdi" => Some(Self::LandsD),
            _ => None,
        }
    }
}

impl std::fmt::Display for DataSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a single cell of an upstream record can be after normalization.
///
/// HKGOV data is sparse and heterogeneous (mixes ints, floats, `null`, and
/// strings freely), so we model each value as an explicit enum rather than
/// forcing everything into a string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RecordValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl RecordValue {
    pub fn is_null(&self) -> bool {
        matches!(self, RecordValue::Null)
    }
}

/// A single normalized row from any source. Keys are the original field names
/// from upstream; the [`NormalizedRecord::source`] + [`NormalizedRecord::dataset`]
/// pair is the cache key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedRecord {
    pub source: DataSource,
    pub dataset: String,
    /// Stable identifier for the record within the dataset. For HKMA this is
    /// the period field (e.g. `2026-05`); for press it is the release id.
    pub record_id: String,
    pub fields: BTreeMap<String, RecordValue>,
    pub fetched_at: DateTime<Utc>,
}

/// The domain taxonomy for a dataset. A fixed, browseable vocabulary aligned
/// to how people interested in Hong Kong think about city data — monetary
/// policy vs. fiscal vs. property vs. livability. New datasets declare exactly
/// one category at compile time (it's a required field on `DatasetSpec`), so
/// the taxonomy stays complete rather than aspirational.
///
/// Free-form cross-cutting concerns (e.g. `hibor`, `interest-rate`) go in the
/// `tags` field instead — see [`DatasetMeta::tags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    /// HIBOR, monetary base, exchange fund, discount window, interbank liquidity.
    Monetary,
    /// Budget, revenue, expenditure, land premium, government bonds.
    Fiscal,
    /// Land sales, transactions, price indices, housing pipeline, vacancy.
    Property,
    /// Imports/exports, balance of payments, merchandise trade.
    Trade,
    /// Residents, births, migration, employment, labour force.
    Population,
    /// Air quality, transport ridership, visitor arrivals — quality-of-city signals.
    Livability,
    /// Press releases, policy addresses, consultations, tenders, legislation.
    Government,
    /// Anything that doesn't fit the above. The explicit default so a new dataset
    /// that forgets to categorize still compiles — but should be re-categorized.
    #[default]
    Other,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Monetary => "monetary",
            Category::Fiscal => "fiscal",
            Category::Property => "property",
            Category::Trade => "trade",
            Category::Population => "population",
            Category::Livability => "livability",
            Category::Government => "government",
            Category::Other => "other",
        }
    }

    /// Parse a category slug (case-insensitive). Mirrors `DataSource::parse`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "monetary" => Some(Self::Monetary),
            "fiscal" => Some(Self::Fiscal),
            "property" => Some(Self::Property),
            "trade" => Some(Self::Trade),
            "population" => Some(Self::Population),
            "livability" => Some(Self::Livability),
            "government" => Some(Self::Government),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata about a dataset we serve. Returned by `/sources` and
/// `/datasets/:id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMeta {
    pub source: DataSource,
    pub dataset: String,
    pub title: String,
    pub description: Option<String>,
    /// Domain category — the primary browse dimension. See [`Category`].
    #[serde(default = "default_category")]
    pub category: Category,
    /// Free-form cross-cutting tags (e.g. `hibor`, `interest-rate`). Empty for
    /// datasets that haven't declared any.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Declared update cadence. `Unknown` when the dataset hasn't declared one.
    #[serde(default)]
    pub cadence: Cadence,
    /// How often the cache for this dataset is refreshed.
    pub refresh_interval_secs: u64,
    pub last_refreshed_at: Option<DateTime<Utc>>,
    pub record_count: usize,
}

/// Serde default for the `category` field — `Other`. Used so older cached meta
/// blobs that predate the field still deserialize.
fn default_category() -> Category {
    Category::Other
}
