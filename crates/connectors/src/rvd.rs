//! Rating & Valuation Department (差餉物業估價處, RVD) connector — property
//! price/rental indices.
//!
//! Source: the monthly index CSVs published at
//! `https://www.rvd.gov.hk/datagovhk/` (verified live):
//! - `1.4M.csv` — Private Domestic **Price** Indices by Class (territory-wide).
//! - `1.3M.csv` — Private Domestic **Rental** Indices by Class (territory-wide).
//!
//! Each file is one row per month from 1993 onward. The classes are flats by
//! saleable area: A (<40 m²), B (40–69.9), C (70–99.9), D (100–159.9),
//! E (≥160), plus the A–B–C and D–E groupings and an All-Classes total. These
//! feed the property Silence Index (a 10% monthly index move is large for a
//! smoothed property series and worth flagging).
//!
//! The CSV is **plain text, not a queryable API** (data.gov.hk badges it "API"
//! but that refers to CKAN catalog metadata only; there is no JSON/filter
//! endpoint). We pull the whole file and parse it client-side.
//!
//! Quirks handled here (verified against the live file):
//! - `Month` is `MM-YYYY` (e.g. `05-2026`) → converted to ISO `YYYY-MM` for the
//!   `record_id`.
//! - Value columns are **interleaved** with `<Class> - Remarks` columns: every
//!   class has a value cell immediately followed by a remarks cell. We read by
//!   position (fixed column indices) and ignore the remarks columns entirely.
//! - Cells may be empty or carry footnote markers → parsed defensively (non-
//!   numeric values are skipped, never crash).

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use std::collections::BTreeMap;
use std::time::Duration;

/// The two datasets this connector exposes. Both share the same CSV shape;
/// only the upstream file and the dataset id differ. `&'static` — no projection
/// needed because every field is a literal.
const DATASETS: &[DatasetSpec] = &[
    DatasetSpec {
        id: "price-indices-monthly",
        title: "Private Domestic Price Indices by Class",
        description: Some(
            "Rating & Valuation Department monthly price indices for private \
             domestic flats, broken down by size class (A–E), the A–B–C and \
             D–E groupings, and an All-Classes total. Territory-wide, from \
             1993. This is the headline HK property-price series.",
        ),
        category: Category::Property,
        tags: &["rvd", "property", "price-index", "domestic", "by-class"],
        cadence: Cadence::Monthly,
        refresh_interval_secs: 60 * 60, // hourly refresh window
    },
    DatasetSpec {
        id: "rental-indices-monthly",
        title: "Private Domestic Rental Indices by Class",
        description: Some(
            "Rating & Valuation Department monthly rental indices for private \
             domestic flats, broken down by size class (A–E), the A–B–C and \
             D–E groupings, and an All-Classes total. Territory-wide, from \
             1993. Covers the 地區樓盤租金 / by-class rental breakdown.",
        ),
        category: Category::Property,
        tags: &["rvd", "property", "rental-index", "domestic", "by-class"],
        cadence: Cadence::Monthly,
        refresh_interval_secs: 60 * 60,
    },
];

const DEFAULT_PRICE_CSV_URL: &str = "https://www.rvd.gov.hk/datagovhk/1.4M.csv";
const DEFAULT_RENTAL_CSV_URL: &str = "https://www.rvd.gov.hk/datagovhk/1.3M.csv";

pub struct RvdConnector {
    price_csv_url: String,
    rental_csv_url: String,
    client: reqwest::Client,
}

impl RvdConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(settings.hkma_timeout_ms.max(30_000)))
            .gzip(true)
            .pool_max_idle_per_host(16)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            price_csv_url: DEFAULT_PRICE_CSV_URL.to_string(),
            rental_csv_url: DEFAULT_RENTAL_CSV_URL.to_string(),
            client,
        })
    }
}

#[async_trait]
impl Connector for RvdConnector {
    fn source(&self) -> DataSource {
        DataSource::Rvd
    }

    fn datasets(&self) -> &[DatasetSpec] {
        DATASETS
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let (url, dataset_id) = match dataset {
            "price-indices-monthly" => (self.price_csv_url.as_str(), "price-indices-monthly"),
            "rental-indices-monthly" => (self.rental_csv_url.as_str(), "rental-indices-monthly"),
            other => {
                return Err(Error::Internal(format!(
                    "rvd: no dataset mapping for {other}"
                )))
            }
        };

        let body = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "rvd",
                status: 0,
                detail: format!("transport: {e}"),
            })?
            .error_for_status()
            .map_err(|e| Error::Upstream {
                origin: "rvd",
                status: e.status().map(|s| s.as_u16()).unwrap_or(0),
                detail: format!("http: {e}"),
            })?
            .text()
            .await
            .map_err(|e| Error::Upstream {
                origin: "rvd",
                status: 0,
                detail: format!("body read: {e}"),
            })?;

        let now = Utc::now();
        let rows = parse_csv(&body)?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_record(dataset_id, now))
            .collect())
    }
}

// ---- CSV parsing ----

/// The fixed column positions of the value cells in both RVD CSVs. The header
/// is interleaved — each class value cell is immediately followed by a
/// `<Class> - Remarks` cell — so the value cells live at these exact indices:
///
/// ```text
/// 0  Month
/// 1  Class A               2  Class A - Remarks
/// 3  Class B               4  Class B - Remarks
/// 5  Class C               6  Class C - Remarks
/// 7  Class D               8  Class D - Remarks
/// 9  Class E              10  Class E - Remarks
/// 11 Classes A, B & C     12  Classes A, B & C - Remarks
/// 13 Classes D & E        14  Classes D & E - Remarks
/// 15 All Classes          16  All Classes - Remarks
/// ```
const COL_MONTH: usize = 0;
const COL_CLASS_A: usize = 1;
const COL_CLASS_B: usize = 3;
const COL_CLASS_C: usize = 5;
const COL_CLASS_D: usize = 7;
const COL_CLASS_E: usize = 9;
const COL_CLASSES_ABC: usize = 11;
const COL_CLASSES_DE: usize = 13;
const COL_ALL_CLASSES: usize = 15;

/// One parsed row of the upstream CSV (before normalization). `month_iso` is the
/// converted ISO `YYYY-MM`; the index fields are `Option` because cells may be
/// empty or carry footnote markers.
struct IndexRow {
    month_iso: String,
    class_a: Option<f64>,
    class_b: Option<f64>,
    class_c: Option<f64>,
    class_d: Option<f64>,
    class_e: Option<f64>,
    classes_abc: Option<f64>,
    classes_de: Option<f64>,
    all_classes: Option<f64>,
}

impl IndexRow {
    /// Normalize into one record per month. Rows with no parseable month are
    /// dropped upstream; an index field that failed to parse is simply omitted
    /// from `fields` rather than emitted as null.
    fn into_record(self, dataset_id: &str, now: chrono::DateTime<Utc>) -> Option<NormalizedRecord> {
        if self.month_iso.is_empty() {
            return None;
        }
        let mut fields = BTreeMap::new();
        if let Some(v) = self.class_a {
            fields.insert("class_a".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.class_b {
            fields.insert("class_b".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.class_c {
            fields.insert("class_c".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.class_d {
            fields.insert("class_d".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.class_e {
            fields.insert("class_e".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.classes_abc {
            fields.insert("classes_abc".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.classes_de {
            fields.insert("classes_de".into(), RecordValue::Float(v));
        }
        if let Some(v) = self.all_classes {
            fields.insert("all_classes".into(), RecordValue::Float(v));
        }
        Some(NormalizedRecord {
            source: DataSource::Rvd,
            dataset: dataset_id.into(),
            record_id: self.month_iso,
            fields,
            fetched_at: now,
        })
    }
}

/// Parse the full CSV body into rows. Handles the interleaved remarks columns
/// (read by fixed position) and the `MM-YYYY` month format.
fn parse_csv(body: &str) -> Result<Vec<IndexRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true) // tolerate the title row's many trailing commas
        .trim(csv::Trim::All)
        .from_reader(body.as_bytes());
    let mut rows = Vec::new();
    for record in rdr.records() {
        let record = record.map_err(|e| Error::Decode {
            origin: "rvd",
            backtrace: serde::de::Error::custom(format!("csv parse: {e}")),
        })?;
        // Be defensive: skip rows without a parseable month (the title row is
        // consumed by has_headers, but blanks/malformed lines still skip here).
        let Some(month_raw) = record.get(COL_MONTH) else {
            continue;
        };
        let Some(month_iso) = parse_mm_yyyy(month_raw) else {
            continue; // skip malformed/unparseable months (e.g. blank rows)
        };
        rows.push(IndexRow {
            month_iso,
            class_a: parse_index(record.get(COL_CLASS_A)),
            class_b: parse_index(record.get(COL_CLASS_B)),
            class_c: parse_index(record.get(COL_CLASS_C)),
            class_d: parse_index(record.get(COL_CLASS_D)),
            class_e: parse_index(record.get(COL_CLASS_E)),
            classes_abc: parse_index(record.get(COL_CLASSES_ABC)),
            classes_de: parse_index(record.get(COL_CLASSES_DE)),
            all_classes: parse_index(record.get(COL_ALL_CLASSES)),
        });
    }
    Ok(rows)
}

/// Parse `"05-2026"` → `"2026-05"`. Returns `None` if unparseable.
fn parse_mm_yyyy(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // The upstream format is MM-YYYY. Try that first; fall back to an
    // already-ISO month (YYYY-MM) in case the source ever normalizes.
    if let Some((mm, yyyy)) = s.split_once('-') {
        if mm.len() == 2
            && yyyy.len() == 4
            && mm.bytes().all(|b| b.is_ascii_digit())
            && yyyy.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(format!("{yyyy}-{mm}"));
        }
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(&format!("{}-01", s), "%Y-%m-%d") {
        return Some(d.format("%Y-%m").to_string());
    }
    None
}

/// Parse an index cell that may be empty or carry a footnote marker (e.g.
/// `90.4`, ``, `*`). Returns `None` for anything non-numeric so the field is
/// simply omitted rather than emitted as null.
fn parse_index(s: Option<&str>) -> Option<f64> {
    let v = s?.trim();
    if v.is_empty() {
        return None;
    }
    v.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mm_yyyy_to_iso_month() {
        assert_eq!(parse_mm_yyyy("05-2026").as_deref(), Some("2026-05"));
        assert_eq!(parse_mm_yyyy("  01-1993 ").as_deref(), Some("1993-01"));
        assert_eq!(parse_mm_yyyy("12-2020").as_deref(), Some("2020-12"));
        // Already-ISO passthrough.
        assert_eq!(parse_mm_yyyy("2026-05").as_deref(), Some("2026-05"));
        assert!(parse_mm_yyyy("not-a-date").is_none());
        assert!(parse_mm_yyyy("").is_none());
    }

    #[test]
    fn parses_index_defensively() {
        assert_eq!(parse_index(Some("90.4")), Some(90.4));
        assert_eq!(parse_index(Some("  85.5 ")), Some(85.5));
        // Empty / footnote / non-numeric → None (skip gracefully).
        assert_eq!(parse_index(Some("")), None);
        assert_eq!(parse_index(Some("*")), None);
        assert_eq!(parse_index(Some("N/A")), None);
        assert_eq!(parse_index(None), None);
    }

    #[test]
    fn parses_csv_sample_with_remarks_and_blank_cell() {
        // A miniature version of the live file: title row + header + 2 rows.
        // Value cells are interleaved with `- Remarks` columns; the second row
        // has an empty Class C cell (simulating a missing value).
        let sample = "PRIVATE DOMESTIC PRICE INDICES,,,,,,,,,,,,,,,\n\
                      Month,Class A,Class A - Remarks,Class B,Class B - Remarks,Class C,Class C - Remarks,Class D,Class D - Remarks,Class E,Class E - Remarks,\"Classes A, B & C\",\"Classes A, B & C - Remarks\",Classes D & E,Classes D & E - Remarks,All Classes,All Classes - Remarks\n\
                      01-1993,90.4,,81.3,,80.7,,74.2,,58.1,,85.5,,69.6,,84.4,\n\
                      05-2026,310.2,,300.1,,,3,210.5,,190.0,,305.0,,200.0,,290.0,\n";
        let rows = parse_csv(sample).unwrap();
        assert_eq!(rows.len(), 2);
        // First row: clean.
        assert_eq!(rows[0].month_iso, "1993-01");
        assert_eq!(rows[0].class_a, Some(90.4));
        assert_eq!(rows[0].class_b, Some(81.3));
        assert_eq!(rows[0].class_c, Some(80.7));
        assert_eq!(rows[0].all_classes, Some(84.4));
        // Second row: Class C cell is "" → None (skipped, not a crash).
        assert_eq!(rows[1].month_iso, "2026-05");
        assert_eq!(rows[1].class_a, Some(310.2));
        assert_eq!(
            rows[1].class_c, None,
            "empty Class C cell must parse to None"
        );
        assert_eq!(rows[1].all_classes, Some(290.0));
    }

    #[test]
    fn into_record_omits_unparseable_fields() {
        let now = Utc::now();
        let row = IndexRow {
            month_iso: "2026-05".into(),
            class_a: Some(310.2),
            class_b: None, // unparseable / missing
            class_c: Some(280.0),
            class_d: None,
            class_e: None,
            classes_abc: Some(305.0),
            classes_de: None,
            all_classes: Some(290.0),
        };
        let rec = row.into_record("price-indices-monthly", now).unwrap();
        assert_eq!(rec.source, DataSource::Rvd);
        assert_eq!(rec.dataset, "price-indices-monthly");
        assert_eq!(rec.record_id, "2026-05");
        // class_a present.
        assert_eq!(
            rec.fields.get("class_a").and_then(|v| match v {
                RecordValue::Float(f) => Some(*f),
                _ => None,
            }),
            Some(310.2)
        );
        // class_b absent (omitted, not null).
        assert!(
            !rec.fields.contains_key("class_b"),
            "unparseable field must be omitted, not emitted"
        );
        // all_classes present.
        assert_eq!(
            rec.fields.get("all_classes").and_then(|v| match v {
                RecordValue::Float(f) => Some(*f),
                _ => None,
            }),
            Some(290.0)
        );
    }

    #[test]
    fn into_record_drops_empty_month() {
        let now = Utc::now();
        let row = IndexRow {
            month_iso: String::new(),
            class_a: Some(1.0),
            class_b: None,
            class_c: None,
            class_d: None,
            class_e: None,
            classes_abc: None,
            classes_de: None,
            all_classes: None,
        };
        assert!(row.into_record("price-indices-monthly", now).is_none());
    }
}
