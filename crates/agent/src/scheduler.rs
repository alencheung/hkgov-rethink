//! Agent scheduler — runs analysis passes on a timer, decoupled from serving.
//!
//! Each pass:
//! 1. Reads the warmed records for the target dataset(s) from the `RecordStore`.
//! 2. Runs the detectors configured in [`ScanTarget`]s against those records.
//! 3. Asks the [`LlmClient`] to frame each finding.
//! 4. Upserts [`Insight`]s into the [`InsightStore`].
//!
//! The scheduler owns no HTTP; it only reads from the store and writes insights.
//! This keeps serving latency untouched even when the LLM client is slow.
//!
//! What gets scanned is fully config-driven (see `[[agent.scan]]` in
//! `config.toml` / `AgentSettings::scan`). An empty `scan` list falls back to
//! [`default_scan_targets`], so out-of-the-box behavior is unchanged from v3.

use crate::alerts::AlertDispatcher;
use crate::analysis::{
    detect_benchmark_deviation, detect_correlation, detect_cross_source_gaps, detect_outliers,
    detect_proxy_divergence, detect_seasonality, detect_series_jumps_cadenced,
    detect_threshold_crossing, detect_year_over_year, CrossDirection, Finding,
    DEFAULT_CORRELATION_R, DEFAULT_OUTLIER_Z, DEFAULT_PCT_THRESHOLD, DEFAULT_PROXY_DELTA_PCT,
    DEFAULT_PROXY_R, DEFAULT_SEASONALITY_R,
};
use crate::insight::{Insight, InsightStore};
use crate::llm::LlmClient;
use hkgov_common::{
    default_scan_targets, Comparison, DataSource, RecordValue, ScanTarget, Settings,
};
use hkgov_store::{DatasetId, MemoryStore, RecordStore};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

pub struct AgentSupervisor {
    handles: Vec<JoinHandle<()>>,
}

impl AgentSupervisor {
    /// Spawn the agent. Runs immediately once, then on `interval`. Returns
    /// immediately; analysis runs in the background.
    ///
    /// `alerts` is optional: when `Some`, each pass evaluates its new insights
    /// against the dispatcher for proactive push.
    pub fn spawn(
        store: Arc<MemoryStore>,
        insights: Arc<InsightStore>,
        llm: Arc<dyn LlmClient>,
        settings: Arc<Settings>,
        alerts: Option<Arc<AlertDispatcher>>,
        interval: Duration,
    ) -> Self {
        let handle = tokio::spawn(async move {
            run_pass(
                &store,
                &insights,
                llm.as_ref(),
                &settings,
                alerts.as_deref(),
            )
            .await;
            let mut ticker = tokio::time::interval(interval.max(Duration::from_secs(60)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // skip immediate
            loop {
                ticker.tick().await;
                run_pass(
                    &store,
                    &insights,
                    llm.as_ref(),
                    &settings,
                    alerts.as_deref(),
                )
                .await;
            }
        });
        Self {
            handles: vec![handle],
        }
    }

    pub fn abort_all(&self) {
        for h in &self.handles {
            h.abort();
        }
    }
}

/// Resolve the effective scan list: configured targets, or the defaults if the
/// operator left the list empty.
fn effective_scan(settings: &Settings) -> Vec<ScanTarget> {
    if settings.agent.scan.is_empty() {
        default_scan_targets()
    } else {
        settings.agent.scan.clone()
    }
}

async fn run_pass(
    store: &Arc<MemoryStore>,
    insights: &Arc<InsightStore>,
    llm: &dyn LlmClient,
    settings: &Settings,
    alerts: Option<&AlertDispatcher>,
) {
    tracing::info!(producer = llm.name(), "agent: analysis pass starting");

    let scan = effective_scan(settings);
    // Pair each finding with its source-target's `experimental` flag so the
    // promoted Insight can carry it (user-visible honesty).
    let mut all_findings: Vec<(Finding, bool)> = Vec::new();

    for target in &scan {
        let Some(source) = DataSource::parse(&target.source) else {
            tracing::warn!(
                source = %target.source,
                "agent: unknown source in scan target, skipping"
            );
            continue;
        };
        if target.experimental {
            tracing::info!(
                detector = %target.detector,
                source = %target.source,
                dataset = %target.dataset,
                "agent: running EXPERIMENTAL scan target (detector not yet validated on real data; \
                 see EXAMPLES.md)"
            );
        }
        let findings = run_one_target(store, source, target).await;
        for f in findings {
            all_findings.push((f, target.experimental));
        }
    }

    // Frame each finding and store as an Insight.
    let mut stored_insights: Vec<Insight> = Vec::new();
    for (finding, experimental) in all_findings {
        match llm.frame(&finding).await {
            Ok(framing) => {
                let insight = finding.into_insight_experimental(
                    framing.summary,
                    framing.confidence,
                    llm.name(),
                    experimental,
                );
                insights.upsert(insight.clone()).await;
                stored_insights.push(insight);
            }
            Err(e) => {
                tracing::warn!(error = %e, kind = %finding.kind, "agent: framing failed, skipping");
            }
        }
    }

    // Proactive alerting: evaluate the pass's new insights against the
    // dispatcher (severity threshold + dedup handled inside).
    if let Some(dispatcher) = alerts {
        dispatcher.evaluate(&stored_insights).await;
    }

    tracing::info!(
        producer = llm.name(),
        stored = stored_insights.len(),
        "agent: analysis pass complete"
    );
}

/// Run one `ScanTarget`: load the (and, for cross-source/proxy/benchmark
/// detectors, the companion) dataset from the store and dispatch to the named
/// detector.
async fn run_one_target(
    store: &Arc<MemoryStore>,
    source: DataSource,
    target: &ScanTarget,
) -> Vec<Finding> {
    let id = DatasetId::new(source, &target.dataset);

    // Detectors that need a companion dataset.
    match target.detector.as_str() {
        "cross_source_gap" => return run_cross_source_gap(store, source, target).await,
        "proxy_divergence" => return run_proxy_divergence(store, source, target).await,
        "benchmark_deviation" => return run_benchmark_deviation(store, source, target).await,
        _ => {}
    }

    let Ok(page) = store.get_page(&id, 0, 500).await else {
        return Vec::new();
    };
    if page.records.is_empty() {
        return Vec::new();
    }
    let Some(field) = target.field.as_deref() else {
        tracing::warn!(
            detector = %target.detector,
            "agent: detector needs `field`, none set, skipping"
        );
        return Vec::new();
    };
    let threshold = target.threshold.unwrap_or(0.0);

    match target.detector.as_str() {
        "series_jump" => {
            // v7: cadence-aware. If comparison is YoY, delegate to YoY detector.
            if matches!(target.comparison, Comparison::YearOverYear) {
                let ppy = target.cadence.periods_per_year().round() as usize;
                let ppy = ppy.max(1);
                detect_year_over_year(
                    source,
                    &target.dataset,
                    &page.records,
                    field,
                    if threshold > 0.0 {
                        threshold
                    } else {
                        DEFAULT_PCT_THRESHOLD
                    },
                    ppy,
                )
            } else {
                // Cadence-scaled PoP (Unknown cadence → unchanged v3 behavior).
                let t = if threshold > 0.0 { threshold } else { 25.0 };
                detect_series_jumps_cadenced(
                    source,
                    &target.dataset,
                    &page.records,
                    field,
                    t,
                    target.cadence,
                )
            }
        }
        "year_over_year" => {
            let ppy = target.cadence.periods_per_year().round() as usize;
            let ppy = ppy.max(1);
            detect_year_over_year(
                source,
                &target.dataset,
                &page.records,
                field,
                if threshold > 0.0 {
                    threshold
                } else {
                    DEFAULT_PCT_THRESHOLD
                },
                ppy,
            )
        }
        "outlier" => detect_outliers(
            source,
            &target.dataset,
            &page.records,
            field,
            if threshold > 0.0 {
                threshold
            } else {
                DEFAULT_OUTLIER_Z
            },
        ),
        "seasonality" => detect_seasonality(
            source,
            &target.dataset,
            &page.records,
            field,
            if threshold > 0.0 {
                threshold
            } else {
                DEFAULT_SEASONALITY_R
            },
        ),
        "correlation" => {
            let Some(field_b) = target.field_b.as_deref() else {
                tracing::warn!(
                    detector = "correlation",
                    "agent: correlation needs `field_b`, none set, skipping"
                );
                return Vec::new();
            };
            detect_correlation(
                source,
                &target.dataset,
                &page.records,
                field,
                field_b,
                if threshold > 0.0 {
                    threshold
                } else {
                    DEFAULT_CORRELATION_R
                },
            )
        }
        "threshold_crossing" => {
            // v7 wiring (P-102 prerequisite): the detector existed but was
            // unreachable from the scheduler. Required a `direction` on the
            // scan target; default to "above" (the watch-level case).
            let direction = match target.direction.as_deref() {
                Some("below") => CrossDirection::Below,
                _ => CrossDirection::Above,
            };
            detect_threshold_crossing(
                source,
                &target.dataset,
                &page.records,
                field,
                threshold,
                direction,
            )
        }
        other => {
            tracing::warn!(detector = other, "agent: unknown detector, skipping");
            Vec::new()
        }
    }
}

/// `cross_source_gap` compares the dates of one dataset (the press side) against
/// the record_ids of a companion dataset (the data side). `target.field` names
/// the date field on the press side (default `date`).
async fn run_cross_source_gap(
    store: &Arc<MemoryStore>,
    source: DataSource,
    target: &ScanTarget,
) -> Vec<Finding> {
    let Some(companion) = &target.companion else {
        tracing::warn!(
            detector = "cross_source_gap",
            "agent: needs `companion`, none set, skipping"
        );
        return Vec::new();
    };
    let Some(comp_source) = DataSource::parse(&companion.source) else {
        tracing::warn!(
            source = %companion.source,
            "agent: unknown companion source, skipping"
        );
        return Vec::new();
    };

    let date_field = target.field.as_deref().unwrap_or("date");

    // Press side: extract the date field as strings.
    let press_id = DatasetId::new(source, &target.dataset);
    let press_dates: Vec<String> = match store.get_page(&press_id, 0, 500).await {
        Ok(p) => p
            .records
            .iter()
            .filter_map(|r| match r.fields.get(date_field) {
                Some(RecordValue::Str(s)) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    if press_dates.is_empty() {
        return Vec::new();
    }

    // Data side: record_ids ARE the dates for daily series.
    let data_id = DatasetId::new(comp_source, &companion.dataset);
    let data_dates: Vec<String> = match store.get_page(&data_id, 0, 500).await {
        Ok(p) => p.records.iter().map(|r| r.record_id.clone()).collect(),
        Err(_) => return Vec::new(),
    };

    detect_cross_source_gaps(source, &target.dataset, &press_dates, &data_dates)
}

/// Load the primary + companion datasets and run `detect_proxy_divergence`.
/// Requires `target.field`, `target.companion`, `target.companion_field`.
async fn run_proxy_divergence(
    store: &Arc<MemoryStore>,
    source: DataSource,
    target: &ScanTarget,
) -> Vec<Finding> {
    let Some(field) = target.field.as_deref() else {
        tracing::warn!(
            detector = "proxy_divergence",
            "agent: needs `field`, skipping"
        );
        return Vec::new();
    };
    let Some(companion_field) = target.companion_field.as_deref() else {
        tracing::warn!(
            detector = "proxy_divergence",
            "agent: needs `companion_field`, skipping"
        );
        return Vec::new();
    };
    let Some(companion) = &target.companion else {
        tracing::warn!(
            detector = "proxy_divergence",
            "agent: needs `companion`, skipping"
        );
        return Vec::new();
    };
    let Some(comp_source) = DataSource::parse(&companion.source) else {
        tracing::warn!(source = %companion.source, "agent: unknown companion source, skipping");
        return Vec::new();
    };

    let primary = match store
        .get_page(&DatasetId::new(source, &target.dataset), 0, 500)
        .await
    {
        Ok(p) => p.records,
        Err(_) => return Vec::new(),
    };
    let companion_recs = match store
        .get_page(&DatasetId::new(comp_source, &companion.dataset), 0, 500)
        .await
    {
        Ok(p) => p.records,
        Err(_) => return Vec::new(),
    };

    let threshold = target.threshold.unwrap_or(0.0);
    detect_proxy_divergence(
        source,
        &target.dataset,
        field,
        comp_source,
        &companion.dataset,
        companion_field,
        &primary,
        &companion_recs,
        target.join_field.as_deref(),
        if threshold > 0.0 {
            threshold
        } else {
            DEFAULT_PROXY_DELTA_PCT
        },
        DEFAULT_PROXY_R,
    )
}

/// Load the actual + benchmark datasets and run `detect_benchmark_deviation`.
/// Requires `target.field`, `target.companion` (the benchmark dataset), and
/// `target.companion_field` (the benchmark value field).
async fn run_benchmark_deviation(
    store: &Arc<MemoryStore>,
    source: DataSource,
    target: &ScanTarget,
) -> Vec<Finding> {
    let Some(field) = target.field.as_deref() else {
        tracing::warn!(
            detector = "benchmark_deviation",
            "agent: needs `field`, skipping"
        );
        return Vec::new();
    };
    let Some(benchmark_field) = target.companion_field.as_deref() else {
        tracing::warn!(
            detector = "benchmark_deviation",
            "agent: needs `companion_field` (benchmark value), skipping"
        );
        return Vec::new();
    };
    let Some(bench_ref) = &target.companion else {
        tracing::warn!(
            detector = "benchmark_deviation",
            "agent: needs `companion`, skipping"
        );
        return Vec::new();
    };
    let Some(bench_source) = DataSource::parse(&bench_ref.source) else {
        tracing::warn!(source = %bench_ref.source, "agent: unknown benchmark source, skipping");
        return Vec::new();
    };

    let actual = match store
        .get_page(&DatasetId::new(source, &target.dataset), 0, 500)
        .await
    {
        Ok(p) => p.records,
        Err(_) => return Vec::new(),
    };
    let benchmarks = match store
        .get_page(&DatasetId::new(bench_source, &bench_ref.dataset), 0, 500)
        .await
    {
        Ok(p) => p.records,
        Err(_) => return Vec::new(),
    };

    let threshold = target.threshold.unwrap_or(0.0);
    detect_benchmark_deviation(
        source,
        &target.dataset,
        field,
        benchmark_field,
        &actual,
        &benchmarks,
        target.join_field.as_deref(),
        if threshold > 0.0 { threshold } else { 10.0 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::InsightStore;
    use crate::llm::{HeuristicClient, LlmFraming};
    use async_trait::async_trait;
    use hkgov_common::{CompanionRef, Error, NormalizedRecord, RecordValue};
    use std::collections::BTreeMap;

    /// A scripted LLM client that frames every finding with its own summary,
    /// so we can assert that run_pass produced insights without a real model.
    struct CountingClient {
        frames: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmClient for CountingClient {
        fn name(&self) -> &'static str {
            "test"
        }
        async fn frame(&self, _finding: &Finding) -> hkgov_common::Result<LlmFraming> {
            self.frames
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(LlmFraming {
                summary: "test framing".into(),
                confidence: 0.8,
            })
        }
    }

    fn settings_with_scan(scan: Vec<ScanTarget>) -> Settings {
        let mut s = Settings::default();
        s.agent.scan = scan;
        s
    }

    fn rec(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut f = BTreeMap::new();
        f.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            record_id: id.into(),
            fields: f,
            fetched_at: chrono::Utc::now(),
        }
    }

    async fn seed(store: &MemoryStore, records: Vec<NormalizedRecord>) {
        if records.is_empty() {
            return;
        }
        let id = DatasetId::new(records[0].source, records[0].dataset.clone());
        store.put_dataset(&id, records).await.unwrap();
    }

    #[tokio::test]
    async fn run_pass_dispatches_series_jump_target() {
        // A scan target mirroring the v3 default for hibor_overnight.
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "series_jump".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(50.0),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        seed(
            &store,
            vec![
                rec("2026-01", "hibor_overnight", 2.0),
                rec("2026-02", "hibor_overnight", 6.0), // +200% → flagged
            ],
        )
        .await;
        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(CountingClient {
            frames: std::sync::atomic::AtomicUsize::new(0),
        });

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 1);
        let list = insights.list(10).await;
        assert_eq!(list[0].kind, "series_jump");
        assert_eq!(list[0].producer, "test");
    }

    #[tokio::test]
    async fn run_pass_dispatches_outlier_target() {
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "outlier".into(),
            field: Some("v".into()),
            threshold: Some(3.5),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        // Baseline needs real variance for MAD to be > 0.
        let baseline = [9.8_f64, 10.1, 9.9, 10.2, 10.0, 9.7];
        let mut recs: Vec<NormalizedRecord> = baseline
            .iter()
            .enumerate()
            .map(|(i, v)| rec(&format!("2026-{i:02}"), "v", *v))
            .collect();
        recs.push(rec("2026-99", "v", 100.0));
        seed(&store, recs).await;

        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(CountingClient {
            frames: std::sync::atomic::AtomicUsize::new(0),
        });

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert!(insights.count().await >= 1);
        assert_eq!(insights.list(10).await[0].kind, "outlier");
    }

    #[tokio::test]
    async fn run_pass_unknown_detector_is_skipped() {
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "no_such_detector".into(),
            field: Some("v".into()),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        seed(&store, vec![rec("2026-01", "v", 1.0)]).await;
        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 0);
    }

    #[tokio::test]
    async fn run_pass_unknown_source_is_skipped() {
        let scan = vec![ScanTarget {
            source: "nonsense".into(),
            dataset: "x".into(),
            detector: "series_jump".into(),
            field: Some("v".into()),
            threshold: Some(50.0),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 0);
    }

    #[tokio::test]
    async fn cross_source_gap_target_uses_companion() {
        let scan = vec![ScanTarget {
            source: "press".into(),
            dataset: "hkma-press-releases".into(),
            detector: "cross_source_gap".into(),
            field: Some("date".into()),
            companion: Some(CompanionRef {
                source: "hkma".into(),
                dataset: "daily-interbank-liquidity".into(),
            }),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));

        // Press side: two dates, one with no matching data.
        let press = vec![
            NormalizedRecord {
                source: DataSource::Press,
                dataset: "hkma-press-releases".into(),
                record_id: "r1".into(),
                fields: {
                    let mut m = BTreeMap::new();
                    m.insert("date".into(), RecordValue::Str("2026-06-18".into()));
                    m
                },
                fetched_at: chrono::Utc::now(),
            },
            NormalizedRecord {
                source: DataSource::Press,
                dataset: "hkma-press-releases".into(),
                record_id: "r2".into(),
                fields: {
                    let mut m = BTreeMap::new();
                    m.insert("date".into(), RecordValue::Str("2026-06-19".into()));
                    m
                },
                fetched_at: chrono::Utc::now(),
            },
        ];
        store
            .put_dataset(
                &DatasetId::new(DataSource::Press, "hkma-press-releases"),
                press,
            )
            .await
            .unwrap();
        // Data side: only one of those dates.
        store
            .put_dataset(
                &DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity"),
                vec![rec("2026-06-18", "hibor_overnight", 2.0)],
            )
            .await
            .unwrap();

        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 1);
        assert_eq!(insights.list(10).await[0].kind, "cross_source_gap");
    }

    /// Sanity: the error import isn't dead (keeps clippy happy at the
    /// `Error` re-export in the test module scope).
    #[test]
    fn error_type_is_in_scope() {
        let _e = Error::Internal("x".into());
    }

    /// v7 wiring (P-102 prerequisite): `detect_threshold_crossing` was
    /// unreachable from the scheduler — this test proves the match arm now
    /// routes to it. "HIBOR above 2.5" is the flagship signal-subscription
    /// use case, so this MUST produce an insight.
    #[tokio::test]
    async fn threshold_crossing_target_fires_when_above() {
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "threshold_crossing".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(2.5),
            direction: Some("above".into()),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        seed(
            &store,
            vec![
                rec("2026-06-17", "hibor_overnight", 2.0),
                rec("2026-06-18", "hibor_overnight", 2.93), // crosses above 2.5
            ],
        )
        .await;

        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 1);
        assert_eq!(insights.list(10).await[0].kind, "threshold_crossing");
    }

    /// `below` direction: fires when the latest value drops under the watch line.
    #[tokio::test]
    async fn threshold_crossing_target_fires_when_below() {
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "threshold_crossing".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(1.0),
            direction: Some("below".into()),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        seed(
            &store,
            vec![
                rec("2026-06-17", "hibor_overnight", 1.5),
                rec("2026-06-18", "hibor_overnight", 0.8), // drops below 1.0
            ],
        )
        .await;

        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 1);
        assert_eq!(insights.list(10).await[0].kind, "threshold_crossing");
    }

    /// No crossing → no insight (the detector's negative-case contract).
    #[tokio::test]
    async fn threshold_crossing_target_silent_when_not_crossed() {
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "threshold_crossing".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(5.0), // far above the data
            direction: Some("above".into()),
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        seed(&store, vec![rec("2026-06-18", "hibor_overnight", 2.0)]).await;

        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 0);
    }

    /// Direction defaults to "above" when unset (backward compat).
    #[tokio::test]
    async fn threshold_crossing_defaults_to_above() {
        let scan = vec![ScanTarget {
            source: "hkma".into(),
            dataset: "daily-interbank-liquidity".into(),
            detector: "threshold_crossing".into(),
            field: Some("hibor_overnight".into()),
            threshold: Some(2.5),
            // direction deliberately unset
            ..Default::default()
        }];
        let settings = settings_with_scan(scan);

        let store = Arc::new(MemoryStore::new(100, 60));
        seed(
            &store,
            vec![rec("2026-06-18", "hibor_overnight", 3.0)], // above 2.5
        )
        .await;

        let insights = Arc::new(InsightStore::new());
        let llm = Arc::new(HeuristicClient::new());

        run_pass(&store, &insights, llm.as_ref(), &settings, None).await;
        assert_eq!(insights.count().await, 1);
    }
}
