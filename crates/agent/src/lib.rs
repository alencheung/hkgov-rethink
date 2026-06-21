//! AI-agent analysis layer (ROADMAP v3).
//!
//! The agent reads normalized records from the [`RecordStore`], runs analysis
//! passes (anomaly detection, cross-source divergence), optionally consults an
//! LLM for natural-language framing, and writes the results back as
//! [`Insight`] records served via the same `/insights` API.
//!
//! It runs on its own scheduler (see [`scheduler`]) so it never blocks serving.
//!
//! Two LLM client implementations:
//! - [`llm::HeuristicClient`] (default): pure-Rust statistical heuristics, no
//!   network, no API key. Used in dev/CI and as the deterministic baseline.
//! - [`llm::HttpLlmClient`] (behind the `llm` feature): OpenAI-compatible
//!   chat-completions client for richer narrative framing.
//!
//! The core analysis ([`analysis`]) is provider-agnostic: the heuristic client
//! surfaces the same structured findings an LLM would, so insights work end to
//! end without external dependencies.

pub mod analysis;
pub mod insight;
pub mod llm;
pub mod scheduler;

pub use insight::{Insight, InsightSeverity, InsightStore};
#[cfg(feature = "llm")]
pub use llm::HttpLlmClient;
pub use llm::{HeuristicClient, LlmClient};
pub use scheduler::AgentSupervisor;
