//! Transparency report generator — M6.
//!
//! Turns a Silence/Transparency Index into a citable quarterly report artifact:
//! the score, the signal breakdown, the top contributing insights (each with
//! its M3 provenance + a cite permalink), and the methodology version. Ships as
//! Markdown + JSON now; `format=pdf-data` returns the payload a renderer
//! consumes (no PDF rendering in scope — see plan guardrails).

use crate::cite::CITE_VERSION;
use crate::insight::{Insight, InsightStore};
use crate::provenance::ProvenanceStore;
use crate::silence::{source_label, SilenceIndex, SilenceSignalKind};
use crate::transparency::build_index_from_registry;
use chrono::{DateTime, Utc};
use hkgov_common::DataSource;
use serde::Serialize;
use std::sync::Arc;

/// The default publisher name for the report. Parameterizable so other
/// institutions can re-brand (the cite.rs hard-coded "HK City Pulse" blocks
/// adoption; this default is institution-neutral).
pub const DEFAULT_PUBLISHER: &str = "Hong Kong Data Transparency Index";

/// One contributing insight in the report, with its provenance + cite permalink.
#[derive(Debug, Clone, Serialize)]
pub struct ReportInsight {
    pub insight_id: String,
    pub kind: String,
    pub source: DataSource,
    pub dataset: String,
    pub title: String,
    pub severity: String,
    pub summary: String,
    pub confidence: f64,
    pub permalink: String,
    /// The M3 provenance: detector, evidence hash, deterministic flag.
    pub provenance: Option<crate::provenance::ProvenanceRecord>,
}

/// The full quarterly transparency report.
#[derive(Debug, Clone, Serialize)]
pub struct TransparencyReport {
    pub report_version: &'static str,
    pub publisher: String,
    pub methodology_version: String,
    pub cite_version: &'static str,
    pub source: DataSource,
    pub source_label: String,
    pub period: String,
    pub score: f64,
    pub raw_score: f64,
    pub total_events: usize,
    pub signal_breakdown: Vec<ReportSignal>,
    pub top_insights: Vec<ReportInsight>,
    pub generated_at: DateTime<Utc>,
}

pub const REPORT_VERSION: &str = "1.0";

/// One signal row in the report, with a human-readable label.
#[derive(Debug, Clone, Serialize)]
pub struct ReportSignal {
    pub kind: String,
    pub label: String,
    pub count: usize,
    pub weight: f64,
    pub contribution: f64,
}

/// Shaping options for a transparency report, grouped so `build_report` stays
/// under clippy's argument-count threshold. Construct via [`ReportOptions::new`]
/// and chain builder methods, or build inline.
#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub source: DataSource,
    pub period: String,
    /// Public origin for cite permalinks.
    pub base_url: String,
    /// Institution name for the report header.
    pub publisher: String,
    /// Max contributing insights to list.
    pub top_n: usize,
}

impl ReportOptions {
    pub fn new(source: DataSource, period: impl Into<String>) -> Self {
        Self {
            source,
            period: period.into(),
            base_url: "http://localhost:8080".to_string(),
            publisher: DEFAULT_PUBLISHER.to_string(),
            top_n: 10,
        }
    }
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
    pub fn publisher(mut self, name: impl Into<String>) -> Self {
        self.publisher = name.into();
        self
    }
    pub fn top_n(mut self, n: usize) -> Self {
        self.top_n = n;
        self
    }
}

/// Build a transparency report from the held insights + provenance.
/// `insights`/`provenance` are the live stores; `opts` shapes the report
/// (source, period, base_url, publisher, top_n); `now` is injected for
/// deterministic timestamps.
pub async fn build_report(
    insights: &Arc<InsightStore>,
    provenance: &Arc<ProvenanceStore>,
    opts: &ReportOptions,
    now: DateTime<Utc>,
) -> TransparencyReport {
    let registry = crate::transparency::default_registry();
    let snapshot = insights.snapshot().await;
    let all = &snapshot.insights;
    let index: SilenceIndex =
        build_index_from_registry(all, opts.source, &opts.period, now, &registry);

    // Map signal kinds to human-readable labels.
    let signal_breakdown: Vec<ReportSignal> = index
        .signals
        .iter()
        .map(|s| ReportSignal {
            kind: format!("{:?}", s.kind).to_lowercase(),
            label: signal_label(s.kind).to_string(),
            count: s.count,
            weight: s.weight,
            contribution: s.contribution,
        })
        .collect();

    // Top contributing insights: the in-period insights for this source,
    // ranked by severity (critical > warning > info) then confidence.
    let mut in_period: Vec<&Insight> = all
        .iter()
        .filter(|i| i.source == opts.source && crate::silence::insight_in_period(i, &opts.period))
        .collect();
    in_period.sort_by(|a, b| {
        severity_rank(&a.severity)
            .cmp(&severity_rank(&b.severity))
            .then_with(|| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut top: Vec<ReportInsight> = Vec::new();
    for i in in_period.iter().take(opts.top_n) {
        let provenance = provenance.get(&i.id).await;
        // Cite permalink (best-effort; base_url is the public origin).
        let permalink = format!(
            "{}/cite/{}",
            opts.base_url.trim_end_matches('/'),
            url_encoded(&i.id)
        );
        top.push(ReportInsight {
            insight_id: i.id.clone(),
            kind: i.kind.clone(),
            source: i.source,
            dataset: i.dataset.clone(),
            title: i.title.clone(),
            severity: format!("{:?}", i.severity).to_lowercase(),
            summary: i.summary.clone(),
            confidence: i.confidence,
            permalink,
            provenance,
        });
    }

    TransparencyReport {
        report_version: REPORT_VERSION,
        publisher: opts.publisher.clone(),
        methodology_version: index.methodology_version,
        cite_version: CITE_VERSION,
        source: index.source,
        source_label: source_label(index.source).to_string(),
        period: index.period.clone(),
        score: index.score,
        raw_score: index.raw_score,
        total_events: index.total_events,
        signal_breakdown,
        top_insights: top,
        generated_at: now,
    }
}

/// Render the report as Markdown — the citable, human-readable form.
pub fn render_markdown(report: &TransparencyReport) -> String {
    let mut md = String::new();
    md.push_str(&format!(
        "# {} Transparency Report — {}\n\n",
        report.source_label, report.period
    ));
    md.push_str(&format!(
        "*Publisher: {} · Methodology v{} · Cite v{} · Report v{} · Generated {}*\n\n",
        report.publisher,
        report.methodology_version,
        report.cite_version,
        report.report_version,
        report.generated_at.format("%Y-%m-%d"),
    ));
    md.push_str(&format!(
        "## Opacity score: **{:.1}** / 100\n\n",
        report.score
    ));
    md.push_str(&format!(
        "Higher = more opaque. Raw weighted sum: {:.1}. Total opacity events: {}.\n\n",
        report.raw_score, report.total_events
    ));
    md.push_str("## Signal breakdown\n\n");
    md.push_str("| Signal | Count | Weight | Contribution |\n");
    md.push_str("|---|---:|---:|---:|\n");
    for s in &report.signal_breakdown {
        md.push_str(&format!(
            "| {} | {} | {:.1} | {:.1} |\n",
            s.label, s.count, s.weight, s.contribution
        ));
    }
    md.push_str("\n## Top contributing insights\n\n");
    if report.top_insights.is_empty() {
        md.push_str("_No in-period insights recorded for this source._\n");
    } else {
        for (i, ins) in report.top_insights.iter().enumerate() {
            let pct = format!("{:.0}%", ins.confidence * 100.0);
            md.push_str(&format!(
                "{}. **[{}] {}** ({} · confidence {})\n   {}\n   {}\n",
                i + 1,
                ins.severity,
                ins.title,
                ins.kind,
                pct,
                ins.summary,
                ins.permalink,
            ));
            if let Some(p) = &ins.provenance {
                md.push_str(&format!(
                    "   *Provenance: detector `{}` v{}, evidence hash `{}`, deterministic: {}.*\n",
                    p.detector,
                    p.detector_version,
                    &p.input_sha256[..16],
                    p.deterministic
                ));
            }
        }
    }
    md.push_str(
        "\n---\n*This report is reproducible: same insights in → same report out. \
                 Each insight carries a provenance record attesting its detector + evidence hash; \
                 the cite permalinks resolve to CI-reproducible manifests.*\n",
    );
    md
}

/// Severity rank for sorting (critical first).
fn severity_rank(s: &crate::insight::InsightSeverity) -> u8 {
    use crate::insight::InsightSeverity;
    match s {
        InsightSeverity::Critical => 0,
        InsightSeverity::Warning => 1,
        InsightSeverity::Info => 2,
    }
}

/// Human-readable label for a signal kind.
fn signal_label(k: SilenceSignalKind) -> &'static str {
    match k {
        SilenceSignalKind::PressOnlyGap => "Press release with no data row",
        SilenceSignalKind::DataOnlyGap => "Data row with no press release",
        SilenceSignalKind::UnattributedJump => "Unattributed series move",
        SilenceSignalKind::MissingDataDay => "Missing data day",
    }
}

/// Minimal URL-encoding for the insight id in the permalink (the cite route
/// decodes it). Mirrors `cite.rs`'s permalink construction.
fn url_encoded(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u8)
            }
        })
        .collect()
}
