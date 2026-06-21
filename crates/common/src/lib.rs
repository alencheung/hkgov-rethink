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
    AgentSettings, ApiSettings, CacheSettings, LogSettings, Settings, StoreSettings,
    UpstreamSettings,
};
pub use error::{Error, Result};
pub use model::{DataSource, DatasetMeta, NormalizedRecord, RecordValue};
