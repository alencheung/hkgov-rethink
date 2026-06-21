//! Proactive alerting (v6).
//!
//! When the agent supervisor produces new insights, an [`AlertDispatcher`]
//! decides which ones are worth pushing (by severity + dedup) and fans them out
//! to every registered [`AlertSink`]. The only built-in sink is
//! [`WebhookSink`] (POST JSON to a URL), behind the `alerts` feature so the
//! default build needs no HTTP for alerting either.
//!
//! Severity ordering: `info < warning < critical`. The dispatcher only pushes
//! insights at or above `AlertSettings::min_severity`, and only insights it
//! hasn't dispatched before (dedup by insight id).
//!
//! An [`AlertLog`] records what was dispatched and the outcome, exposed via
//! `GET /v1/alerts` for ops visibility.

use crate::insight::{Insight, InsightSeverity};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hkgov_common::{AlertSettings, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(feature = "alerts")]
use {crate::alerts_webhook_deps::build_webhook_client, serde_json::Value, std::time::Duration};

/// What every alert sink must do. Implementations are fire-and-forget: a
/// delivery failure is logged in the [`AlertLog`] but does not abort the fan-out
/// to other sinks.
#[async_trait]
pub trait AlertSink: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn dispatch(&self, insight: &Insight) -> Result<()>;
}

/// Severity rank for threshold comparison: higher = more severe.
fn severity_rank(s: &InsightSeverity) -> u8 {
    match s {
        InsightSeverity::Info => 0,
        InsightSeverity::Warning => 1,
        InsightSeverity::Critical => 2,
    }
}

/// Parse the configured `min_severity` string into an `InsightSeverity`.
fn parse_severity(s: &str) -> InsightSeverity {
    match s.to_ascii_lowercase().as_str() {
        "critical" => InsightSeverity::Critical,
        "warning" => InsightSeverity::Warning,
        _ => InsightSeverity::Info,
    }
}

/// One entry in the alert dispatch log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertLogEntry {
    pub insight_id: String,
    pub insight_kind: String,
    pub severity: String,
    pub sink: String,
    /// "ok" or the error message.
    pub status: String,
    pub dispatched_at: DateTime<Utc>,
}

/// Ring-buffer-ish log of recent dispatches, queryable via `GET /v1/alerts`.
#[derive(Default)]
pub struct AlertLog {
    entries: Mutex<Vec<AlertLogEntry>>,
    cap: usize,
}

impl AlertLog {
    pub fn new(cap: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            cap: cap.max(1),
        }
    }

    pub fn record(&self, entry: AlertLogEntry) {
        let mut g = self.entries.lock().unwrap();
        if g.len() >= self.cap {
            g.remove(0);
        }
        g.push(entry);
    }

    pub fn recent(&self, limit: usize) -> Vec<AlertLogEntry> {
        let g = self.entries.lock().unwrap();
        let start = g.len().saturating_sub(limit);
        g[start..].to_vec()
    }

    pub fn count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

/// Owns the sink fan-out + dedup + the dispatch log.
pub struct AlertDispatcher {
    sinks: Vec<Box<dyn AlertSink>>,
    min_severity: InsightSeverity,
    dispatched: Mutex<HashSet<String>>,
    log: Arc<AlertLog>,
}

impl AlertDispatcher {
    /// Build from settings. Returns `None` when alerting is disabled (so the
    /// caller can skip the evaluate step entirely).
    #[allow(unused_mut, unused_variables)]
    pub fn from_settings(settings: &AlertSettings) -> Option<Self> {
        if !settings.enabled {
            return None;
        }
        let mut sinks: Vec<Box<dyn AlertSink>> = Vec::new();
        for url in &settings.webhooks {
            #[cfg(feature = "alerts")]
            sinks.push(Box::new(WebhookSink::new(
                url.clone(),
                settings.webhook_token.clone(),
                build_webhook_client(),
            )));
            #[cfg(not(feature = "alerts"))]
            {
                let _ = url;
                tracing::warn!(
                    webhook = url,
                    "alerts enabled but `alerts` feature is off; webhook will not fire"
                );
            }
        }
        Some(Self {
            sinks,
            min_severity: parse_severity(&settings.min_severity),
            dispatched: Mutex::new(HashSet::new()),
            log: Arc::new(AlertLog::new(200)),
        })
    }

    /// Test-only constructor with explicit sinks.
    pub fn with_sinks(sinks: Vec<Box<dyn AlertSink>>, min_severity: InsightSeverity) -> Self {
        Self {
            sinks,
            min_severity,
            dispatched: Mutex::new(HashSet::new()),
            log: Arc::new(AlertLog::new(200)),
        }
    }

    pub fn log(&self) -> Arc<AlertLog> {
        self.log.clone()
    }

    /// Evaluate a batch of insights: for each that (a) meets the severity
    /// threshold and (b) hasn't been dispatched before, fan out to all sinks.
    /// Dedup is by insight id, so a re-run that produces the same insight id
    /// won't re-alert.
    pub async fn evaluate(&self, insights: &[Insight]) {
        let threshold = severity_rank(&self.min_severity);
        for insight in insights {
            if severity_rank(&insight.severity) < threshold {
                continue;
            }
            // Dedup check + mark atomically.
            let already = {
                let mut d = self.dispatched.lock().unwrap();
                if d.contains(&insight.id) {
                    true
                } else {
                    d.insert(insight.id.clone());
                    false
                }
            };
            if already {
                continue;
            }
            for sink in &self.sinks {
                let status = match sink.dispatch(insight).await {
                    Ok(()) => "ok".to_string(),
                    Err(e) => e.to_string(),
                };
                self.log.record(AlertLogEntry {
                    insight_id: insight.id.clone(),
                    insight_kind: insight.kind.clone(),
                    severity: insight.severity.to_string(),
                    sink: sink.name().to_string(),
                    status,
                    dispatched_at: Utc::now(),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WebhookSink (behind the `alerts` feature)
// ---------------------------------------------------------------------------

/// POSTs each insight as JSON to a webhook URL. Bounded retry: one immediate
/// attempt, one retry after 1s. Delivery failures are surfaced via the
/// returned error (which the dispatcher logs).
#[cfg(feature = "alerts")]
pub struct WebhookSink {
    url: String,
    token: Option<String>,
    client: reqwest::Client,
}

#[cfg(feature = "alerts")]
impl WebhookSink {
    pub fn new(url: String, token: Option<String>, client: reqwest::Client) -> Self {
        Self { url, token, client }
    }
}

#[cfg(feature = "alerts")]
#[async_trait]
impl AlertSink for WebhookSink {
    fn name(&self) -> &'static str {
        "webhook"
    }

    async fn dispatch(&self, insight: &Insight) -> Result<()> {
        let body = serde_json::to_value(insight)
            .map_err(|e| hkgov_common::Error::Internal(format!("serialize insight: {e}")))?;
        let payload: Value = serde_json::json!({ "event": "insight", "insight": body });

        for attempt in 0..2u8 {
            let mut req = self.client.post(&self.url).json(&payload);
            if let Some(ref tok) = self.token {
                if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {tok}")) {
                    req = req.header("Authorization", v);
                }
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp) => {
                    let status = resp.status();
                    let detail = resp.text().await.unwrap_or_default();
                    if attempt == 0 {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tracing::warn!(
                            url = %self.url,
                            status = status.as_u16(),
                            "webhook attempt 1 failed, retrying"
                        );
                        continue;
                    }
                    return Err(hkgov_common::Error::Upstream {
                        origin: "alert-webhook",
                        status: status.as_u16(),
                        detail,
                    });
                }
                Err(e) => {
                    if attempt == 0 {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tracing::warn!(url = %self.url, error = %e, "webhook transport error, retrying");
                        continue;
                    }
                    return Err(hkgov_common::Error::Upstream {
                        origin: "alert-webhook",
                        status: 0,
                        detail: format!("transport: {e}"),
                    });
                }
            }
        }
        unreachable!("retry loop returns within 2 attempts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{EvidenceRef, Insight, InsightSeverity};
    use async_trait::async_trait;
    use chrono::Utc;

    fn insight(id: &str, severity: InsightSeverity) -> Insight {
        Insight {
            id: id.into(),
            kind: "test".into(),
            severity,
            title: "t".into(),
            summary: "s".into(),
            source: hkgov_common::DataSource::Hkma,
            dataset: "x".into(),
            evidence: vec![EvidenceRef {
                record_id: "r".into(),
                field: "f".into(),
                value: serde_json::json!(1),
                context: None,
            }],
            confidence: 0.9,
            generated_at: Utc::now(),
            producer: "test".into(),
        }
    }

    /// A sink that records what it received.
    struct RecordingSink {
        received: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AlertSink for RecordingSink {
        fn name(&self) -> &'static str {
            "recording"
        }
        async fn dispatch(&self, insight: &Insight) -> Result<()> {
            self.received.lock().unwrap().push(insight.id.clone());
            Ok(())
        }
    }

    /// A sink that always fails.
    struct FailingSink;

    #[async_trait]
    impl AlertSink for FailingSink {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn dispatch(&self, _insight: &Insight) -> Result<()> {
            Err(hkgov_common::Error::Internal("boom".into()))
        }
    }

    #[tokio::test]
    async fn dispatches_warning_and_above() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = RecordingSink {
            received: received.clone(),
        };
        let dispatcher =
            AlertDispatcher::with_sinks(vec![Box::new(sink)], InsightSeverity::Warning);
        let insights = vec![
            insight("info-1", InsightSeverity::Info),
            insight("warn-1", InsightSeverity::Warning),
            insight("crit-1", InsightSeverity::Critical),
        ];
        dispatcher.evaluate(&insights).await;
        let got = received.lock().unwrap().clone();
        assert_eq!(got, vec!["warn-1", "crit-1"]);
    }

    #[tokio::test]
    async fn dedups_repeated_insights() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = RecordingSink {
            received: received.clone(),
        };
        let dispatcher = AlertDispatcher::with_sinks(vec![Box::new(sink)], InsightSeverity::Info);
        // Same insight id twice → only one dispatch.
        let insights = vec![insight("dup", InsightSeverity::Critical)];
        dispatcher.evaluate(&insights).await;
        dispatcher.evaluate(&insights).await;
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failing_sink_is_logged_not_fatal() {
        let dispatcher =
            AlertDispatcher::with_sinks(vec![Box::new(FailingSink)], InsightSeverity::Info);
        dispatcher
            .evaluate(&[insight("x", InsightSeverity::Critical)])
            .await;
        let log = dispatcher.log();
        assert_eq!(log.count(), 1);
        let entry = &log.recent(10)[0];
        assert_eq!(entry.status, "internal error: boom");
        assert_eq!(entry.severity, "critical");
    }

    #[test]
    fn severity_threshold_parsing() {
        assert!(matches!(
            parse_severity("critical"),
            InsightSeverity::Critical
        ));
        assert!(matches!(
            parse_severity("warning"),
            InsightSeverity::Warning
        ));
        assert!(matches!(parse_severity("info"), InsightSeverity::Info));
        assert!(matches!(parse_severity("nonsense"), InsightSeverity::Info));
    }

    #[test]
    fn alert_log_caps_entries() {
        let log = AlertLog::new(2);
        for i in 0..5 {
            log.record(AlertLogEntry {
                insight_id: format!("i{i}"),
                insight_kind: "k".into(),
                severity: "warning".into(),
                sink: "s".into(),
                status: "ok".into(),
                dispatched_at: Utc::now(),
            });
        }
        assert_eq!(log.count(), 2);
        // Oldest evicted; newest two remain.
        let ids: Vec<String> = log.recent(10).into_iter().map(|e| e.insight_id).collect();
        assert!(ids.contains(&"i3".to_string()));
        assert!(ids.contains(&"i4".to_string()));
        assert!(!ids.contains(&"i0".to_string()));
    }
}
