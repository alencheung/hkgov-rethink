//! Shared primitives for the hkgov-rethink platform.
//!
//! Everything that more than one crate needs to agree on (config, the normalized
//! [`Record`] shape, the [`DataSource`] enum, observability bootstrap, and error
//! types) lives here so connectors, the ingest pipeline, and the serving API
//! never drift apart.

pub mod config;
pub mod error;
pub mod model;
pub mod telemetry;

pub use config::{
    default_scan_targets, AgentSettings, AlertSettings, ApiSettings, CacheSettings, Cadence,
    CompanionRef, Comparison, LogSettings, ScanTarget, Settings, StoreSettings, UpstreamSettings,
};
pub use error::{Error, Result};
pub use model::{Category, DataSource, DatasetMeta, NormalizedRecord, RecordValue};
