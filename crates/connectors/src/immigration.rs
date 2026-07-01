//! Immigration Department (入境事務處) connector — border-crossing traffic.
//!
//! Source: the daily passenger-traffic CSV published at
//! `https://www.immd.gov.hk/opendata/eng/transport/immigration_clearance/`
//! `statistics_on_daily_passenger_traffic.csv` (verified live; one row per
//! `Date × Control Point × Direction`, refreshed daily since 2021).
//!
//! Two datasets are exposed:
//! - **`daily-passenger-traffic`** — the full tidy breakdown (one record per
//!   checkpoint × direction × day). Useful for the timeline / drill-down.
//! - **`daily-passenger-traffic-totals`** — one record per day, aggregated
//!   across all control points, with separate columns for arrivals vs.
//!   departures and HK-resident vs. Mainland-visitor totals. This is the
//!   series `series_jump` runs on (a halving/doubling of cross-border flow is
//!   the headline opacity signal — e.g. a checkpoint quietly closed).
//!
//! The CSV is **plain text, not a queryable API** (data.gov.hk badges it "API"
//! but that refers to CKAN catalog metadata only; there is no JSON/filter
//! endpoint). We pull the whole file and parse it client-side.
//!
//! Quirks handled here (verified against the live file):
//! - Dates are `DD-MM-YYYY` (not ISO) → parsed to `YYYY-MM-DD` for `record_id`.
//! - A trailing empty column on every row (extra comma in the header).
//! - Long/tidy format: ~28 rows per day (14 checkpoints × 2 directions).

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use std::collections::BTreeMap;
use std::time::Duration;

/// The two datasets this connector exposes. `&'static` — no projection needed
/// because every field is a literal.
const DATASETS: &[DatasetSpec] = &[
    DatasetSpec {
        id: "daily-passenger-traffic",
        title: "Daily Passenger Traffic by Control Point",
        description: Some(
            "Immigration Department daily statistics on passenger traffic, \
             broken down by control point (border crossing), direction \
             (arrival/departure), and visitor type (HK residents / Mainland \
             visitors / other). One record per checkpoint × direction × day.",
        ),
        category: Category::Livability,
        tags: &[
            "immigration",
            "border-crossing",
            "passenger-traffic",
            "control-point",
        ],
        cadence: Cadence::Daily,
        refresh_interval_secs: 60 * 60, // hourly refresh window
    },
    DatasetSpec {
        id: "daily-passenger-traffic-totals",
        title: "Daily Passenger Traffic — Territory Totals",
        description: Some(
            "Immigration Department daily passenger traffic aggregated across \
             all control points to one record per day. Separate columns for \
             arrivals, departures, HK residents, Mainland visitors, and others. \
             This is the series the series_jump detector runs on.",
        ),
        category: Category::Livability,
        tags: &[
            "immigration",
            "border-crossing",
            "passenger-traffic",
            "totals",
        ],
        cadence: Cadence::Daily,
        refresh_interval_secs: 60 * 60,
    },
];

const DEFAULT_CSV_URL: &str =
    "https://www.immd.gov.hk/opendata/eng/transport/immigration_clearance/statistics_on_daily_passenger_traffic.csv";

pub struct ImmigrationConnector {
    csv_url: String,
    client: reqwest::Client,
}

impl ImmigrationConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(settings.hkma_timeout_ms.max(30_000)))
            .gzip(true)
            .pool_max_idle_per_host(16)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self {
            csv_url: DEFAULT_CSV_URL.to_string(),
            client,
        })
    }
}

#[async_trait]
impl Connector for ImmigrationConnector {
    fn source(&self) -> DataSource {
        DataSource::Immigration
    }

    fn datasets(&self) -> &[DatasetSpec] {
        DATASETS
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let body = self
            .client
            .get(&self.csv_url)
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "immigration",
                status: 0,
                detail: format!("transport: {e}"),
            })?
            .error_for_status()
            .map_err(|e| Error::Upstream {
                origin: "immigration",
                status: e.status().map(|s| s.as_u16()).unwrap_or(0),
                detail: format!("http: {e}"),
            })?
            .text()
            .await
            .map_err(|e| Error::Upstream {
                origin: "immigration",
                status: 0,
                detail: format!("body read: {e}"),
            })?;

        let rows = parse_csv(&body)?;
        let now = Utc::now();
        match dataset {
            "daily-passenger-traffic" => Ok(rows
                .into_iter()
                .filter_map(|r| r.into_record(now))
                .collect()),
            "daily-passenger-traffic-totals" => {
                let totals = aggregate_totals(&rows, now)?;
                Ok(totals)
            }
            other => Err(Error::Internal(format!(
                "immigration: no dataset mapping for {other}"
            ))),
        }
    }
}

// ---- CSV parsing ----

/// One parsed row of the upstream CSV (before normalization).
struct TrafficRow {
    /// ISO `YYYY-MM-DD`, converted from the upstream `DD-MM-YYYY`.
    date_iso: String,
    control_point: String,
    direction: String, // "Arrival" | "Departure"
    hk_residents: i64,
    mainland_visitors: i64,
    other_visidents: i64,
    total: i64,
}

impl TrafficRow {
    /// Normalize into the full tidy record (one per checkpoint × direction).
    fn into_record(self, now: chrono::DateTime<Utc>) -> Option<NormalizedRecord> {
        if self.date_iso.is_empty() {
            return None;
        }
        let mut fields = BTreeMap::new();
        fields.insert("control_point".into(), RecordValue::Str(self.control_point));
        fields.insert("direction".into(), RecordValue::Str(self.direction));
        fields.insert("hk_residents".into(), RecordValue::Int(self.hk_residents));
        fields.insert(
            "mainland_visitors".into(),
            RecordValue::Int(self.mainland_visitors),
        );
        fields.insert(
            "other_visitors".into(),
            RecordValue::Int(self.other_visidents),
        );
        fields.insert("total".into(), RecordValue::Int(self.total));
        Some(NormalizedRecord {
            source: DataSource::Immigration,
            dataset: "daily-passenger-traffic".into(),
            record_id: self.date_iso,
            fields,
            fetched_at: now,
        })
    }
}

/// Parse the full CSV body into rows. Handles the trailing empty column and
/// the `DD-MM-YYYY` date format.
fn parse_csv(body: &str) -> Result<Vec<TrafficRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true) // tolerate the trailing empty column
        .trim(csv::Trim::All)
        .from_reader(body.as_bytes());
    let mut rows = Vec::new();
    for record in rdr.records() {
        let record = record.map_err(|e| Error::Decode {
            origin: "immigration",
            backtrace: serde::de::Error::custom(format!("csv parse: {e}")),
        })?;
        // The upstream schema (verified live):
        //   Date, Control Point, Arrival / Departure,
        //   Hong Kong Residents, Mainland Visitors, Other Visitors, Total [, trailing empty]
        // Be defensive: skip rows that don't have enough columns.
        let Some(date_raw) = record.get(0) else {
            continue;
        };
        let Some(control_point) = record.get(1) else {
            continue;
        };
        let Some(direction) = record.get(2) else {
            continue;
        };
        let Some(date_iso) = parse_dd_mm_yyyy(date_raw) else {
            continue; // skip malformed/unparseable dates (e.g. blank rows)
        };
        rows.push(TrafficRow {
            date_iso,
            control_point: control_point.to_string(),
            direction: direction.to_string(),
            hk_residents: parse_count(record.get(3)),
            mainland_visitors: parse_count(record.get(4)),
            other_visidents: parse_count(record.get(5)),
            total: parse_count(record.get(6)),
        });
    }
    Ok(rows)
}

/// Parse `"30-06-2026"` → `"2026-06-30"`. Returns `None` if unparseable.
fn parse_dd_mm_yyyy(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // The upstream format is DD-MM-YYYY. Try that first; fall back to an
    // already-ISO value in case the source ever normalizes.
    if let Ok(d) = NaiveDate::parse_from_str(s, "%d-%m-%Y") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    None
}

/// Parse a count column that may carry commas ("1,186") or be blank.
fn parse_count(s: Option<&str>) -> i64 {
    s.and_then(|v| v.trim().replace(',', "").parse::<i64>().ok())
        .unwrap_or(0)
}

/// Aggregate the tidy rows into one record per day for the totals dataset.
/// Sums across all control points; separates arrivals vs departures and the
/// three visitor types into distinct columns so `series_jump` can pick a field.
fn aggregate_totals(
    rows: &[TrafficRow],
    now: chrono::DateTime<Utc>,
) -> Result<Vec<NormalizedRecord>> {
    use std::collections::HashMap;
    // Per-day accumulators.
    #[derive(Default)]
    struct DayTotals {
        arrivals: i64,
        departures: i64,
        hk_residents: i64,
        mainland_visitors: i64,
        other_visitors: i64,
        total: i64,
    }
    let mut by_date: HashMap<String, DayTotals> = HashMap::new();
    for r in rows {
        let t = by_date.entry(r.date_iso.clone()).or_default();
        if r.direction.eq_ignore_ascii_case("Arrival") {
            t.arrivals += r.total;
        } else if r.direction.eq_ignore_ascii_case("Departure") {
            t.departures += r.total;
        }
        t.hk_residents += r.hk_residents;
        t.mainland_visitors += r.mainland_visitors;
        t.other_visitors += r.other_visidents;
        t.total += r.total;
    }
    let mut out: Vec<NormalizedRecord> = by_date
        .into_iter()
        .map(|(date_iso, t)| {
            let mut fields = BTreeMap::new();
            fields.insert("arrivals".into(), RecordValue::Int(t.arrivals));
            fields.insert("departures".into(), RecordValue::Int(t.departures));
            fields.insert("hk_residents".into(), RecordValue::Int(t.hk_residents));
            fields.insert(
                "mainland_visitors".into(),
                RecordValue::Int(t.mainland_visitors),
            );
            fields.insert("other_visitors".into(), RecordValue::Int(t.other_visitors));
            fields.insert("total".into(), RecordValue::Int(t.total));
            NormalizedRecord {
                source: DataSource::Immigration,
                dataset: "daily-passenger-traffic-totals".into(),
                record_id: date_iso,
                fields,
                fetched_at: now,
            }
        })
        .collect();
    // Sort by date (record_id) for deterministic ordering.
    out.sort_by(|a, b| a.record_id.cmp(&b.record_id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dd_mm_yyyy_to_iso() {
        assert_eq!(
            parse_dd_mm_yyyy("30-06-2026").as_deref(),
            Some("2026-06-30")
        );
        assert_eq!(
            parse_dd_mm_yyyy("  01-01-2021 ").as_deref(),
            Some("2021-01-01")
        );
        // Already-ISO passthrough.
        assert_eq!(
            parse_dd_mm_yyyy("2026-06-30").as_deref(),
            Some("2026-06-30")
        );
        assert!(parse_dd_mm_yyyy("not-a-date").is_none());
        assert!(parse_dd_mm_yyyy("").is_none());
    }

    #[test]
    fn parses_count_with_commas() {
        assert_eq!(parse_count(Some("1,186")), 1186);
        assert_eq!(parse_count(Some("42")), 42);
        assert_eq!(parse_count(Some("")), 0);
        assert_eq!(parse_count(None), 0);
    }

    #[test]
    fn parses_csv_sample_into_tidy_records() {
        // A miniature version of the live file: header + 2 rows + trailing comma.
        let sample = "Date,Control Point,Arrival / Departure,Hong Kong Residents,Mainland Visitors,Other Visitors,Total,\n\
                      30-06-2026,Airport,Arrival,1000,500,200,1700,\n\
                      30-06-2026,Lo Wu,Arrival,2000,8000,100,10100,\n";
        let rows = parse_csv(sample).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date_iso, "2026-06-30");
        assert_eq!(rows[0].control_point, "Airport");
        assert_eq!(rows[0].direction, "Arrival");
        assert_eq!(rows[0].total, 1700);
        assert_eq!(rows[1].control_point, "Lo Wu");
        assert_eq!(rows[1].mainland_visitors, 8000);
    }

    #[test]
    fn aggregate_totals_sums_per_day_and_splits_direction() {
        let rows = vec![
            TrafficRow {
                date_iso: "2026-06-30".into(),
                control_point: "Airport".into(),
                direction: "Arrival".into(),
                hk_residents: 1000,
                mainland_visitors: 500,
                other_visidents: 200,
                total: 1700,
            },
            TrafficRow {
                date_iso: "2026-06-30".into(),
                control_point: "Lo Wu".into(),
                direction: "Arrival".into(),
                hk_residents: 2000,
                mainland_visitors: 8000,
                other_visidents: 100,
                total: 10100,
            },
            TrafficRow {
                date_iso: "2026-06-30".into(),
                control_point: "Airport".into(),
                direction: "Departure".into(),
                hk_residents: 900,
                mainland_visitors: 400,
                other_visidents: 150,
                total: 1450,
            },
        ];
        let now = Utc::now();
        let totals = aggregate_totals(&rows, now).unwrap();
        assert_eq!(totals.len(), 1, "one day → one record");
        let rec = &totals[0];
        assert_eq!(rec.record_id, "2026-06-30");
        assert_eq!(
            rec.fields.get("arrivals").and_then(|v| match v {
                RecordValue::Int(i) => Some(*i),
                _ => None,
            }),
            Some(11800),
            "arrivals = 1700 + 10100"
        );
        assert_eq!(
            rec.fields.get("departures").and_then(|v| match v {
                RecordValue::Int(i) => Some(*i),
                _ => None,
            }),
            Some(1450),
            "departures = 1450"
        );
        assert_eq!(
            rec.fields.get("mainland_visitors").and_then(|v| match v {
                RecordValue::Int(i) => Some(*i),
                _ => None,
            }),
            Some(8900),
            "mainland visitors summed across all rows"
        );
    }

    #[test]
    fn totals_records_are_date_sorted() {
        let rows = vec![
            TrafficRow {
                date_iso: "2026-06-30".into(),
                control_point: "Airport".into(),
                direction: "Arrival".into(),
                hk_residents: 1,
                mainland_visitors: 1,
                other_visidents: 1,
                total: 3,
            },
            TrafficRow {
                date_iso: "2026-06-28".into(),
                control_point: "Airport".into(),
                direction: "Arrival".into(),
                hk_residents: 1,
                mainland_visitors: 1,
                other_visidents: 1,
                total: 3,
            },
        ];
        let now = Utc::now();
        let totals = aggregate_totals(&rows, now).unwrap();
        assert_eq!(totals[0].record_id, "2026-06-28");
        assert_eq!(totals[1].record_id, "2026-06-30");
    }
}
