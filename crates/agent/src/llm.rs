//! Pluggable LLM client. The agent talks only to the [`LlmClient`] trait so it
//! runs identically in heuristic mode (no key, deterministic) and LLM mode.
//!
//! The trait is deliberately narrow: given structured `Finding`s, produce a
//! natural-language `summary` + adjusted `confidence`. The detection logic
//! itself lives in [`crate::analysis`] and is provider-independent.

use async_trait::async_trait;
#[cfg(feature = "llm")]
use hkgov_common::AgentSettings;
#[cfg(feature = "llm")]
use hkgov_common::Error;
use hkgov_common::Result;
use serde::{Deserialize, Serialize};

use crate::analysis::Finding;

/// What an LLM client returns for a finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmFraming {
    pub summary: String,
    /// Adjusted confidence in 0..=1. The heuristic client echoes the finding's
    /// score; an LLM may raise/lower it.
    pub confidence: f64,
}

#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn frame(&self, finding: &Finding) -> Result<LlmFraming>;
}

/// Pure-Rust heuristic client. No network, no key, deterministic. This is the
/// default and the baseline — every insight works end to end without it.
pub struct HeuristicClient;

impl HeuristicClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HeuristicClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmClient for HeuristicClient {
    fn name(&self) -> &'static str {
        "heuristic"
    }

    async fn frame(&self, finding: &Finding) -> Result<LlmFraming> {
        Ok(LlmFraming {
            summary: finding.heuristic_summary(),
            confidence: finding.confidence,
        })
    }
}

/// OpenAI-compatible chat-completions client. Behind the `llm` feature so the
/// default build needs no HTTP/key. Uses a POST to `{base}/chat/completions`.
#[cfg(feature = "llm")]
pub struct HttpLlmClient {
    base_url: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

#[cfg(feature = "llm")]
impl HttpLlmClient {
    pub fn new(settings: &AgentSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Internal(format!("llm reqwest build: {e}")))?;
        Ok(Self {
            base_url: settings.llm_base_url.trim_end_matches('/').to_string(),
            api_key: settings.llm_api_key.clone(),
            model: settings.llm_model.clone(),
            client,
        })
    }
}

#[cfg(feature = "llm")]
#[async_trait]
impl LlmClient for HttpLlmClient {
    fn name(&self) -> &'static str {
        "llm"
    }

    async fn frame(&self, finding: &Finding) -> Result<LlmFraming> {
        let system = "You are a financial-data analyst for Hong Kong government \
            open data. Given a structured finding, write a concise (<=2 sentence) \
            plain-language summary and return JSON {summary, confidence}. \
            Confidence is 0..1.";
        let user = serde_json::to_string_pretty(finding)
            .map_err(|e| Error::Internal(format!("serialize finding: {e}")))?;

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0.2,
            "response_format": {"type": "json_object"},
        });

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(ref key) = self.api_key {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
                req = req.header("Authorization", v);
            }
        }
        let resp = req.send().await.map_err(|e| Error::Upstream {
            origin: "llm",
            status: 0,
            detail: format!("transport: {e}"),
        })?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream {
                origin: "llm",
                status,
                detail,
            });
        }
        let v: serde_json::Value = resp.json().await.map_err(|e| Error::Decode {
            origin: "llm",
            backtrace: serde::de::Error::custom(e.to_string()),
        })?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");
        let parsed: LlmFraming = serde_json::from_str(content).map_err(|e| Error::Decode {
            origin: "llm",
            backtrace: e,
        })?;
        Ok(parsed)
    }
}
