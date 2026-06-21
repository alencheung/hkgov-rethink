//! Provider-agnostic agent loop (v6 — agentic investigation).
//!
//! [`run_agent_loop`] drives a multi-step conversation: the LLM proposes a tool
//! call (or a final answer), the loop executes any tool via the [`ToolBelt`],
//! feeds the result back, and repeats until the model finalizes or the
//! `max_steps` ceiling is hit.
//!
//! **Determinism guarantee:** the LLM only ever *chooses* which deterministic
//! tool to call and *frames* the result — it never performs detection itself.
//! A heuristic client (whose [`LlmClient::step`] finalizes immediately) makes
//! the loop a no-op, so the same code path serves both modes.

use crate::analysis::Finding;
use crate::llm::{AgentStep, LlmClient, Message};
use crate::tools::ToolBelt;
use hkgov_common::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a finished agent run produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentOutcome {
    /// The model answered a question in natural language, with the evidence
    /// it gathered along the way.
    Answer(Answer),
    /// The model's reasoning surfaced one or more findings worth promoting to
    /// insights (used by the periodic supervisor in agentic mode).
    Findings(Vec<Finding>),
}

/// A natural-language answer to a user question, with verifiable evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub text: String,
    pub confidence: f64,
    /// Tool-call trace, in order — lets the caller show *how* the answer was
    /// reached and verify the evidence pointers.
    pub trace: Vec<TraceStep>,
}

/// One step in the agent's reasoning trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub tool: String,
    pub arguments: Value,
    pub result: Value,
}

/// Drive an agentic conversation to completion.
///
/// - `system` is the system prompt establishing the agent's role.
/// - `prompt` is the user's question or task.
/// - `max_steps` bounds the number of tool-call rounds (a hard ceiling so a
///   misbehaving model can't loop forever or rack up cost).
///
/// On `max_steps` exhaustion without a `Final`, the loop returns an error so
/// the caller can decide whether to fall back (e.g. to heuristic framing).
pub async fn run_agent_loop(
    llm: &dyn LlmClient,
    belt: &ToolBelt,
    system: &str,
    prompt: &str,
    max_steps: u8,
) -> Result<AgentOutcome> {
    let tool_specs = belt.tool_specs();

    // Seed the conversation.
    let mut messages: Vec<Message> = vec![
        Message::System {
            content: system.to_string(),
        },
        Message::User {
            content: prompt.to_string(),
        },
    ];
    let mut trace: Vec<TraceStep> = Vec::new();

    for _ in 0..max_steps {
        match llm.step(&messages, &tool_specs).await? {
            AgentStep::ToolCall(call) => {
                // Execute the tool against the belt (deterministic).
                let result = belt
                    .invoke(&call.name, &call.arguments)
                    .await
                    .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }));

                // Record the trace.
                trace.push(TraceStep {
                    tool: call.name.clone(),
                    arguments: call.arguments.clone(),
                    result: result.clone(),
                });

                // Feed the assistant's tool request + the tool result back.
                messages.push(Message::Assistant {
                    content: None,
                    tool_calls: vec![call.clone()],
                });
                messages.push(Message::Tool {
                    tool_call_id: call.id,
                    content: result.to_string(),
                });
            }
            AgentStep::Final(framing) => {
                return Ok(AgentOutcome::Answer(Answer {
                    text: framing.summary,
                    confidence: framing.confidence,
                    trace,
                }));
            }
        }
    }

    // Hit the step ceiling without a final answer.
    Err(hkgov_common::Error::Internal(format!(
        "agent loop exhausted max_steps ({max_steps}) without a final answer"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Finding;
    use crate::llm::ToolCall;
    use async_trait::async_trait;
    use hkgov_common::{DataSource, NormalizedRecord, RecordValue};
    use hkgov_store::{DatasetId, MemoryStore, RecordStore};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// A scripted LLM client that replays a queue of steps then finalizes.
    /// Used to drive the loop deterministically without any HTTP.
    struct ScriptedClient {
        steps: Mutex<Vec<AgentStep>>,
    }

    #[async_trait]
    impl LlmClient for ScriptedClient {
        fn name(&self) -> &'static str {
            "scripted"
        }
        async fn frame(&self, _finding: &Finding) -> hkgov_common::Result<crate::llm::LlmFraming> {
            Ok(crate::llm::LlmFraming {
                summary: "x".into(),
                confidence: 0.5,
            })
        }
        async fn step(
            &self,
            _messages: &[Message],
            _tools: &[Value],
        ) -> hkgov_common::Result<AgentStep> {
            let mut guard = self.steps.lock().unwrap();
            if guard.is_empty() {
                Ok(AgentStep::Final(crate::llm::LlmFraming {
                    summary: "done".into(),
                    confidence: 0.9,
                }))
            } else {
                Ok(guard.remove(0))
            }
        }
    }

    fn make_record(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut fields = BTreeMap::new();
        fields.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields,
            fetched_at: chrono::Utc::now(),
        }
    }

    async fn seed(store: &MemoryStore) {
        let id = DatasetId::new(DataSource::Hkma, "x");
        store
            .put_dataset(
                &id,
                vec![
                    make_record("2026-01", "v", 2.0),
                    make_record("2026-02", "v", 6.0),
                ],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn loop_runs_one_tool_then_finalizes() {
        let store = Arc::new(MemoryStore::new(10, 60));
        seed(&store).await;
        let belt = ToolBelt::for_store(store);

        // The model first asks for a series_jump detector run, then finalizes.
        let llm = ScriptedClient {
            steps: Mutex::new(vec![AgentStep::ToolCall(ToolCall {
                id: "call_1".into(),
                name: "run_detector".into(),
                arguments: json!({
                    "detector": "series_jump",
                    "source": "hkma",
                    "dataset": "x",
                    "field": "v",
                    "threshold": 50.0
                }),
            })]),
        };

        let outcome = run_agent_loop(&llm, &belt, "system", "find jumps", 4)
            .await
            .unwrap();
        match outcome {
            AgentOutcome::Answer(a) => {
                assert_eq!(a.text, "done");
                assert_eq!(a.trace.len(), 1);
                assert_eq!(a.trace[0].tool, "run_detector");
                // The detector should have surfaced the 2→6 jump.
                let findings = a.trace[0].result["findings"].as_array().unwrap();
                assert_eq!(findings.len(), 1);
                assert_eq!(findings[0]["kind"], "series_jump");
            }
            _ => panic!("expected Answer"),
        }
    }

    #[tokio::test]
    async fn loop_finalizes_immediately_when_no_tool_calls() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        // Empty step queue → first step finalizes immediately.
        let llm = ScriptedClient {
            steps: Mutex::new(vec![]),
        };
        let outcome = run_agent_loop(&llm, &belt, "system", "hi", 4)
            .await
            .unwrap();
        match outcome {
            AgentOutcome::Answer(a) => {
                assert!(a.trace.is_empty());
                assert_eq!(a.text, "done");
            }
            _ => panic!("expected Answer"),
        }
    }

    #[tokio::test]
    async fn loop_errors_on_step_exhaustion() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        // A client that never finalizes — only requests tool calls forever.
        let llm = ScriptedClient {
            steps: Mutex::new(vec![
                AgentStep::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "list_datasets".into(),
                    arguments: json!({}),
                }),
                AgentStep::ToolCall(ToolCall {
                    id: "c2".into(),
                    name: "list_datasets".into(),
                    arguments: json!({}),
                }),
                AgentStep::ToolCall(ToolCall {
                    id: "c3".into(),
                    name: "list_datasets".into(),
                    arguments: json!({}),
                }),
            ]),
        };
        let r = run_agent_loop(&llm, &belt, "system", "loop", 2).await;
        assert!(r.is_err(), "expected step-exhaustion error");
    }

    #[tokio::test]
    async fn heuristic_client_step_finalizes_immediately() {
        // The heuristic client's default step() should finalize, so the loop
        // is a one-shot no-op — this is the "no LLM key" baseline.
        let store = Arc::new(MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        let llm = crate::llm::HeuristicClient::new();
        let outcome = run_agent_loop(&llm, &belt, "system", "anything", 4)
            .await
            .unwrap();
        match outcome {
            AgentOutcome::Answer(a) => {
                assert!(a.trace.is_empty());
            }
            _ => panic!("expected Answer"),
        }
    }
}
