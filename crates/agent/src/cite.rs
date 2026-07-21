//! Cite-It (P-101) — citation-grade export & reproducibility manifests.
//!
//! The bridge from "the product found it" → "I published/acted on it". From any
//! insight, generate:
//!
//! - a **stable permalink** (`/{cite,id}`)
//! - **citation strings** in BibTeX / RIS / APA / Chicago / Markdown
//! - a **reproducibility manifest** — the exact recipe (dataset version +
//!   detector + threshold + a content hash) that reproduces the finding in CI
//!
//! ## Determinism
//!
//! The manifest is pure Rust over the insight's own metadata + evidence. Same
//! insight in → same manifest out → byte-identical citation string. No LLM, no
//! API key. The reproducibility hash is the defense against drift: if the
//! upstream data has been revised since the finding was made, the manifest's
//! `data_sha256` won't match a fresh computation, and the cite drawer surfaces a
//! `⚠ reproduces on data as of {ts}` chip rather than false-claiming.
//!
//! ## Design
//!
//! - [`build_citation`] — the top-level entry: takes an `Insight` + the records
//!   it was drawn from, returns a [`Citation`] bundle.
//! - [`CitationFormat`] — the supported reference styles.
//! - [`ReproducibilityManifest`] — the CI recipe + content hash.
//!
//! Persistence of permalinks is a separate concern (the roadmap's Postgres
//! tier). This module produces the *artifact*; the route stores/serves it.

use crate::insight::{EvidenceRef, Insight};
use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Methodology version for the citation format. Bump when the citation
/// templates or manifest fields change.
pub const CITE_VERSION: &str = "1.0";

/// The supported citation/reference formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CitationFormat {
    /// BibTeX — for LaTeX papers.
    Bibtex,
    /// RIS — for reference managers (Zotero, EndNote).
    Ris,
    /// APA 7th.
    Apa,
    /// Chicago (author-date).
    Chicago,
    /// Markdown / plain text.
    Markdown,
}

impl CitationFormat {
    /// All supported formats, in the order the UI renders the tabs.
    pub fn all() -> &'static [CitationFormat] {
        &[
            CitationFormat::Bibtex,
            CitationFormat::Ris,
            CitationFormat::Apa,
            CitationFormat::Chicago,
            CitationFormat::Markdown,
        ]
    }
}

/// A reproducibility manifest — the exact recipe to recompute a finding in CI.
///
/// The `data_sha256` is a content hash over the evidence records (record_id +
/// field + value) used to detect upstream data drift. If a reviewer recomputes
/// the finding against current data and the hash differs, the cite drawer
/// surfaces a warning chip — it never false-claims reproducibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReproducibilityManifest {
    /// Cite-It methodology version.
    pub cite_version: &'static str,
    /// The detector that produced the finding (e.g. "series_jump").
    pub detector: String,
    /// The (source, dataset) the evidence was drawn from.
    pub source: DataSource,
    pub dataset: String,
    /// The numeric threshold the detector was configured with, if known.
    /// `None` when not carried on the insight (e.g. `cross_source_gap`).
    pub threshold: Option<f64>,
    /// Content hash over the evidence (SHA-256 of the canonical evidence
    /// serialization). Detects upstream data drift.
    pub data_sha256: String,
    /// Runtime/software version that produced the finding, for full
    /// reproducibility. `None` when unknown (e.g. a test fixture).
    pub runtime_version: Option<String>,
    /// When the finding was generated.
    pub generated_at: DateTime<Utc>,
}

/// The full citation bundle for one insight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    /// The stable permalink path: `/cite/{insight_id}`.
    pub permalink: String,
    /// The insight id (the permalink target).
    pub insight_id: String,
    /// Cite-It methodology version.
    pub cite_version: &'static str,
    /// The title used in citation strings.
    pub title: String,
    /// Attribution publisher string ("HK City Pulse").
    pub publisher: String,
    /// The year the finding was generated (for citation strings).
    pub year: i32,
    /// When the finding was generated (ISO 8601).
    pub generated_at: DateTime<Utc>,
    /// The reproducibility manifest.
    pub manifest: ReproducibilityManifest,
    /// True when the producing detector is experimental — citation strings
    /// auto-suffix the honesty marker so a researcher cites honestly.
    pub experimental: bool,
}

impl Citation {
    /// Render the citation string for the requested format.
    pub fn render(&self, fmt: CitationFormat) -> String {
        let exp_suffix = if self.experimental {
            " (experimental detector — not yet validated on real data)"
        } else {
            ""
        };
        match fmt {
            CitationFormat::Bibtex => {
                let key = citation_key(self);
                // BibTeX renders the experimental honesty marker in the title
                // field (LaTeX picks it up); fold it in rather than emitting a
                // redundant note.
                let title_with_exp = if self.experimental {
                    format!("{}{exp_suffix}", self.title)
                } else {
                    self.title.clone()
                };
                format!(
                    "@misc{{{key},\n  title = {{{title}}},\n  author = {{{publisher}}},\n  year = {{{year}}},\n  howpublished = {{\\url{{{permalink}}}}},\n  note = {{Reproducibility manifest: data SHA-256 {hash}}}\n}}",
                    key = key,
                    title = title_with_exp,
                    publisher = self.publisher,
                    year = self.year,
                    permalink = self.permalink,
                    hash = short_hash(&self.manifest.data_sha256),
                )
            }
            CitationFormat::Ris => format!(
                "TY  - GEN\nTI  - {title}{exp}\nAU  - {publisher}\nPY  - {year}\nUR  - {permalink}\nN1  - Reproducibility manifest: data SHA-256 {hash}\nER  -\n",
                title = self.title,
                exp = exp_suffix,
                publisher = self.publisher,
                year = self.year,
                permalink = self.permalink,
                hash = short_hash(&self.manifest.data_sha256),
            ),
            CitationFormat::Apa => format!(
                "{publisher}. ({year}). {title}{exp}. {permalink}",
                publisher = self.publisher,
                year = self.year,
                title = self.title,
                exp = exp_suffix,
                permalink = self.permalink,
            ),
            CitationFormat::Chicago => format!(
                "{publisher}, \"{title}{exp},\" accessed {accessed}, {permalink}.",
                publisher = self.publisher,
                title = self.title,
                exp = if self.experimental {
                    ". Experimental detector"
                } else {
                    ""
                },
                accessed = self.generated_at.format("%B %-d, %Y"),
                permalink = self.permalink,
            ),
            CitationFormat::Markdown => format!(
                "[{title}{exp}]({permalink}) — {publisher}, {year}. Reproduces in CI (manifest `{hash}`).",
                title = self.title,
                exp = if self.experimental { " *(experimental)*" } else { "" },
                permalink = self.permalink,
                publisher = self.publisher,
                year = self.year,
                hash = short_hash(&self.manifest.data_sha256),
            ),
        }
    }
}

/// Build a [`Citation`] bundle from an insight + its evidence records.
///
/// `records` is the slice of `NormalizedRecord`s the evidence points into (used
/// to compute the reproducibility content hash). `base_url` is the public origin
/// for the permalink (e.g. `https://hkgov-rethink.example`); the permalink path
/// is `/cite/{insight_id}`.
///
/// `runtime_version` is the software version producing the finding (pass
/// `env!("CARGO_PKG_VERSION")` from the binary; `None` is fine for tests).
pub fn build_citation(
    insight: &Insight,
    records: &[hkgov_common::NormalizedRecord],
    base_url: &str,
    runtime_version: Option<&str>,
) -> Citation {
    let base = base_url.trim_end_matches('/');
    let permalink = format!("{base}/cite/{}", url_encode(&insight.id));
    let data_sha256 = evidence_hash(&insight.evidence, records);
    let threshold = derive_threshold(insight);
    let manifest = ReproducibilityManifest {
        cite_version: CITE_VERSION,
        detector: insight.kind.clone(),
        source: insight.source,
        dataset: insight.dataset.clone(),
        threshold,
        data_sha256,
        runtime_version: runtime_version.map(str::to_string),
        generated_at: insight.generated_at,
    };
    Citation {
        permalink,
        insight_id: insight.id.clone(),
        cite_version: CITE_VERSION,
        title: insight.title.clone(),
        publisher: "HK City Pulse".to_string(),
        year: insight
            .generated_at
            .format("%Y")
            .to_string()
            .parse()
            .unwrap_or(1970),
        generated_at: insight.generated_at,
        manifest,
        experimental: insight.experimental,
    }
}

/// Derive the detector threshold from the insight, when it's recoverable from
/// the evidence. For `series_jump`/`benchmark_deviation`/`threshold_crossing`
/// the threshold is often one of the evidence values; for others it's unknown.
fn derive_threshold(insight: &Insight) -> Option<f64> {
    // The "threshold" evidence ref carries the watch line for threshold_crossing.
    for e in &insight.evidence {
        if e.context.as_deref().unwrap_or("").contains("threshold") {
            if let Some(f) = e.value.as_f64() {
                return Some(f);
            }
        }
    }
    None
}

/// Record_ids that detectors emit as `EvidenceRef.record_id` but that are
/// **not** rows in the source dataset — they carry a detector-derived value in
/// `evidence.value` (a median, a threshold line, a joined history blob). See
/// `analysis.rs`: `"series"` (MAD baseline median), `"threshold"` (a watch
/// line), `"joined_history"` (proxy-divergence companion series).
///
/// `get_by_ids` for these will (correctly) return nothing — they aren't
/// records. The cite route uses this set to tell "a real evidence record went
/// missing from the cache" (a partial set, which must NOT be silently hashed)
/// from "this evidence ref is a derived-value marker" (expected, fine). (A-003.)
pub fn is_synthetic_evidence_id(record_id: &str) -> bool {
    matches!(record_id, "series" | "threshold" | "joined_history")
}

/// Compute the SHA-256 content hash over the evidence refs only (M3
/// provenance). This is the always-available portion — the scheduler has the
/// evidence refs at capture time but not the backing records. It uses the same
/// canonical (sorted) + NaN/Inf-safe serialization as [`evidence_hash`], so for
/// an insight where the records equal the evidence values, this hash is a
/// prefix of the full evidence+records hash.
///
/// Use this when you need a stable, reproducible anchor for an insight's
/// *claimed* evidence (provenance, audit). Use [`evidence_hash`] when you can
/// also supply the backing records (cite manifest) — the latter additionally
/// detects silent upstream data revisions.
pub fn reproducibility_hash(evidence: &[EvidenceRef]) -> String {
    let mut hasher = Sha256::new();
    let mut canonical: Vec<(&str, &str, String)> = evidence
        .iter()
        .map(|e| {
            (
                e.record_id.as_str(),
                e.field.as_str(),
                canonical_json_string(&e.value),
            )
        })
        .collect();
    canonical.sort();
    for (rid, field, val) in canonical {
        hasher.update(rid.as_bytes());
        hasher.update(b"\x00");
        hasher.update(field.as_bytes());
        hasher.update(b"\x00");
        hasher.update(val.as_bytes());
        hasher.update(b"\x00");
    }
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Compute the SHA-256 content hash over the evidence + the records it points
/// into. This is the drift-detection anchor: recompute against current data and
/// compare.
fn evidence_hash(evidence: &[EvidenceRef], records: &[hkgov_common::NormalizedRecord]) -> String {
    let mut hasher = Sha256::new();
    // Hash the evidence in canonical (sorted) order.
    let mut canonical: Vec<(&str, &str, String)> = evidence
        .iter()
        .map(|e| {
            (
                e.record_id.as_str(),
                e.field.as_str(),
                canonical_json_string(&e.value),
            )
        })
        .collect();
    canonical.sort();
    for (rid, field, val) in canonical {
        hasher.update(rid.as_bytes());
        hasher.update(b"\x00");
        hasher.update(field.as_bytes());
        hasher.update(b"\x00");
        hasher.update(val.as_bytes());
        hasher.update(b"\x00");
    }
    // Hash the record_ids + their numeric field values too, so a data revision
    // (same record_id, changed value) is detected even if the evidence list is
    // a subset.
    let mut rec_canonical: Vec<(&str, String)> = records
        .iter()
        .map(|r| (r.record_id.as_str(), canonical_record_values(r)))
        .collect();
    rec_canonical.sort();
    for (rid, vals) in rec_canonical {
        hasher.update(rid.as_bytes());
        hasher.update(b"\x01");
        hasher.update(vals.as_bytes());
        hasher.update(b"\x01");
    }
    let hash = hasher.finalize();
    // Hex-encode.
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Canonical JSON serialization of a serde_json::Value (sorted object keys),
/// so two semantically-equal values hash identically regardless of key order.
fn canonical_json_string(value: &serde_json::Value) -> String {
    let canonical = canonicalize(value);
    // serde_json::to_string is stable for our canonicalized (sorted-key) form
    // for finite numbers. Non-finite f64 (NaN/+inf/-inf) would serialize as
    // `null` by default, silently collapsing three distinct values into one in
    // the hash — so we canonicalize them to distinct tagged strings first.
    // (A-007.)
    canonical_value_to_string(&canonical)
}

/// Like `serde_json::to_string`, but renders non-finite f64 as distinct tagged
/// strings (`"__nan__"`, `"__inf__"`, `"__ninf__"`) instead of collapsing them
/// to `null`. Finite values, ints, bools, strings, arrays, and objects use the
/// normal serde_json form. This keeps the reproducibility hash honest: a
/// corrupt `NaN` upstream value hashes differently from a genuine `null` or
/// from `+inf`, so the drift detector still fires. (A-007.)
fn canonical_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.is_nan() {
                    return "\"__nan__\"".into();
                }
                if f.is_infinite() {
                    return if f > 0.0 {
                        "\"__inf__\"".into()
                    } else {
                        "\"__ninf__\"".into()
                    };
                }
            }
            serde_json::to_string(value).unwrap_or_else(|_| "null".into())
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
    }
}

/// Recursively sort object keys so the hash is order-independent.
fn canonicalize(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Map, Value};
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(String, Value)> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonicalize(v)))
                .collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (k, v) in sorted {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// Stringify a record's numeric fields in a canonical (sorted) form for hashing.
///
/// `RecordValue::Float` is rendered via [`canonical_f64`] rather than
/// `serde_json::to_string`, which maps NaN/Infinity to `null` — collapsing
/// three distinct values into one hash bucket and breaking the drift detector.
/// (A-007.)
fn canonical_record_values(rec: &hkgov_common::NormalizedRecord) -> String {
    let mut pairs: Vec<(&str, String)> = rec
        .fields
        .iter()
        .map(|(k, v)| (k.as_str(), canonical_record_value_string(v)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Stable, NaN/Inf-safe string form of a [`RecordValue`] for hashing. Finite
/// floats use `serde_json`'s shortest-round-trip form (stable within a major
/// version); non-finite floats get distinct tags so they don't collapse to the
/// same hash as `Null`. (A-007.)
fn canonical_record_value_string(v: &hkgov_common::RecordValue) -> String {
    use hkgov_common::RecordValue;
    match v {
        RecordValue::Float(f) => canonical_f64(*f),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

/// Canonical string for an f64 that distinguishes NaN / +Inf / -Inf from each
/// other and from any finite value, instead of serde_json's silent `null`.
fn canonical_f64(f: f64) -> String {
    if f.is_nan() {
        "\"__nan__\"".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "\"__inf__\"".into()
        } else {
            "\"__ninf__\"".into()
        }
    } else {
        // Shortest-round-trip form; stable within a serde_json major version.
        serde_json::to_string(&f).unwrap_or_else(|_| "null".into())
    }
}

/// A short, display-friendly prefix of a SHA-256 (first 12 hex chars).
fn short_hash(full: &str) -> String {
    full.chars().take(12).collect()
}

/// A stable citation key (BibTeX-friendly) derived from the publisher + year +
/// insight id hash.
fn citation_key(c: &Citation) -> String {
    let id_hash: String = c
        .insight_id
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect();
    let trunc: String = id_hash.chars().take(16).collect();
    format!("hkp{}{}", c.year, trunc.to_lowercase())
}

/// Minimal percent-encoding for the insight id in a URL path segment. Encodes
/// everything except unreserved characters so the permalink is safe.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{EvidenceRef, InsightSeverity};
    use chrono::TimeZone;
    use hkgov_common::{DataSource, NormalizedRecord, RecordValue};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn insight(id: &str, kind: &str, experimental: bool) -> Insight {
        Insight {
            id: id.into(),
            kind: kind.into(),
            severity: InsightSeverity::Warning,
            title: format!("Test finding {id}"),
            summary: "s".into(),
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            evidence: vec![
                EvidenceRef {
                    record_id: "2026-04-01".into(),
                    field: "rate".into(),
                    value: json!(1.0),
                    context: Some("previous period".into()),
                },
                EvidenceRef {
                    record_id: "2026-04-15".into(),
                    field: "rate".into(),
                    value: json!(2.0),
                    context: Some("current period".into()),
                },
            ],
            confidence: 0.8,
            generated_at: Utc.with_ymd_and_hms(2026, 6, 22, 12, 0, 0).unwrap(),
            producer: "test".into(),
            experimental,
            first_seen: None,
            version: 1,
            evolution: None,
        }
    }

    fn records() -> Vec<NormalizedRecord> {
        let mk = |id: &str, v: f64| NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            record_id: id.into(),
            fields: {
                let mut m = BTreeMap::new();
                m.insert("rate".into(), RecordValue::Float(v));
                m
            },
            fetched_at: Utc::now(),
        };
        vec![mk("2026-04-01", 1.0), mk("2026-04-15", 2.0)]
    }

    #[test]
    fn permalink_is_stable_and_url_safe() {
        let ins = insight("series_jump:hkma:x:abc/def", "series_jump", false);
        let c = build_citation(&ins, &records(), "https://example.com/", None);
        assert_eq!(
            c.permalink,
            "https://example.com/cite/series_jump%3Ahkma%3Ax%3Aabc%2Fdef"
        );
        // Deterministic: same input → same permalink.
        let c2 = build_citation(&ins, &records(), "https://example.com", None);
        assert_eq!(c.permalink, c2.permalink);
    }

    #[test]
    fn manifest_hash_is_deterministic() {
        let ins = insight("id1", "series_jump", false);
        let a = build_citation(&ins, &records(), "https://x", None);
        let b = build_citation(&ins, &records(), "https://x", None);
        assert_eq!(a.manifest.data_sha256, b.manifest.data_sha256);
        assert_eq!(a.manifest.cite_version, "1.0");
        assert_eq!(a.manifest.detector, "series_jump");
    }

    #[test]
    fn manifest_hash_detects_data_drift() {
        // Same insight, but the underlying record value changed (data revision).
        let ins = insight("id1", "series_jump", false);
        let original = build_citation(&ins, &records(), "https://x", None);
        let mut drifted_recs = records();
        // Revise the value of the 2026-04-15 record from 2.0 → 2.5.
        drifted_recs[1]
            .fields
            .insert("rate".into(), RecordValue::Float(2.5));
        let drifted = build_citation(&ins, &drifted_recs, "https://x", None);
        assert_ne!(
            original.manifest.data_sha256, drifted.manifest.data_sha256,
            "a data revision must change the hash"
        );
    }

    #[test]
    fn manifest_hash_independent_of_evidence_order() {
        let ins = insight("id1", "series_jump", false);
        let mut ins_rev = ins.clone();
        ins_rev.evidence.reverse(); // swap the two evidence refs
        let a = build_citation(&ins, &records(), "https://x", None);
        let b = build_citation(&ins_rev, &records(), "https://x", None);
        assert_eq!(
            a.manifest.data_sha256, b.manifest.data_sha256,
            "evidence order must not change the hash"
        );
    }

    #[test]
    fn renders_all_formats() {
        let ins = insight("id1", "series_jump", false);
        let c = build_citation(&ins, &records(), "https://example.com", None);
        for fmt in CitationFormat::all() {
            let s = c.render(*fmt);
            assert!(!s.is_empty(), "render({fmt:?}) empty");
            assert!(
                s.contains("example.com") || s.contains("hkp"),
                "{fmt:?}: {s}"
            );
        }
    }

    #[test]
    fn bibtex_has_valid_entry() {
        let ins = insight("id1", "series_jump", false);
        let c = build_citation(&ins, &records(), "https://x", None);
        let bib = c.render(CitationFormat::Bibtex);
        assert!(bib.starts_with("@misc{"));
        assert!(bib.contains("title = {"));
        assert!(bib.contains("howpublished"));
    }

    #[test]
    fn experimental_finding_carries_honesty_marker() {
        let ins = insight("id1", "series_jump", true);
        let c = build_citation(&ins, &records(), "https://x", None);
        let apa = c.render(CitationFormat::Apa);
        assert!(apa.contains("experimental"), "APA: {apa}");
        let ris = c.render(CitationFormat::Ris);
        assert!(ris.contains("experimental"), "RIS: {ris}");
    }

    #[test]
    fn threshold_crossing_recovers_threshold() {
        // A threshold_crossing insight carries the watch line in evidence.
        let ins = Insight {
            id: "tc1".into(),
            kind: "threshold_crossing".into(),
            severity: InsightSeverity::Warning,
            title: "crossed".into(),
            summary: "s".into(),
            source: DataSource::Hkma,
            dataset: "x".into(),
            evidence: vec![
                EvidenceRef {
                    record_id: "2026-04-15".into(),
                    field: "rate".into(),
                    value: json!(0.8),
                    context: Some("latest value".into()),
                },
                EvidenceRef {
                    record_id: "threshold".into(),
                    field: "rate".into(),
                    value: json!(1.0),
                    context: Some("watch Below threshold".into()),
                },
            ],
            confidence: 0.9,
            generated_at: Utc::now(),
            producer: "test".into(),
            experimental: false,
            first_seen: None,
            version: 1,
            evolution: None,
        };
        let c = build_citation(&ins, &[], "https://x", None);
        assert_eq!(c.manifest.threshold, Some(1.0));
    }

    #[test]
    fn determinism_full_bundle() {
        let ins = insight("id1", "series_jump", false);
        let a = build_citation(&ins, &records(), "https://x", Some("0.1.0"));
        let b = build_citation(&ins, &records(), "https://x", Some("0.1.0"));
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
        );
    }

    #[test]
    fn citation_key_is_bibtex_safe() {
        let ins = insight("series_jump:hkma:x:abc", "series_jump", false);
        let c = build_citation(&ins, &records(), "https://x", None);
        let bib = c.render(CitationFormat::Bibtex);
        // Key should be hkp2026seriesjumphkmaxabc-ish (alphanumeric only).
        assert!(bib.starts_with("@misc{hkp2026"));
    }

    // ---- A-007: NaN/Inf f64 values must not collapse to the same hash as ----
    // ---- Null or each other in the reproducibility manifest.              ----
    //
    // serde_json maps NaN, +Inf, and -Inf all to `null`. Without the canonical
    // form, a record whose value drifted from `1.0` to `NaN` (an upstream
    // connector bug) would hash identically to a record whose value was
    // deleted (Null) — the drift detector would never fire on the corruption.

    #[test]
    fn canonical_f64_distinguishes_nan_inf_finite_and_null() {
        // The four non-finite / empty cases must each hash distinctly.
        assert_ne!(canonical_f64(f64::NAN), canonical_f64(f64::INFINITY));
        assert_ne!(canonical_f64(f64::NAN), canonical_f64(f64::NEG_INFINITY));
        assert_ne!(
            canonical_f64(f64::INFINITY),
            canonical_f64(f64::NEG_INFINITY)
        );
        // And none of them may equal the canonical form of a Null record value.
        let null_str = canonical_record_value_string(&RecordValue::Null);
        assert_ne!(canonical_f64(f64::NAN), null_str);
        assert_ne!(canonical_f64(f64::INFINITY), null_str);
        // A finite value must still use serde_json's normal form.
        assert_eq!(canonical_f64(1.5), "1.5");
    }

    #[test]
    fn manifest_hash_detects_drift_to_nan() {
        // A record value changing from a finite number to NaN is a data
        // corruption the manifest must catch — not silently hash as Null.
        let ins = insight("id1", "series_jump", false);
        let original = build_citation(&ins, &records(), "https://x", None);
        let mut corrupted = records();
        // Drift 2026-04-15's value 2.0 → NaN (e.g. an upstream divide-by-zero).
        corrupted[1]
            .fields
            .insert("rate".into(), RecordValue::Float(f64::NAN));
        let corrupted_citation = build_citation(&ins, &corrupted, "https://x", None);
        assert_ne!(
            original.manifest.data_sha256, corrupted_citation.manifest.data_sha256,
            "drift from 2.0 to NaN must change the hash (A-007)"
        );
    }

    #[test]
    fn manifest_hash_distinguishes_nan_from_inf() {
        // Two different corruptions (NaN vs +Inf) must hash differently, so a
        // reviewer can tell which kind of corruption occurred.
        let ins = insight("id1", "series_jump", false);
        let mut nan_recs = records();
        nan_recs[1]
            .fields
            .insert("rate".into(), RecordValue::Float(f64::NAN));
        let mut inf_recs = records();
        inf_recs[1]
            .fields
            .insert("rate".into(), RecordValue::Float(f64::INFINITY));
        let nan_c = build_citation(&ins, &nan_recs, "https://x", None);
        let inf_c = build_citation(&ins, &inf_recs, "https://x", None);
        assert_ne!(
            nan_c.manifest.data_sha256, inf_c.manifest.data_sha256,
            "NaN and +Inf must hash differently (A-007)"
        );
    }
}
