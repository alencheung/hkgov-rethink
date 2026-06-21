//! Pluggable LLM client. The agent talks only to the [`LlmClient`] trait so it
//! runs identically in heuristic mode (no key, deterministic) and LLM mode.
//!
//! The trait has two responsibilities:
//! - [`LlmClient::frame`] (v3): turn a deterministic [`Finding`] into a
//!   natural-language `summary` + adjusted `confidence`. Detection stays
//!   deterministic; this is pure framing.
//! - [`LlmClient::step`] (v6): one round of an agentic conversation. Given the
//!   message history + available tools, the model either requests a tool call
//!   ([`AgentStep::ToolCall`]) or finalizes ([`AgentStep::Final`]). The
//!   default impl finalizes immediately, so deterministic clients (the
//!   heuristic baseline) opt out of multi-step reasoning for free.

use async_trait::async_trait;
#[cfg(feature = "llm")]
use hkgov_common::AgentSettings;
#[cfg(feature = "llm")]
use hkgov_common::Error;
use hkgov_common::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::analysis::Finding;

/// What an LLM client returns for a finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmFraming {
    pub summary: String,
    /// Adjusted confidence in 0..=1. The heuristic client echoes the finding's
    /// score; an LLM may raise/lower it.
    pub confidence: f64,
}

/// One message in an agent conversation. Mirrors the OpenAI chat roles; the
/// `tool` role carries a tool-call result back to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    /// The assistant's turn — optionally including tool calls it wants made.
    Assistant {
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    /// A tool result, keyed by the `tool_call_id` it answers.
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// A tool call the model wants the agent to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments as the model produced them.
    pub arguments: Value,
}

/// One step's outcome: either the model wants a tool run, or it's done.
#[derive(Debug, Clone)]
pub enum AgentStep {
    /// The model requests a tool call. The loop executes it and feeds the
    /// result back as a `Message::Tool`.
    ToolCall(ToolCall),
    /// The model is done reasoning and produced a final framing.
    Final(LlmFraming),
}

#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn frame(&self, finding: &Finding) -> Result<LlmFraming>;

    /// One reasoning step. Default impl finalizes with an empty framing so
    /// deterministic clients (which don't reason) opt out of the loop cleanly.
    async fn step(&self, _messages: &[Message], _tool_specs: &[Value]) -> Result<AgentStep> {
        Ok(AgentStep::Final(LlmFraming {
            summary: String::new(),
            confidence: 0.0,
        }))
    }
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

    async fn step(&self, messages: &[Message], tool_specs: &[Value]) -> Result<AgentStep> {
        // Serialize the conversation into the OpenAI message shape.
        let msgs: Vec<Value> = messages.iter().map(message_to_openai).collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "temperature": 0.2,
        });
        if !tool_specs.is_empty() {
            body["tools"] = Value::Array(tool_specs.to_vec());
            // Let the model choose between calling a tool and answering.
            body["tool_choice"] = serde_json::json!("auto");
        }

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
        let v: Value = resp.json().await.map_err(|e| Error::Decode {
            origin: "llm",
            backtrace: serde::de::Error::custom(e.to_string()),
        })?;

        let msg = &v["choices"][0]["message"];

        // If the model emitted tool_calls, return the first one.
        if let Some(calls) = msg["tool_calls"].as_array() {
            if let Some(call) = calls.first() {
                let id = call["id"].as_str().unwrap_or("call").to_string();
                let name = call["function"]["name"].as_str().unwrap_or("").to_string();
                let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);
                return Ok(AgentStep::ToolCall(ToolCall {
                    id,
                    name,
                    arguments,
                }));
            }
        }

        // Otherwise the model produced a final answer. It may be a JSON
        // object (our requested shape) or free text.
        let content = msg["content"].as_str().unwrap_or("");
        let framing = serde_json::from_str::<LlmFraming>(content).unwrap_or(LlmFraming {
            summary: content.to_string(),
            confidence: 0.5,
        });
        Ok(AgentStep::Final(framing))
    }
}

/// Render a [`Message`] into the OpenAI chat-message JSON shape.
#[cfg(feature = "llm")]
fn message_to_openai(m: &Message) -> Value {
    match m {
        Message::System { content } => serde_json::json!({"role": "system", "content": content}),
        Message::User { content } => serde_json::json!({"role": "user", "content": content}),
        Message::Assistant {
            content,
            tool_calls,
        } => {
            let mut obj = serde_json::Map::new();
            obj.insert("role".into(), Value::String("assistant".into()));
            if let Some(c) = content {
                obj.insert("content".into(), Value::String(c.clone()));
            }
            if !tool_calls.is_empty() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                obj.insert("tool_calls".into(), Value::Array(calls));
            }
            Value::Object(obj)
        }
        Message::Tool {
            tool_call_id,
            content,
        } => serde_json::json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "content": content,
        }),
    }
}
