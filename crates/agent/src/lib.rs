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

pub mod alerts;
#[cfg(feature = "alerts")]
pub mod alerts_webhook_deps;
pub mod analysis;
pub mod brief;
pub mod cite;
pub mod insight;
pub mod llm;
pub mod loop_mod;
pub mod qa;
pub mod scheduler;
pub mod silence;
pub mod tools;
pub mod unprecedentedness;

pub use alerts::{AlertDispatcher, AlertLog, AlertLogEntry, AlertSink};
#[cfg(feature = "alerts")]
pub use alerts::{EmailSink, WebhookSink};
pub use brief::{build_brief, Brief, BriefItem};
pub use cite::{build_citation, Citation, CitationFormat, ReproducibilityManifest, CITE_VERSION};
pub use insight::{Feedback, FeedbackStore, Insight, InsightSeverity, InsightStore};
#[cfg(feature = "llm")]
pub use llm::HttpLlmClient;
pub use llm::{AgentStep, HeuristicClient, LlmClient, LlmFraming, Message, ToolCall};
pub use loop_mod::{run_agent_loop, AgentOutcome, Answer, TraceStep};
pub use qa::heuristic_answer;
pub use scheduler::AgentSupervisor;
pub use silence::{
    build_index as build_silence_index,
    build_index_from_insights as build_silence_index_from_insights, SilenceIndex, SilenceSignal,
    SilenceSignalKind, METHODOLOGY_VERSION as SILENCE_METHODOLOGY_VERSION,
};
pub use tools::{FindingDto, Tool, ToolBelt};
pub use unprecedentedness::{
    score as score_unprecedentedness, LastExceeded, NormalRange, Unprecedentedness, DEFAULT_BAND_K,
    MIN_HISTORY_POINTS,
};
