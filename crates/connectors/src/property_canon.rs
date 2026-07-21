//! Canonical property projection — M5 (Property / NT Metropolis Intelligence).
//!
//! The 6 property connectors (RVD, LandRegistry, HKP, Midland, Chung Sen, AA
//! Property) expose 9 datasets with 5 distinct field vocabularies. This module
//! projects them all onto one [`CanonicalListing`] struct so cross-portal
//! comparison becomes possible — the prerequisite for a cross-portal divergence
//! detector and an NT-Metropolis market composite.
//!
//! ## The divergence this reconciles
//!
//! - Price: `price_hkd` (HKP txns) / `sale_price_hkd` (Midland) / `price_10k`
//!   (Chung Sen — HK$10k units) / `price_hint` (AA Property — free text, often
//!   unparseable → None).
//! - Area: `build_area_sqft` + `net_area_sqft` (HKP/Midland) /
//!   `saleable_area_sqft` (Chung Sen) / `area_sqft` (AA Property — ambiguous).
//! - Per-sqft: `unit_price_net` (HKP) / `price_per_net_sqft` (Midland).
//! - Market segment: `mkt_type`/`is_firsthand` (HKP) / `is_foreclosure`/
//!   `tags` (Midland) / `page_label` (Chung Sen).
//!
//! Honesty rule: when a field can't be reliably parsed (AA Property's free-text
//! price, ambiguous area), the projection returns `None` rather than guessing.
//! A cross-portal composite then only counts portals that actually contributed.

use hkgov_common::{DataSource, NormalizedRecord, RecordValue};

/// A property listing/transaction projected onto a canonical vocabulary, so
/// cross-portal comparison is possible. All fields are optional — a portal that
/// doesn't expose a field leaves it None (honest, not guessed).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CanonicalListing {
    /// Which portal this listing came from.
    pub source: DataSource,
    /// The portal's own record id (preserved for evidence pointers).
    pub source_record_id: String,
    /// Normalized region: "hk", "kln", "nt", or None when the portal doesn't
    /// carry region (Chung Sen, AA Property are address-only).
    pub region: Option<String>,
    /// Estate name (HKP, Midland carry it; others don't).
    pub estate: Option<String>,
    /// Sale/transaction price in HKD. Chung Sen's price_10k is multiplied by
    /// 10,000. AA Property's price_hint is usually unparseable → None.
    pub price_hkd: Option<f64>,
    /// Gross floor area in sqft.
    pub build_area_sqft: Option<f64>,
    /// Saleable/net floor area in sqft. Chung Sen's `saleable_area_sqft` maps
    /// here; HKP/Midland's `net_area_sqft` maps here.
    pub net_area_sqft: Option<f64>,
    /// Per-sqft price (net). Derived = price_hkd / net_area_sqft when both are
    /// present; taken directly from the portal's unit_price field when not.
    pub unit_price_net: Option<f64>,
    /// True = primary/new, False = secondary. Normalized from the portal's
    /// market-segment field.
    pub is_primary: Option<bool>,
    /// YYYY-MM for time-bucketing (the cross-portal composite groups by this).
    pub tx_date_month: Option<String>,
    /// Transaction type: "S" (sale), "L" (let/lease), "auction", or None.
    pub tx_type: Option<String>,
}

/// Project a normalized record onto the canonical vocabulary, using the
/// portal-specific field mapping. Returns None when the record has no
/// price/area signal at all (e.g. an RVD index row, which is a pure time
/// series — those don't project to a listing).
///
/// The mapping is dispatch-by-source so each portal's vocabulary is handled in
/// one place. Adding a new portal is adding an arm.
pub fn project(record: &NormalizedRecord, source: DataSource) -> Option<CanonicalListing> {
    match source {
        DataSource::Hkp => project_hkp(record),
        DataSource::Midland => project_midland(record),
        DataSource::ChungSen => project_chungsen(record),
        DataSource::AaProperty => project_aaproperty(record),
        // RVD and LandRegistry are time-series (indices / monthly aggregates),
        // not per-listing records — they don't project to a CanonicalListing.
        // They feed the cross-portal composite via their aggregate fields
        // instead (handled in the composite builder, not here).
        DataSource::Rvd | DataSource::LandRegistry => None,
        _ => None,
    }
}

// ---- Per-portal projections ----

fn project_hkp(r: &NormalizedRecord) -> Option<CanonicalListing> {
    // HKP transactions: price_hkd, build_area_sqft, net_area_sqft,
    // unit_price_net, region, estate_name, mkt_type/is_firsthand, tx_type,
    // tx_date_iso.
    let price_hkd = f64_field(r, "price_hkd");
    let build_area = f64_field(r, "build_area_sqft");
    let net_area = f64_field(r, "net_area_sqft");
    let unit_price = f64_field(r, "unit_price_net");
    // If there's no price and no area, this isn't a projectable listing.
    if price_hkd.is_none() && build_area.is_none() && net_area.is_none() {
        return None;
    }
    let region = str_field(r, "region").map(normalize_region);
    let estate = str_field(r, "estate_name");
    let is_primary = bool_field(r, "is_firsthand").or_else(|| {
        str_field(r, "mkt_type").and_then(|m| match m.as_str() {
            "1ST" => Some(true),
            "2ND" => Some(false),
            _ => None,
        })
    });
    let tx_type = str_field(r, "tx_type");
    let tx_date_month = str_field(r, "tx_date_iso")
        .and_then(|d| d.get(..7).map(|s| s.to_string()))
        .or_else(|| str_field(r, "tx_date").and_then(|d| month_from(&d)));
    Some(CanonicalListing {
        source: DataSource::Hkp,
        source_record_id: r.record_id.clone(),
        region,
        estate,
        price_hkd,
        build_area_sqft: build_area,
        net_area_sqft: net_area,
        unit_price_net: unit_price.or_else(|| derive_unit_price(price_hkd, net_area)),
        is_primary,
        tx_date_month,
        tx_type,
    })
}

fn project_midland(r: &NormalizedRecord) -> Option<CanonicalListing> {
    // Midland: sale_price_hkd, build_area_sqft, net_area_sqft,
    // price_per_net_sqft, region, estate_name, is_foreclosure, tx_type.
    let price_hkd = f64_field(r, "sale_price_hkd");
    let build_area = f64_field(r, "build_area_sqft");
    let net_area = f64_field(r, "net_area_sqft");
    let unit_price = f64_field(r, "price_per_net_sqft");
    if price_hkd.is_none() && build_area.is_none() && net_area.is_none() {
        return None;
    }
    let region = str_field(r, "region").map(normalize_region);
    let estate = str_field(r, "estate_name");
    // Midland's is_foreclosure is the market-segment signal; a foreclosure is
    // by definition secondary.
    let is_primary = bool_field(r, "is_foreclosure").map(|fc| !fc && false);
    let _ = is_primary; // Midland doesn't cleanly distinguish primary; leave None.
    let tx_type = str_field(r, "tx_type");
    Some(CanonicalListing {
        source: DataSource::Midland,
        source_record_id: r.record_id.clone(),
        region,
        estate,
        price_hkd,
        build_area_sqft: build_area,
        net_area_sqft: net_area,
        unit_price_net: unit_price.or_else(|| derive_unit_price(price_hkd, net_area)),
        is_primary: None,
        tx_date_month: None,
        tx_type,
    })
}

fn project_chungsen(r: &NormalizedRecord) -> Option<CanonicalListing> {
    // Chung Sen: price_10k (HK$10k units → multiply by 10,000), build_area_sqft
    // (Int), saleable_area_sqft (Int → net). No region/estate (address-only).
    let price_10k = f64_field(r, "price_10k");
    let build_area = f64_field(r, "build_area_sqft");
    let net_area = f64_field(r, "saleable_area_sqft");
    if price_10k.is_none() && build_area.is_none() && net_area.is_none() {
        return None;
    }
    let price_hkd = price_10k.map(|p| p * 10_000.0);
    Some(CanonicalListing {
        source: DataSource::ChungSen,
        source_record_id: r.record_id.clone(),
        region: None,
        estate: None,
        price_hkd,
        build_area_sqft: build_area,
        net_area_sqft: net_area,
        unit_price_net: derive_unit_price(price_hkd, net_area),
        is_primary: None,
        tx_date_month: None,
        tx_type: None,
    })
}

fn project_aaproperty(r: &NormalizedRecord) -> Option<CanonicalListing> {
    // AA Property: area_sqft (ambiguous build/net), price_hint (free text,
    // usually "歡迎查詢" → unparseable). Honest: price is None unless the hint
    // parses to a number; area goes to build_area_sqft (can't tell build vs
    // net from one field).
    let area = f64_field(r, "area_sqft");
    let price_hint = str_field(r, "price_hint");
    let price_hkd = price_hint.and_then(|h| parse_price_hint(&h));
    if area.is_none() && price_hkd.is_none() {
        return None;
    }
    Some(CanonicalListing {
        source: DataSource::AaProperty,
        source_record_id: r.record_id.clone(),
        region: None,
        estate: None,
        price_hkd,
        build_area_sqft: area,
        net_area_sqft: None, // ambiguous — can't reliably map to net
        unit_price_net: None, // no net area → can't derive
        is_primary: None,
        tx_date_month: None,
        tx_type: Some("auction".into()),
    })
}

// ---- Field-extraction helpers ----

fn f64_field(r: &NormalizedRecord, name: &str) -> Option<f64> {
    match r.fields.get(name)? {
        RecordValue::Float(f) => Some(*f),
        RecordValue::Int(i) => Some(*i as f64),
        _ => None,
    }
}

fn str_field(r: &NormalizedRecord, name: &str) -> Option<String> {
    match r.fields.get(name)? {
        RecordValue::Str(s) => Some(s.clone()),
        RecordValue::Int(i) => Some(i.to_string()),
        RecordValue::Float(f) => Some(f.to_string()),
        RecordValue::Bool(b) => Some(b.to_string()),
        RecordValue::Null => None,
    }
}

fn bool_field(r: &NormalizedRecord, name: &str) -> Option<bool> {
    match r.fields.get(name)? {
        RecordValue::Bool(b) => Some(*b),
        _ => None,
    }
}

/// Derive per-net-sqft price = price / net_area, when both present and area > 0.
fn derive_unit_price(price: Option<f64>, net_area: Option<f64>) -> Option<f64> {
    let (p, a) = (price?, net_area?);
    if a > 0.0 {
        Some(p / a)
    } else {
        None
    }
}

/// Normalize a region string to "hk"/"kln"/"nt". Case-insensitive; passes
/// through unknown values lowercased.
fn normalize_region(s: String) -> String {
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "hk" | "hong kong" | "香港" => "hk".into(),
        "kln" | "kowloon" | "九龍" | "九龙" => "kln".into(),
        "nt" | "new territories" | "新界" => "nt".into(),
        _ => lower,
    }
}

/// Extract a YYYY-MM from a date-ish string ("2026-06-15", "2026-06",
/// "2026/06/15"). Returns None if no month is parseable.
fn month_from(s: &str) -> Option<String> {
    if s.len() >= 7 {
        let candidate = &s[..7];
        // Accept "YYYY-MM" or "YYYY/MM".
        if candidate.chars().nth(4) == Some('-') || candidate.chars().nth(4) == Some('/') {
            return Some(candidate.replace('/', "-"));
        }
    }
    None
}

/// Try to parse a price-hint string (AA Property's free-text field). Handles
/// "港幣500萬" / "5000000" / "5,000,000" patterns; returns None for
/// non-numeric hints like "歡迎查詢".
fn parse_price_hint(s: &str) -> Option<f64> {
    use std::collections::HashMap;
    // Chinese-unit multipliers.
    let multipliers: HashMap<char, f64> = [
        ('萬', 10_000.0),
        ('万', 10_000.0),
        ('億', 100_000_000.0),
        ('亿', 100_000_000.0),
    ]
    .into_iter()
    .collect();
    // Extract the leading numeric run (digits + optional decimal + commas).
    let mut num_str = String::new();
    let mut unit_mult: Option<f64> = None;
    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' || ch == ',' {
            num_str.push(ch);
        } else if let Some(&m) = multipliers.get(&ch) {
            unit_mult = Some(m);
            break; // the unit terminates the number
        } else if !num_str.is_empty() {
            break; // non-numeric, non-unit after digits → stop
        }
    }
    if num_str.is_empty() {
        return None;
    }
    let n: f64 = num_str.replace(',', "").parse().ok()?;
    Some(n * unit_mult.unwrap_or(1.0))
}

/// A cross-portal divergence finding (M5). Pure-Rust — no IO — so it
/// preserves the determinism guarantee: same listings in → same findings out.
/// Wired into the scan config like the other detectors
/// (`detector = "portal_divergence"`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PortalDivergenceFinding {
    /// The two portals whose medians diverged.
    pub source_a: DataSource,
    pub source_b: DataSource,
    /// The join bucket (e.g. region "kln" + month "2026-06").
    pub region: Option<String>,
    pub month: Option<String>,
    /// The median per-net-sqft price each portal reported for this bucket.
    pub median_a: f64,
    pub median_b: f64,
    /// The percentage gap (|a-b|/min(a,b) × 100).
    pub pct_divergence: f64,
    /// How many listings each portal contributed to its median.
    pub count_a: usize,
    pub count_b: usize,
}

impl PortalDivergenceFinding {
    /// A short summary for the insight body. Mirrors the analysis.rs
    /// heuristic-summary style.
    pub fn summary(&self) -> String {
        let where_clause = match (&self.region, &self.month) {
            (Some(r), Some(m)) => format!("{r} / {m}"),
            (Some(r), None) => r.clone(),
            (None, Some(m)) => format!("all regions / {m}"),
            (None, None) => "all buckets".to_string(),
        };
        format!(
            "{}/{} per-sqft prices diverge by {:.1}% in {where_clause} \
             ({}: {:.0}/sqft from {} listings; {}: {:.0}/sqft from {} listings)",
            self.source_a, self.source_b, self.pct_divergence,
            self.source_a, self.median_a, self.count_a,
            self.source_b, self.median_b, self.count_b,
        )
    }
}

/// The default divergence threshold: portals whose medians differ by more than
/// this percentage fire a finding. 10% — loose enough to catch real
/// methodology/pipeline gaps, tight enough to ignore normal market spread.
pub const DEFAULT_PORTAL_DIVERGENCE_PCT: f64 = 10.0;

/// Minimum listings per portal per bucket for the median to be trustworthy.
/// Below this, a single outlier could swing the median — don't report.
pub const MIN_LISTINGS_FOR_MEDIAN: usize = 3;

/// Detect cross-portal divergence: for each (region, month) bucket where both
/// portals contributed ≥ MIN_LISTINGS_FOR_MEDIAN listings, compare the median
/// per-net-sqft price. Fire a finding when they diverge by more than
/// `max_pct_divergence`.
///
/// `join` controls the bucketing: Region+Month is the default (the most
/// granular meaningful comparison); callers wanting a coarser view can pass
/// Month-only or Region-only via the [`JoinKey`] variants.
///
/// Pure-Rust, no IO. Same listings in → same findings out.
pub fn detect_portal_divergence(
    source_a: DataSource,
    listings_a: &[CanonicalListing],
    source_b: DataSource,
    listings_b: &[CanonicalListing],
    join: JoinKey,
    max_pct_divergence: f64,
) -> Vec<PortalDivergenceFinding> {
    let mut findings = Vec::new();
    let buckets_a = bucket_by_join(listings_a, join);
    let buckets_b = bucket_by_join(listings_b, join);
    // Iterate the union of buckets; only compare where both sides have enough.
    let mut all_keys: Vec<(Option<String>, Option<String>)> = buckets_a.keys().cloned().collect();
    for k in buckets_b.keys() {
        if !all_keys.contains(k) {
            all_keys.push(k.clone());
        }
    }
    for (region, month) in all_keys {
        let Some(vals_a) = buckets_a.get(&(region.clone(), month.clone())) else {
            continue;
        };
        let Some(vals_b) = buckets_b.get(&(region.clone(), month.clone())) else {
            continue;
        };
        if vals_a.len() < MIN_LISTINGS_FOR_MEDIAN || vals_b.len() < MIN_LISTINGS_FOR_MEDIAN {
            continue;
        }
        let med_a = median(vals_a);
        let med_b = median(vals_b);
        if med_a <= 0.0 || med_b <= 0.0 {
            continue;
        }
        let pct = ((med_a - med_b).abs() / med_a.min(med_b)) * 100.0;
        if pct >= max_pct_divergence {
            findings.push(PortalDivergenceFinding {
                source_a,
                source_b,
                region: region.clone(),
                month: month.clone(),
                median_a: med_a,
                median_b: med_b,
                pct_divergence: pct,
                count_a: vals_a.len(),
                count_b: vals_b.len(),
            });
        }
    }
    // Sort by divergence descending so the worst gaps surface first.
    findings.sort_by(|a, b| b.pct_divergence.partial_cmp(&a.pct_divergence).unwrap_or(std::cmp::Ordering::Equal));
    findings
}

/// How to bucket listings for cross-portal comparison.
#[derive(Debug, Clone, Copy)]
pub enum JoinKey {
    /// Bucket by (region, month) — the most granular meaningful comparison.
    RegionAndMonth,
    /// Bucket by month only (aggregate across regions).
    Month,
    /// Bucket by region only (aggregate across months).
    Region,
}

/// Bucket listings by the join key, collecting their per-net-sqft prices.
/// Listings without a unit_price_net are dropped (can't contribute to a median).
fn bucket_by_join(
    listings: &[CanonicalListing],
    join: JoinKey,
) -> std::collections::HashMap<(Option<String>, Option<String>), Vec<f64>> {
    let mut buckets: std::collections::HashMap<(Option<String>, Option<String>), Vec<f64>> =
        std::collections::HashMap::new();
    for l in listings {
        let Some(up) = l.unit_price_net else {
            continue;
        };
        if up <= 0.0 || !up.is_finite() {
            continue;
        }
        let key = match join {
            JoinKey::RegionAndMonth => (l.region.clone(), l.tx_date_month.clone()),
            JoinKey::Month => (None, l.tx_date_month.clone()),
            JoinKey::Region => (l.region.clone(), None),
        };
        buckets.entry(key).or_default().push(up);
    }
    buckets
}

/// Median of a non-empty slice of f64. Returns 0.0 for empty input (the caller
/// guards with MIN_LISTINGS_FOR_MEDIAN).
fn median(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = vals.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Compute a cross-portal market composite: the median per-net-sqft price
/// across all portals for a region/month bucket, with per-portal contribution.
/// This is what an NT-Metropolis planner queries; the per-portal breakdown is
/// the transparency layer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PortalComposite {
    pub region: Option<String>,
    pub month: Option<String>,
    /// The cross-portal median of the per-portal medians.
    pub composite_median_unit_price: Option<f64>,
    /// Per-portal contribution: median, count, and the portal's weight.
    pub by_portal: Vec<PortalContribution>,
    /// Total listings across all portals in this bucket.
    pub total_listings: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PortalContribution {
    pub source: DataSource,
    pub median_unit_price: Option<f64>,
    pub listing_count: usize,
}

/// Build the cross-portal composite for a region/month across the given
/// listing sets (one per portal). Returns the composite + per-portal breakdown.
pub fn build_composite(
    region: Option<&str>,
    month: Option<&str>,
    portal_listings: &[(DataSource, &[CanonicalListing])],
) -> PortalComposite {
    let mut contributions = Vec::new();
    let mut all_medians: Vec<f64> = Vec::new();
    let mut total = 0usize;
    for (source, listings) in portal_listings {
        // Filter to this region/month.
        let filtered: Vec<&CanonicalListing> = listings
            .iter()
            .filter(|l| region.is_none() || l.region.as_deref() == region)
            .filter(|l| month.is_none() || l.tx_date_month.as_deref() == month)
            .filter(|l| l.unit_price_net.is_some())
            .collect();
        let prices: Vec<f64> = filtered
            .iter()
            .filter_map(|l| l.unit_price_net)
            .filter(|p| *p > 0.0 && p.is_finite())
            .collect();
        total += prices.len();
        let med = if prices.is_empty() {
            None
        } else {
            Some(median(&prices))
        };
        if let Some(m) = med {
            all_medians.push(m);
        }
        contributions.push(PortalContribution {
            source: *source,
            median_unit_price: med,
            listing_count: prices.len(),
        });
    }
    let composite = if all_medians.is_empty() {
        None
    } else {
        Some(median(&all_medians))
    };
    PortalComposite {
        region: region.map(String::from),
        month: month.map(String::from),
        composite_median_unit_price: composite,
        by_portal: contributions,
        total_listings: total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::BTreeMap;

    fn rec(id: &str, fields: &[(&str, RecordValue)]) -> NormalizedRecord {
        let mut m = BTreeMap::new();
        for (k, v) in fields {
            m.insert((*k).into(), v.clone());
        }
        NormalizedRecord {
            source: DataSource::Hkp,
            dataset: "test".into(),
            record_id: id.into(),
            fields: m,
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn project_hkp_maps_all_fields() {
        let r = rec(
            "I20260700522",
            &[
                ("price_hkd", RecordValue::Float(8_000_000.0)),
                ("build_area_sqft", RecordValue::Float(500.0)),
                ("net_area_sqft", RecordValue::Float(420.0)),
                ("region", RecordValue::Str("KLN".into())),
                ("estate_name", RecordValue::Str("Whampoa Garden".into())),
                ("mkt_type", RecordValue::Str("2ND".into())),
                ("tx_type", RecordValue::Str("S".into())),
                ("tx_date_iso", RecordValue::Str("2026-06-15".into())),
            ],
        );
        let c = project(&r, DataSource::Hkp).expect("projects");
        assert_eq!(c.price_hkd, Some(8_000_000.0));
        assert_eq!(c.build_area_sqft, Some(500.0));
        assert_eq!(c.net_area_sqft, Some(420.0));
        // unit_price derived = 8_000_000 / 420 ≈ 19047.62
        assert!(c.unit_price_net.unwrap() > 19_000.0);
        assert_eq!(c.region.as_deref(), Some("kln"));
        assert_eq!(c.estate.as_deref(), Some("Whampoa Garden"));
        assert_eq!(c.is_primary, Some(false)); // 2ND
        assert_eq!(c.tx_type.as_deref(), Some("S"));
        assert_eq!(c.tx_date_month.as_deref(), Some("2026-06"));
    }

    #[test]
    fn project_chungsen_converts_price_10k_to_hkd() {
        let r = rec(
            "260612-01",
            &[
                ("price_10k", RecordValue::Int(500)), // HK$5,000,000
                ("build_area_sqft", RecordValue::Int(400)),
                ("saleable_area_sqft", RecordValue::Int(350)),
            ],
        );
        let c = project(&r, DataSource::ChungSen).expect("projects");
        assert_eq!(c.price_hkd, Some(5_000_000.0));
        assert_eq!(c.build_area_sqft, Some(400.0));
        assert_eq!(c.net_area_sqft, Some(350.0));
        // unit_price = 5_000_000 / 350 ≈ 14285.7
        assert!(c.unit_price_net.unwrap() > 14_000.0);
    }

    #[test]
    fn project_aaproperty_returns_none_for_unparseable_price() {
        let r = rec(
            "lot-1",
            &[
                ("price_hint", RecordValue::Str("歡迎查詢".into())),
                ("area_sqft", RecordValue::Int(800)),
            ],
        );
        let c = project(&r, DataSource::AaProperty).expect("projects (has area)");
        assert_eq!(c.price_hkd, None); // honest: can't parse
        assert_eq!(c.build_area_sqft, Some(800.0));
        assert_eq!(c.tx_type.as_deref(), Some("auction"));
    }

    #[test]
    fn project_aaproperty_parses_chinese_million() {
        let r = rec(
            "lot-2",
            &[
                ("price_hint", RecordValue::Str("港幣500萬".into())),
                ("area_sqft", RecordValue::Int(500)),
            ],
        );
        let c = project(&r, DataSource::AaProperty).expect("projects");
        assert_eq!(c.price_hkd, Some(5_000_000.0)); // 500 × 10,000
    }

    #[test]
    fn project_rvd_returns_none() {
        // RVD is a pure time-series index; no listing to project.
        let r = rec("2026-06", &[("all_classes", RecordValue::Float(380.5))]);
        assert!(project(&r, DataSource::Rvd).is_none());
    }

    #[test]
    fn normalize_region_handles_chinese_and_english() {
        assert_eq!(normalize_region("KLN".into()), "kln");
        assert_eq!(normalize_region("九龍".into()), "kln");
        assert_eq!(normalize_region("New Territories".into()), "nt");
        assert_eq!(normalize_region("HK".into()), "hk");
    }

    #[test]
    fn month_from_handles_iso_and_slash_dates() {
        assert_eq!(month_from("2026-06-15"), Some("2026-06".into()));
        assert_eq!(month_from("2026/06/15"), Some("2026-06".into()));
        assert_eq!(month_from("2026-06"), Some("2026-06".into()));
        assert_eq!(month_from("banana"), None);
    }

    // ---- cross-portal divergence + composite ----

    fn listing(source: DataSource, id: &str, region: &str, month: &str, price: f64) -> CanonicalListing {
        CanonicalListing {
            source,
            source_record_id: id.into(),
            region: Some(region.into()),
            estate: None,
            price_hkd: Some(price),
            build_area_sqft: None,
            net_area_sqft: Some(100.0),
            unit_price_net: Some(price / 100.0),
            is_primary: None,
            tx_date_month: Some(month.into()),
            tx_type: None,
        }
    }

    #[test]
    fn detect_portal_divergence_fires_on_30pct_gap() {
        // HKP reports ~10,000/sqft; Midland reports ~15,000/sqft for the same
        // region/month → 50% gap, should fire at the 10% threshold.
        let hkp: Vec<_> = (0..5)
            .map(|i| listing(DataSource::Hkp, &format!("h{i}"), "kln", "2026-06", 1_000_000.0))
            .collect();
        let midland: Vec<_> = (0..5)
            .map(|i| listing(DataSource::Midland, &format!("m{i}"), "kln", "2026-06", 1_500_000.0))
            .collect();
        let findings = detect_portal_divergence(
            DataSource::Hkp,
            &hkp,
            DataSource::Midland,
            &midland,
            JoinKey::RegionAndMonth,
            DEFAULT_PORTAL_DIVERGENCE_PCT,
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].pct_divergence > 40.0);
        assert_eq!(findings[0].count_a, 5);
        assert_eq!(findings[0].count_b, 5);
    }

    #[test]
    fn detect_portal_divergence_quiet_under_threshold() {
        // ~5% gap → under the 10% threshold → no finding.
        let hkp: Vec<_> = (0..5)
            .map(|i| listing(DataSource::Hkp, &format!("h{i}"), "nt", "2026-06", 1_000_000.0))
            .collect();
        let midland: Vec<_> = (0..5)
            .map(|i| listing(DataSource::Midland, &format!("m{i}"), "nt", "2026-06", 1_045_000.0))
            .collect();
        let findings = detect_portal_divergence(
            DataSource::Hkp,
            &hkp,
            DataSource::Midland,
            &midland,
            JoinKey::RegionAndMonth,
            DEFAULT_PORTAL_DIVERGENCE_PCT,
        );
        assert!(findings.is_empty(), "5% gap should not fire at 10% threshold");
    }

    #[test]
    fn detect_portal_divergence_skips_buckets_below_min_listings() {
        // Only 2 listings on one side → below MIN_LISTINGS_FOR_MEDIAN (3).
        let hkp = vec![
            listing(DataSource::Hkp, "h0", "hk", "2026-06", 1_000_000.0),
            listing(DataSource::Hkp, "h1", "hk", "2026-06", 1_000_000.0),
        ];
        let midland: Vec<_> = (0..5)
            .map(|i| listing(DataSource::Midland, &format!("m{i}"), "hk", "2026-06", 5_000_000.0))
            .collect();
        let findings = detect_portal_divergence(
            DataSource::Hkp,
            &hkp,
            DataSource::Midland,
            &midland,
            JoinKey::RegionAndMonth,
            DEFAULT_PORTAL_DIVERGENCE_PCT,
        );
        assert!(findings.is_empty(), "bucket with <3 listings on one side should not fire");
    }

    #[test]
    fn build_composite_merges_portals() {
        let hkp: Vec<_> = (0..3)
            .map(|i| listing(DataSource::Hkp, &format!("h{i}"), "kln", "2026-06", 1_000_000.0))
            .collect();
        let midland: Vec<_> = (0..3)
            .map(|i| listing(DataSource::Midland, &format!("m{i}"), "kln", "2026-06", 1_200_000.0))
            .collect();
        let portals: [(DataSource, &[CanonicalListing]); 2] = [
            (DataSource::Hkp, hkp.as_slice()),
            (DataSource::Midland, midland.as_slice()),
        ];
        let composite = build_composite(Some("kln"), Some("2026-06"), &portals);
        assert!(composite.composite_median_unit_price.is_some());
        assert_eq!(composite.by_portal.len(), 2);
        assert_eq!(composite.total_listings, 6);
    }
}
