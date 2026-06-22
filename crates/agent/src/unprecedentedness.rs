//! Unprecedentedness (P-103) — the historical-context layer.
//!
//! Every numeric finding gets a rarity read: how unusual is this value against
//! its own history? This converts a flat "moved 99%" into "a 1-in-N-year event;
//! last exceeded ___", which is what an analyst (Marcus), a researcher (Chen),
//! and a journalist (Maya) actually need to decide whether to act.
//!
//! ## Determinism
//!
//! This module is pure Rust over already-stored records. It composes from the
//! same median/MAD math the `outlier` detector uses (see `analysis.rs`), so the
//! determinism guarantee holds: same history in → same rarity out, no LLM, no
//! API key. The LLM only *frames*; detection stays deterministic.
//!
//! ## Design
//!
//! - [`percentile_rank`] — what % of history is at-or-below the current value.
//! - [`normal_range`] — the median ± k·MAD envelope ("normal band").
//! - [`one_in_n`] — the expected return period (e.g. "1-in-8-quarter" event).
//! - [`last_exceeded`] — the most recent prior period that crossed the band.
//! - [`Unprecedentedness`] — the full bundle, serializable for the API surface.
//!
//! History-window default is 90 days of records (configurable by the caller).
//! Below [`MIN_HISTORY_POINTS`] the band is undefined and the API surfaces a
//! "not enough history yet" line rather than a misleading envelope.

use chrono::{DateTime, Utc};
use hkgov_common::{NormalizedRecord, RecordValue};
use serde::{Deserialize, Serialize};

/// Minimum history before we'll opine on rarity. Below this the band is hidden
/// and the card shows a muted "not enough history yet (n=…)" line.
pub const MIN_HISTORY_POINTS: usize = 12;

/// Default multiplier on the MAD for the "normal range" envelope. 3.5 matches
/// the `outlier` detector's default robust-z threshold, so a value flagged by
/// `outlier` will also fall outside the band here — the two views agree by
/// construction.
pub const DEFAULT_BAND_K: f64 = 3.5;

/// A normal-range band: the median ± k·MAD envelope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NormalRange {
    pub low: f64,
    pub median: f64,
    pub high: f64,
}

/// The full rarity read for one numeric value against its history.
///
/// Serialized straight to the API surface and rendered as the
/// `UnprecedentednessBand` on the insight card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unprecedentedness {
    /// The value being scored.
    pub value: f64,
    /// Percentile rank of `value` within history (0–100, higher = more extreme
    /// high). `None` when history < [`MIN_HISTORY_POINTS`].
    pub percentile: Option<f64>,
    /// The normal-range band. `None` when history < [`MIN_HISTORY_POINTS`] or
    /// MAD is zero (flat series).
    pub band: Option<NormalRange>,
    /// Expected return period: "this is a 1-in-N event" for the window's
    /// cadence. `None` when undefined.
    pub one_in_n: Option<u64>,
    /// Historical minimum and maximum in the window (always available when
    /// there's any history).
    pub hist_min: Option<f64>,
    pub hist_max: Option<f64>,
    /// Number of points used to compute the read.
    pub n: usize,
    /// The most recent prior period whose value fell outside the band (the
    /// "last time this happened" comparator). `None` if none in the window or
    /// the band is undefined.
    pub last_exceeded: Option<LastExceeded>,
}

/// A prior period that exceeded the normal band — the comparator link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastExceeded {
    pub record_id: String,
    pub value: f64,
    pub when: Option<DateTime<Utc>>,
    /// Signed percentage by which it exceeded the band edge — lets the UI say
    /// "+112% (bigger)" or "−3% (smaller)".
    pub pct_beyond_edge: f64,
}

impl Unprecedentedness {
    /// True when the band is defined and `value` falls outside it — the
    /// "actually unprecedented" predicate.
    pub fn is_unprecedented(&self) -> bool {
        match self.band {
            Some(b) => self.value < b.low || self.value > b.high,
            None => false,
        }
    }
}

/// Compute the percentile rank of `value` within `history` (0–100). Uses a
/// linear-interpolation rank so ties don't collapse to a step. Returns `None`
/// for empty history.
pub fn percentile_rank(history: &[f64], value: f64) -> Option<f64> {
    if history.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = history.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    // Count strictly-below and equal, then interpolate.
    let below = sorted.iter().filter(|&&v| v < value).count() as f64;
    let equal = sorted
        .iter()
        .filter(|&&v| (v - value).abs() < f64::EPSILON)
        .count() as f64;
    let n = sorted.len() as f64;
    // Mid-rank for the ties.
    let rank = below + (equal - 1.0) / 2.0 + 1.0;
    Some((rank / n) * 100.0)
}

/// Compute the normal-range band (median ± k·MAD). Returns `None` when MAD is
/// zero (flat series) or history is empty.
pub fn normal_range(history: &[f64], k: f64) -> Option<NormalRange> {
    if history.is_empty() {
        return None;
    }
    let med = median(history);
    let abs_devs: Vec<f64> = history.iter().map(|v| (v - med).abs()).collect();
    let mad = median(&abs_devs);
    if mad < f64::EPSILON {
        return None;
    }
    let half = k * mad;
    Some(NormalRange {
        low: med - half,
        median: med,
        high: med + half,
    })
}

/// Expected return period ("1-in-N") for a value this extreme. Approximates
/// the proportion of history at-or-beyond this value's deviation from the
/// median, then converts to a return period. Returns `None` when undefined.
pub fn one_in_n(history: &[f64], value: f64) -> Option<u64> {
    let med = median(history);
    let dev = (value - med).abs();
    let at_or_beyond = history.iter().filter(|&&v| (v - med).abs() >= dev).count();
    if at_or_beyond == 0 {
        // More extreme than anything in history — at least 1-in-(n+1).
        return Some(history.len() as u64 + 1);
    }
    // at_least_one: avoid divide-by-zero (guarded above).
    let rate = at_or_beyond as f64 / history.len() as f64;
    let n = (1.0 / rate).round() as u64;
    if n == 0 {
        Some(1)
    } else {
        Some(n)
    }
}

/// Find the most recent prior record whose value falls outside the band.
/// `records` should be sorted ascending by period; `field` is the numeric
/// field to read. The "current" record (the last one) is excluded — we want a
/// *prior* exceedance to compare against.
pub fn last_exceeded(
    records: &[NormalizedRecord],
    field: &str,
    band: NormalRange,
) -> Option<LastExceeded> {
    let series = numeric_series(records, field);
    if series.len() < 2 {
        return None;
    }
    // Walk back from the second-to-last, looking for an out-of-band point.
    for (id, v, when) in series.iter().rev().skip(1) {
        if *v < band.low || *v > band.high {
            let edge = if *v > band.high { band.high } else { band.low };
            let pct = if edge.abs() > f64::EPSILON {
                ((v - edge) / edge.abs()) * 100.0
            } else {
                0.0
            };
            return Some(LastExceeded {
                record_id: id.to_string(),
                value: *v,
                when: *when,
                pct_beyond_edge: pct,
            });
        }
    }
    None
}

/// The top-level entry point: compute the full [`Unprecedentedness`] read for
/// `value` against `history` and the optional prior-exceedance search.
///
/// `k` is the band multiplier (defaults to [`DEFAULT_BAND_K`]); `records` is
/// the same window used for the last-exceeded search (may be empty if the
/// caller only has the bare history).
pub fn score(
    value: f64,
    history: &[f64],
    records: &[NormalizedRecord],
    field: &str,
    k: f64,
) -> Unprecedentedness {
    let n = history.len();
    let percentile = if n >= MIN_HISTORY_POINTS {
        percentile_rank(history, value)
    } else {
        None
    };
    let band = if n >= MIN_HISTORY_POINTS {
        normal_range(history, if k > 0.0 { k } else { DEFAULT_BAND_K })
    } else {
        None
    };
    let one_in_n = if n >= MIN_HISTORY_POINTS {
        one_in_n(history, value)
    } else {
        None
    };
    let (hist_min, hist_max) = if history.is_empty() {
        (None, None)
    } else {
        let lo = history.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = history.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (Some(lo), Some(hi))
    };
    let last = band.and_then(|b| last_exceeded(records, field, b));
    Unprecedentedness {
        value,
        percentile,
        band,
        one_in_n,
        hist_min,
        hist_max,
        n,
        last_exceeded: last,
    }
}

// ---------------------------------------------------------------------------
// Shared math helpers (mirrors of the private ones in analysis.rs; kept local
// so this module is independently testable without exposing analysis internals).
// ---------------------------------------------------------------------------

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Extract (record_id, value, fetched_at) for a numeric field, sorted ascending
/// by record_id. Mirrors `analysis::numeric_series` but also carries the
/// timestamp so the comparator can surface a date.
fn numeric_series<'a>(
    records: &'a [NormalizedRecord],
    field: &str,
) -> Vec<(&'a str, f64, Option<DateTime<Utc>>)> {
    let mut out: Vec<(&'a str, f64, Option<DateTime<Utc>>)> = records
        .iter()
        .filter_map(|r| {
            let v = match r.fields.get(field)? {
                RecordValue::Float(f) => *f,
                RecordValue::Int(i) => *i as f64,
                _ => return None,
            };
            Some((r.record_id.as_str(), v, Some(r.fetched_at)))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hkgov_common::DataSource;
    use std::collections::BTreeMap;

    fn rec(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut f = BTreeMap::new();
        f.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields: f,
            fetched_at: Utc::now(),
        }
    }

    // ---- percentile_rank --------------------------------------------------

    #[test]
    fn percentile_of_max_is_100() {
        let h = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile_rank(&h, 5.0), Some(100.0));
    }

    #[test]
    fn percentile_of_min_is_low() {
        let h = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        // 1.0 is the smallest → rank 1 of 5 → 20%.
        assert_eq!(percentile_rank(&h, 1.0), Some(20.0));
    }

    #[test]
    fn percentile_median_is_50() {
        let h = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile_rank(&h, 3.0), Some(60.0));
    }

    #[test]
    fn percentile_empty_is_none() {
        assert_eq!(percentile_rank(&[], 1.0), None);
    }

    // ---- normal_range -----------------------------------------------------

    #[test]
    fn band_brackets_median() {
        let h = vec![9.8, 10.0, 10.2, 9.9, 10.1, 10.3, 9.7, 10.4];
        let b = normal_range(&h, 3.5).expect("band");
        assert!(b.low < b.median && b.median < b.high);
        assert!((b.median - 10.05).abs() < 0.01, "median ~10.05, got {b:?}");
    }

    #[test]
    fn band_none_for_flat_series() {
        // MAD == 0 → undefined.
        let h = vec![5.0; 8];
        assert!(normal_range(&h, 3.5).is_none());
    }

    // ---- one_in_n ---------------------------------------------------------

    #[test]
    fn one_in_n_extreme_value() {
        // A value more extreme than all of history → at least 1-in-(n+1).
        let h = vec![1.0, 2.0, 3.0, 4.0];
        let median = 2.5;
        let extreme = 100.0; // dev 97.5 > all
        let n = one_in_n(&h, extreme).expect("some");
        // Nothing in history is as far from median → n+1 = 5.
        assert_eq!(n, 5, "median={median}");
    }

    #[test]
    fn one_in_n_typical_value() {
        // A value exactly at the median: every point is at least as extreme.
        let h = vec![1.0, 2.0, 3.0, 4.0];
        let median = median(&h); // 2.5
        let n = one_in_n(&h, median).expect("some");
        // All 4 points are at-or-beyond dev 0 → rate 1.0 → 1-in-1.
        assert_eq!(n, 1);
    }

    // ---- last_exceeded ----------------------------------------------------

    #[test]
    fn last_exceeded_finds_prior_spike() {
        // History with an in-band run and one spike at p07 (genuinely *prior* —
        // before the last/current point p13). The current point p13 is in-band
        // and excluded from the search by the `skip(1)`.
        let mut recs: Vec<NormalizedRecord> = Vec::new();
        for i in 0..14 {
            let val = if i == 7 { 100.0 } else { 10.0 };
            recs.push(rec(&format!("p{i:02}"), "v", val));
        }
        let band = NormalRange {
            low: 5.0,
            median: 10.0,
            high: 15.0,
        };
        let last = last_exceeded(&recs, "v", band).expect("some");
        // p07 is the only out-of-band prior; it's before p13 (the last), so it's found.
        assert_eq!(last.record_id, "p07");
        assert!(last.pct_beyond_edge > 0.0);
    }

    #[test]
    fn last_exceeded_none_when_all_in_band() {
        let recs: Vec<NormalizedRecord> = (0..14)
            .map(|i| rec(&format!("p{i:02}"), "v", 10.0))
            .collect();
        let band = NormalRange {
            low: 5.0,
            median: 10.0,
            high: 15.0,
        };
        assert!(last_exceeded(&recs, "v", band).is_none());
    }

    // ---- score (top-level) ------------------------------------------------

    #[test]
    fn score_marks_extreme_value_unprecedented() {
        // 12 in-band points + a current spike of 100. MAD of an all-10 series is
        // 0 → band undefined → use a varied history instead.
        let varied: Vec<f64> = vec![
            9.5, 10.0, 9.8, 10.2, 10.1, 9.9, 10.3, 9.7, 10.1, 9.9, 10.2, 9.8,
        ];
        let varied_recs: Vec<NormalizedRecord> = varied
            .iter()
            .enumerate()
            .map(|(i, v)| rec(&format!("p{i:02}"), "v", *v))
            .collect();
        let u = score(100.0, &varied, &varied_recs, "v", DEFAULT_BAND_K);
        assert!(u.is_unprecedented(), "should be unprecedented: {u:?}");
        assert!(u.band.is_some());
        assert!(u.one_in_n.unwrap() > 1);
    }

    #[test]
    fn score_returns_none_band_below_min_history() {
        // Only 8 points (< MIN_HISTORY_POINTS=12) → band/percentile None.
        let h: Vec<f64> = (0..8).map(|i| i as f64).collect();
        let recs: Vec<NormalizedRecord> = h
            .iter()
            .enumerate()
            .map(|(i, v)| rec(&format!("p{i:02}"), "v", *v))
            .collect();
        let u = score(5.0, &h, &recs, "v", DEFAULT_BAND_K);
        assert_eq!(u.n, 8);
        assert!(u.percentile.is_none());
        assert!(u.band.is_none());
        assert!(u.one_in_n.is_none());
        // But min/max are still available.
        assert_eq!(u.hist_min, Some(0.0));
        assert_eq!(u.hist_max, Some(7.0));
    }

    #[test]
    fn score_is_deterministic_across_calls() {
        let h: Vec<f64> = vec![
            9.5, 10.0, 9.8, 10.2, 10.1, 9.9, 10.3, 9.7, 10.1, 9.9, 10.2, 9.8,
        ];
        let recs: Vec<NormalizedRecord> = h
            .iter()
            .enumerate()
            .map(|(i, v)| rec(&format!("p{i:02}"), "v", *v))
            .collect();
        let a = score(10.5, &h, &recs, "v", DEFAULT_BAND_K);
        let b = score(10.5, &h, &recs, "v", DEFAULT_BAND_K);
        // Serialize both and compare bytes — the determinism guarantee.
        let sa = serde_json::to_string(&a).unwrap();
        let sb = serde_json::to_string(&b).unwrap();
        assert_eq!(sa, sb);
    }
}
