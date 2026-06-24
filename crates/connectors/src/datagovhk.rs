//! data.gov.hk connector.
//!
//! Targets the platform's *own* APIs (NOT CKAN — `data.gov.hk/api/3/action/*`
//! returns 404). Two endpoints are used:
//!
//! - **Filter API** (`api.data.gov.hk/v2/filter`): query a single resource by its
//!   CSV/JSON URL. Returns a bare JSON array of row objects.
//! - **Historical archive** (`app.data.gov.hk/v1/historical-archive/list-files`):
//!   list files for a provider between two dates.
//!
//! Both verified live — see docs/DATA_SOURCES.md.
//!
//! ## Resource coverage
//!
//! data.gov.hk publishes **376 datasets** across 17 providers, but the v2 filter
//! API only accepts a **registered subset** of PSI resource URLs — the rest are
//! rejected with `{"code":"422","message":"Not a valid resource"}`. Every entry
//! in `RESOURCES` below was probe-verified live against the filter API (returned
//! HTTP 200 with a non-empty row array). The historical-archive connector
//! (`landsd-catalog`) remains the discovery path for the full catalog; this
//! connector only registers resources that actually return queryable data.
//!
//! Adding a resource = adding a row here; the `Connector` impl derives
//! everything else. Each row's category is inferred from its provider:
//! - `hk-cr` → Fiscal (Companies Registry business statistics)
//! - `hk-dh` → Livability (Centre for Health Protection)
//! - `hk-csd` → Government (Correctional Services)
//! - `hk-ofca` → Government (telecoms licensing)
//! - `hk-edb` → Population (Education Bureau)
//! - `hktramways` / `hk-wsd` → Livability (transport / water)
//! - `centaline` → Property (price index)

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use std::collections::BTreeMap;
use std::time::Duration;

/// One verified data.gov.hk resource. Adding a dataset = adding a row here.
#[derive(Debug, Clone, Copy)]
struct DataGovResource {
    /// Dataset id (the slug used in `/v1/datasets/datagovhk/<slug>`).
    slug: &'static str,
    title: &'static str,
    /// data.gov.hk resource URL (CSV/JSON). Queried verbatim via the v2 filter
    /// API. Must be probe-verified — the filter API rejects unregistered URLs.
    resource_url: &'static str,
    category: Category,
    tags: &'static [&'static str],
    cadence: Cadence,
    /// Which field (if any) uniquely identifies a row. `None` → hash fallback.
    id_field: Option<&'static str>,
}

/// The verified data.gov.hk resource table. Every `resource_url` here was
/// confirmed to return data via `api.data.gov.hk/v2/filter` (see module docs).
const RESOURCES: &[DataGovResource] = &[
    // --- hk-cr (Companies Registry; 11 resources) ---
    DataGovResource {
        slug: "money-lenders-licensees",
        title: "Money Lenders Licensees (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/ml_licensees.csv",
        category: Category::Fiscal,
        tags: &["money-lenders", "licensing", "companies-registry"],
        cadence: Cadence::Daily,
        id_field: Some("MLR_No"),
    },
    DataGovResource {
        slug: "cr-prosecution-records",
        title: "Statistics Data on Prosecution (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/conviction_record.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "prosecution", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-non-compliance-change-name",
        title: "Non-Compliance with Directions to Change/Replace Company Names (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/replace_name.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Daily,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-companies-on-register",
        title: "Statistical Data on Local Companies Registered on the Companies Register",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_01.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-companies-incorporated",
        title: "Statistical Data on Local Companies Incorporated",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_02.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-non-hk-companies",
        title: "Statistical Data on Registered Non-Hong Kong Companies",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_03.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-dissolutions",
        title: "Statistical Data on Dissolution (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_04.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-liquidations",
        title: "Statistical Data on Liquidations (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_05.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-documents-delivered",
        title: "Statistical Data on Documents Delivered for Registration (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_06.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-charges-discharge",
        title: "Statistical Data on Charges/Memorandum of Discharge Delivered for Registration",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_07.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },
    DataGovResource {
        slug: "cr-stat-company-searches",
        title: "Statistics Data on Searches on Image Records of Documents (Companies Registry)",
        resource_url: "http://www.cr.gov.hk/datagovhk/psi/statistics_08.csv",
        category: Category::Fiscal,
        tags: &["companies-registry", "business-statistics"],
        cadence: Cadence::Monthly,
        id_field: None,
    },

    // --- hk-dh (Department of Health / Centre for Health Protection; 5 resources) ---
    DataGovResource {
        slug: "dh-covid-active-quarantine-orders",
        title: "Active Quarantine Orders (COVID-19, Cap. 599C) — Centre for Health Protection",
        resource_url: "http://www.chp.gov.hk/files/misc/active_quarantine_orders_cap599c.csv",
        category: Category::Livability,
        tags: &["health", "public-health", "covid-19"],
        cadence: Cadence::Daily,
        id_field: None,
    },
    DataGovResource {
        slug: "dh-gastroenteritis-viruses-2013",
        title: "Detection of Gastroenteritis Viruses from Faecal Specimens (2013)",
        resource_url: "http://www.chp.gov.hk/files/misc/detection_of_gastroenteritis_viruses_from_faecal_specimens_in_2013_en.csv",
        category: Category::Livability,
        tags: &["health", "public-health", "laboratory-surveillance"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "dh-respiratory-pathogens-2014",
        title: "Detection of Pathogens from Respiratory Specimens (2014)",
        resource_url: "http://www.chp.gov.hk/files/misc/detection_of_influenza_viruses_in_respiratory_specimens_in_2014_en.csv",
        category: Category::Livability,
        tags: &["health", "public-health", "laboratory-surveillance"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "dh-influenza-subtyping-2014",
        title: "Influenza Virus Subtyping (2014)",
        resource_url: "http://www.chp.gov.hk/files/misc/influenza_virus_subtyping_in_2014_en.csv",
        category: Category::Livability,
        tags: &["health", "public-health", "laboratory-surveillance"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "dh-dengue-fever-statistics",
        title: "Statistics on Dengue Fever — Centre for Health Protection",
        resource_url: "https://www.chp.gov.hk/files/misc/df2002_en.csv",
        category: Category::Livability,
        tags: &["health", "public-health", "infectious-disease"],
        cadence: Cadence::Unknown,
        id_field: None,
    },

    // --- hk-csd (Correctional Services Department; 6 resources) ---
    DataGovResource {
        slug: "csd-chronology-history",
        title: "Chronology of CSD's Development and Penal Measures of Hong Kong",
        resource_url: "https://www.csd.gov.hk/datagovhk/About_Us_History_EN.csv",
        category: Category::Government,
        tags: &["correctional-services", "statistics"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "csd-approved-hand-in-articles",
        title: "Approved Hand-in Articles (Correctional Services)",
        resource_url: "http://www.csd.gov.hk/datagovhk/Approved_Hand_in_articles_EN.csv",
        category: Category::Government,
        tags: &["correctional-services", "statistics"],
        cadence: Cadence::Daily,
        id_field: None,
    },
    DataGovResource {
        slug: "csd-institution-information",
        title: "Correctional Institutions Information",
        resource_url: "http://www.csd.gov.hk/datagovhk/PSI_Institution_information_EN.csv",
        category: Category::Government,
        tags: &["correctional-services", "statistics"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "csd-rehab-publicity-activities",
        title: "Rehabilitation Publicity Activities for Rehabilitated Persons",
        resource_url: "https://www.csd.gov.hk/datagovhk/Rehabilitation_Publicity_Activities_for_Rehabilitated_Persons_EN.csv",
        category: Category::Government,
        tags: &["correctional-services", "statistics"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "csd-escape-rate",
        title: "Escape Rate of Persons in Custody (Correctional Services)",
        resource_url: "https://www.csd.gov.hk/datagovhk/Stat_T1-11_escape_rate_en.csv",
        category: Category::Government,
        tags: &["correctional-services", "statistics"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "csd-ciu-complaints",
        title: "Complaints/Requests/Enquiries Received by Complaint Investigation Unit",
        resource_url: "https://www.csd.gov.hk/datagovhk/Stat_T1-13_ciu_en.csv",
        category: Category::Government,
        tags: &["correctional-services", "statistics"],
        cadence: Cadence::Unknown,
        id_field: None,
    },

    // --- hk-ofca (Office of the Communications Authority; 4 resources) ---
    DataGovResource {
        slug: "ofca-carrier-licensees",
        title: "List of Carrier Licensees (OFCA)",
        resource_url: "https://www.ofca.gov.hk/filemanager/ofca/common/datagovhk/carrier_lic_en.csv",
        category: Category::Government,
        tags: &["telecommunications", "licensing", "ofca"],
        cadence: Cadence::Daily,
        id_field: None,
    },
    DataGovResource {
        slug: "ofca-experimental-station-licensees",
        title: "List of Experimental Station Licensees (OFCA)",
        resource_url: "https://www.ofca.gov.hk/filemanager/ofca/common/datagovhk/ex_en.csv",
        category: Category::Government,
        tags: &["telecommunications", "licensing", "ofca"],
        cadence: Cadence::Daily,
        id_field: None,
    },
    DataGovResource {
        slug: "ofca-sbo-licensees",
        title: "List of Services-Based Operator (SBO) Licensees (OFCA)",
        resource_url: "https://www.ofca.gov.hk/filemanager/ofca/common/datagovhk/sbo_lic_en.csv",
        category: Category::Government,
        tags: &["telecommunications", "licensing", "ofca"],
        cadence: Cadence::Daily,
        id_field: None,
    },
    DataGovResource {
        slug: "ofca-radio-dealer-unrestricted-licensees",
        title: "List of Radio Dealer (Unrestricted) Licensees (OFCA)",
        resource_url: "https://www.ofca.gov.hk/filemanager/ofca/common/datagovhk/xru_en.csv",
        category: Category::Government,
        tags: &["telecommunications", "licensing", "ofca"],
        cadence: Cadence::Daily,
        id_field: None,
    },

    // --- hk-edb (Education Bureau; 3 resources) ---
    DataGovResource {
        slug: "edb-curriculum-development-council-members",
        title: "Membership of Curriculum Development Council (Education Bureau)",
        resource_url: "https://www.edb.gov.hk/attachment/datagovhk/Membership_of_Curriculum_Development_Council_en.csv",
        category: Category::Population,
        tags: &["education", "edb"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "edb-cross-boundary-students",
        title: "Cross-boundary Students in Kindergartens/Primary/Secondary Schools (Education Bureau)",
        resource_url: "https://www.edb.gov.hk/attachment/datagovhk/Number_of_cross_boundary_students_en.csv",
        category: Category::Population,
        tags: &["education", "edb", "cross-boundary"],
        cadence: Cadence::Annual,
        id_field: None,
    },
    DataGovResource {
        slug: "edb-cross-boundary-students-by-control-point",
        title: "Cross-boundary Students by Land Boundary Control Point and Class Level (Education Bureau)",
        resource_url: "https://www.edb.gov.hk/attachment/datagovhk/Number_of_cross_boundary_students_land_en.csv",
        category: Category::Population,
        tags: &["education", "edb", "cross-boundary"],
        cadence: Cadence::Annual,
        id_field: None,
    },

    // --- hktramways (2 resources) ---
    DataGovResource {
        slug: "tramways-main-routes",
        title: "Hong Kong Tramways Main Routes",
        resource_url: "http://static.data.gov.hk/tramways/datasets/main_routes/tramways_main_routes_en.csv",
        category: Category::Livability,
        tags: &["transport", "tram", "routes"],
        cadence: Cadence::Unknown,
        id_field: None,
    },
    DataGovResource {
        slug: "tramways-tram-stops",
        title: "Hong Kong Tramways Tram Stops",
        resource_url: "http://static.data.gov.hk/tramways/datasets/tram_stops/summary_tram_stops_en.csv",
        category: Category::Livability,
        tags: &["transport", "tram", "stops"],
        cadence: Cadence::Unknown,
        id_field: None,
    },

    // --- hk-wsd (Water Supplies Department; 1 resource) ---
    DataGovResource {
        slug: "wsd-annual-fresh-water-supply",
        title: "Annual Quantity of Fresh Water Supply (Water Supplies Department)",
        resource_url: "https://www.wsd.gov.hk/datagovhk/en-data/annual_quantity_of_fresh_water_supply_en.csv",
        category: Category::Livability,
        tags: &["water-supply", "wsd"],
        cadence: Cadence::Annual,
        id_field: None,
    },

    // --- centaline (1 resource) ---
    DataGovResource {
        slug: "centaline-cci-estates",
        title: "Property Information of the CCI Constituent Estates (Centaline)",
        resource_url: "http://hk.centanet.com/opendata/CCI%20Estate%20for%20Opendata.csv",
        category: Category::Property,
        tags: &["property", "price-index", "centaline"],
        cadence: Cadence::Daily,
        id_field: None,
    },
];

pub struct DataGovHkConnector {
    filter_url: String,
    archive_url: String,
    client: reqwest::Client,
}

impl DataGovHkConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(20_000))
            .gzip(true)
            .pool_max_idle_per_host(32)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            filter_url: settings.data_gov_hk_filter_url.clone(),
            archive_url: settings.data_gov_hk_archive_url.clone(),
            client,
        })
    }

    fn resource_for(&self, dataset: &str) -> Option<&'static DataGovResource> {
        RESOURCES.iter().find(|r| r.slug == dataset)
    }

    /// Build the `q` parameter value for the v2 filter API.
    fn build_query(&self, resource_url: &str) -> serde_json::Value {
        serde_json::json!({
            "resource": resource_url,
            "section": 1,
            "format": "json"
        })
    }

    async fn fetch_filter(&self, resource_url: &str) -> Result<Vec<serde_json::Value>> {
        let q = self.build_query(resource_url);
        // The v2 filter API expects q as a JSON string in the query.
        let resp = self
            .client
            .get(&self.filter_url)
            .query(&[("q", q.to_string())])
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "datagovhk",
                status: 0,
                detail: format!("transport: {e}"),
            })?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "datagovhk",
                status,
                detail,
            });
        }

        // The filter API returns either a bare JSON array or an error object
        // like {"code":"422","message":"..."}. Handle both.
        let text = resp.text().await.map_err(|e| Error::Decode {
            origin: "datagovhk",
            backtrace: serde::de::Error::custom(format!("body read: {e}")),
        })?;
        let trimmed = text.trim_start();
        if trimmed.starts_with('[') {
            serde_json::from_str::<Vec<serde_json::Value>>(&text).map_err(|e| Error::Decode {
                origin: "datagovhk",
                backtrace: e,
            })
        } else {
            // Try to surface the platform error message.
            let detail = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("message").cloned())
                .map(|m| m.to_string())
                .unwrap_or_else(|| text.chars().take(200).collect());
            Err(Error::Upstream {
                origin: "datagovhk",
                status,
                detail,
            })
        }
    }

    /// Historical archive file listing. Returns the raw JSON for now; useful for
    /// the agent layer to discover what changed day-to-day.
    #[allow(dead_code)]
    pub async fn list_archive(
        &self,
        provider: &str,
        start: &str,
        end: &str,
    ) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(&self.archive_url)
            .query(&[
                ("start", start),
                ("end", end),
                ("provider", provider),
                ("max", "100"),
            ])
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "datagovhk",
                status: 0,
                detail: format!("transport: {e}"),
            })?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "datagovhk",
                status,
                detail,
            });
        }
        resp.json().await.map_err(|e| Error::Decode {
            origin: "datagovhk",
            backtrace: serde::de::Error::custom(e.to_string()),
        })
    }
}

#[async_trait]
impl Connector for DataGovHkConnector {
    fn source(&self) -> DataSource {
        DataSource::DataGovHk
    }

    fn datasets(&self) -> &[DatasetSpec] {
        // The table is the source of truth; project lazily on first call so the
        // `&'static`-bound return of the trait is sound.
        ensure_specs_initialized();
        DATAGOVHK_SPECS
            .get()
            .map(Vec::as_slice)
            .expect("datagovhk specs initialized")
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let resource = self.resource_for(dataset).ok_or_else(|| {
            Error::Internal(format!(
                "datagovhk: no resource mapping for dataset {dataset}"
            ))
        })?;
        let rows = self.fetch_filter(resource.resource_url).await?;
        let now = Utc::now();
        let records: Vec<NormalizedRecord> = rows
            .into_iter()
            .map(|raw| {
                let fields = normalize_row(&raw);
                let record_id = record_id_for(dataset, resource.id_field, &fields);
                NormalizedRecord {
                    source: DataSource::DataGovHk,
                    dataset: dataset.to_string(),
                    record_id,
                    fields,
                    fetched_at: now,
                }
            })
            .collect();
        tracing::info!(dataset, count = records.len(), "datagovhk: fetched dataset");
        Ok(records)
    }
}

/// Lazy-built `DatasetSpec` slice projected from [`RESOURCES`].
static DATAGOVHK_SPECS: std::sync::OnceLock<Vec<DatasetSpec>> = std::sync::OnceLock::new();

/// Initialize the projected specs once. Called from the registry build so the
/// `&'static` lifetime in `datasets()` is sound.
pub(crate) fn ensure_specs_initialized() {
    DATAGOVHK_SPECS.get_or_init(|| {
        RESOURCES
            .iter()
            .map(|r| {
                let desc = format!("data.gov.hk: {}", r.title);
                DatasetSpec {
                    id: r.slug,
                    title: r.title,
                    description: Some(Box::leak(desc.into_boxed_str())),
                    category: r.category,
                    tags: r.tags,
                    cadence: r.cadence,
                    refresh_interval_secs: 24 * 3600,
                }
            })
            .collect()
    });
}

fn normalize_row(raw: &serde_json::Value) -> BTreeMap<String, RecordValue> {
    let Some(obj) = raw.as_object() else {
        return BTreeMap::new();
    };
    obj.iter()
        .map(|(k, v)| (k.clone(), json_to_value(v)))
        .collect()
}

fn json_to_value(v: &serde_json::Value) -> RecordValue {
    match v {
        serde_json::Value::Null => RecordValue::Null,
        serde_json::Value::Bool(b) => RecordValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                RecordValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                RecordValue::Float(f)
            } else {
                RecordValue::Null
            }
        }
        serde_json::Value::String(s) => RecordValue::Str(s.clone()),
        other => RecordValue::Str(other.to_string()),
    }
}

fn record_id_for(
    _dataset: &str,
    id_field: Option<&str>,
    fields: &BTreeMap<String, RecordValue>,
) -> String {
    if let Some(field) = id_field {
        if let Some(v) = fields.get(field) {
            return match v {
                RecordValue::Str(s) => s.clone(),
                RecordValue::Int(i) => i.to_string(),
                other => format!("{other:?}"),
            };
        }
    }
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (k, v) in fields {
        k.hash(&mut h);
        format!("{v:?}").hash(&mut h);
    }
    format!("id-{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_object_row() {
        let raw: serde_json::Value = serde_json::json!({
            "MLR_No": 6384,
            "Name_Eng": "VCREDIT Finance Limited",
            "Expiry_Date": "27-Apr-27"
        });
        let fields = normalize_row(&raw);
        assert_eq!(fields.get("MLR_No"), Some(&RecordValue::Int(6384)));
        assert_eq!(
            fields.get("Name_Eng"),
            Some(&RecordValue::Str("VCREDIT Finance Limited".into()))
        );
    }

    #[test]
    fn record_id_uses_configured_field() {
        let mut fields = BTreeMap::new();
        fields.insert("MLR_No".into(), RecordValue::Int(6384));
        let id = record_id_for("money-lenders-licensees", Some("MLR_No"), &fields);
        assert_eq!(id, "6384");
    }

    #[test]
    fn record_id_falls_back_to_hash_when_no_id_field() {
        let mut fields = BTreeMap::new();
        fields.insert("foo".into(), RecordValue::Str("bar".into()));
        let id = record_id_for("x", None, &fields);
        assert!(id.starts_with("id-"));
    }

    #[test]
    fn resource_table_is_well_formed() {
        // Regression guard: every row is unique + URL is a verified PSI path.
        assert!(
            RESOURCES.len() >= 30,
            "datagovhk resource count drifted below the verified set"
        );
        let mut seen = std::collections::HashSet::new();
        for r in RESOURCES {
            assert!(seen.insert(r.slug), "duplicate datagovhk slug: {}", r.slug);
        }
        for r in RESOURCES {
            assert!(!r.resource_url.is_empty(), "empty url for {}", r.slug);
            // Every registered URL uses one of the verified PSI host patterns.
            assert!(
                r.resource_url.contains("/datagovhk/")
                    || r.resource_url.contains("/files/misc/")
                    || r.resource_url.contains("static.data.gov.hk/tramways")
                    || r.resource_url.contains("centanet.com/opendata")
                    || r.resource_url.contains("/filemanager/ofca/"),
                "resource URL for {} is not on a verified PSI path: {}",
                r.slug,
                r.resource_url
            );
        }
    }

    #[test]
    fn money_lenders_resource_preserved() {
        // The original v1 resource must still be present with its id field.
        let r = RESOURCES
            .iter()
            .find(|r| r.slug == "money-lenders-licensees")
            .expect("money-lenders-licensees must remain registered");
        assert_eq!(r.id_field, Some("MLR_No"));
    }
}
