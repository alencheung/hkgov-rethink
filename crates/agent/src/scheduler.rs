//! Agent scheduler — runs analysis passes on a timer, decoupled from serving.
//!
//! Each pass:
//! 1. Reads the warmed records for the target dataset(s) from the `RecordStore`.
//! 2. Runs the detectors in [`analysis`].
//! 3. Asks the [`LlmClient`] to frame each finding.
//! 4. Upserts [`Insight`]s into the [`InsightStore`].
//!
//! The scheduler owns no HTTP; it only reads from the store and writes insights.
//! This keeps serving latency untouched even when the LLM client is slow.

use crate::analysis::{detect_cross_source_gaps, detect_series_jumps, Finding};
use crate::insight::InsightStore;
use crate::llm::LlmClient;
use hkgov_common::{DataSource, RecordValue};
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
    pub fn spawn(
        store: Arc<MemoryStore>,
        insights: Arc<InsightStore>,
        llm: Arc<dyn LlmClient>,
        interval: Duration,
    ) -> Self {
        let handle = tokio::spawn(async move {
            run_pass(&store, &insights, &llm).await;
            let mut ticker = tokio::time::interval(interval.max(Duration::from_secs(60)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // skip immediate
            loop {
                ticker.tick().await;
                run_pass(&store, &insights, &llm).await;
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

async fn run_pass(
    store: &Arc<MemoryStore>,
    insights: &Arc<InsightStore>,
    llm: &Arc<dyn LlmClient>,
) {
    tracing::info!(producer = llm.name(), "agent: analysis pass starting");

    let mut all_findings: Vec<Finding> = Vec::new();

    // 1. Series jumps on daily interbank liquidity (HIBOR overnight).
    if let Ok(page) = store
        .get_page(
            &DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity"),
            0,
            500,
        )
        .await
    {
        if !page.records.is_empty() {
            all_findings.extend(detect_series_jumps(
                DataSource::Hkma,
                "daily-interbank-liquidity",
                &page.records,
                "hibor_overnight",
                25.0,
            ));
            all_findings.extend(detect_series_jumps(
                DataSource::Hkma,
                "daily-interbank-liquidity",
                &page.records,
                "closing_balance",
                15.0,
            ));
        }
    }

    // 2. Series jumps on monthly capital market stats.
    if let Ok(page) = store
        .get_page(
            &DatasetId::new(DataSource::Hkma, "capital-market-statistics"),
            0,
            500,
        )
        .await
    {
        if !page.records.is_empty() {
            all_findings.extend(detect_series_jumps(
                DataSource::Hkma,
                "capital-market-statistics",
                &page.records,
                "eq_mkt_hs_index",
                10.0,
            ));
        }
    }

    // 3. Cross-source gap: press release dates vs data dates.
    let mut press_dates = Vec::new();
    if let Ok(page) = store
        .get_page(
            &DatasetId::new(DataSource::Press, "hkma-press-releases"),
            0,
            500,
        )
        .await
    {
        press_dates = page
            .records
            .iter()
            .filter_map(|r| match r.fields.get("date") {
                Some(RecordValue::Str(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();
    }
    let mut data_dates = Vec::new();
    if let Ok(page) = store
        .get_page(
            &DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity"),
            0,
            500,
        )
        .await
    {
        data_dates = page.records.iter().map(|r| r.record_id.clone()).collect();
    }
    if !press_dates.is_empty() {
        all_findings.extend(detect_cross_source_gaps(&press_dates, &data_dates));
    }

    // 4. Frame each finding and store as an Insight.
    let mut stored = 0usize;
    for finding in all_findings {
        match llm.frame(&finding).await {
            Ok(framing) => {
                let insight = finding.into_insight(framing.summary, framing.confidence, llm.name());
                insights.upsert(insight).await;
                stored += 1;
            }
            Err(e) => {
                tracing::warn!(error = %e, kind = %finding.kind, "agent: framing failed, skipping");
            }
        }
    }

    tracing::info!(
        producer = llm.name(),
        stored,
        "agent: analysis pass complete"
    );
}
