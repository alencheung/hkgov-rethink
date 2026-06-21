//! Connector registry — the single place ingest asks "who serves this source?".
//!
//! v1 only registers the HKMA connector. The trait + registry shape is here so
//! adding `DataGovHkConnector`, `PressConnector`, `LandsDConnector` later is a
//! one-line append, not a refactor.

use crate::{Connector, DatasetSpec};
use hkgov_common::{DataSource, Result, Settings};
use std::sync::Arc;

/// All live connectors, keyed by source.
pub struct Registry {
    by_source: Vec<(DataSource, Arc<dyn Connector>)>,
}

impl Registry {
    /// Build the registry from settings. Only sources whose connectors are
    /// implemented get registered.
    pub fn build(settings: &Settings) -> Result<Self> {
        let mut by_source: Vec<(DataSource, Arc<dyn Connector>)> = Vec::new();

        let hkma = crate::hkma::HkmaConnector::new(&settings.upstream)?;
        by_source.push((DataSource::Hkma, Arc::new(hkma)));

        // Future: DataGovHkConnector, PressConnector, LandsDConnector land here.

        Ok(Self { by_source })
    }

    pub fn lookup(&self, source: DataSource) -> Option<Arc<dyn Connector>> {
        self.by_source
            .iter()
            .find(|(s, _)| *s == source)
            .map(|(_, c)| c.clone())
    }

    /// Every (source, dataset) we currently expose — feeds `/sources`.
    pub fn all_datasets(&self) -> Vec<(DataSource, &DatasetSpec)> {
        self.by_source
            .iter()
            .flat_map(|(s, c)| c.datasets().iter().map(move |d| (*s, d)))
            .collect()
    }

    pub fn sources(&self) -> Vec<DataSource> {
        self.by_source.iter().map(|(s, _)| *s).collect()
    }
}
