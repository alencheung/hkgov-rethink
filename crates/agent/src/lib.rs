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
pub mod bilingual;
pub mod brief;
pub mod cite;
pub mod identity;
pub mod insight;
pub mod investigation;
pub mod llm;
pub mod loop_mod;
pub mod persist;
pub mod provenance;
pub mod qa;
pub mod scheduler;
pub mod signal;
pub mod silence;
pub mod tools;
pub mod transparency;
pub mod transparency_report;
pub mod unprecedentedness;

pub use alerts::{AlertDispatcher, AlertLog, AlertLogEntry, AlertSink};
#[cfg(feature = "alerts")]
pub use alerts::{EmailSink, WebhookSink};
pub use bilingual::{frame_zh_hk, select_summary, Language};
pub use brief::{build_brief, Brief, BriefItem};
pub use cite::{build_citation, Citation, CitationFormat, ReproducibilityManifest, CITE_VERSION};
#[cfg(feature = "alerts")]
pub use identity::HttpMagicLinkDelivery;
pub use identity::{
    user_id_for, LogMagicLinkDelivery, MagicLinkDelivery, Session, Token, User, UserStore,
    UserStoreSnapshot,
};
pub use insight::{
    EvolutionDiff, Feedback, FeedbackStore, FeedbackStoreSnapshot, FieldChange, Insight,
    InsightRevision, InsightSeverity, InsightStore, InsightStoreSnapshot,
};
pub use investigation::{
    investigation_id, Investigation, InvestigationNote, InvestigationStep, InvestigationStore,
    InvestigationStoreSnapshot, StepKind,
};
#[cfg(feature = "llm")]
pub use llm::HttpLlmClient;
pub use llm::{AgentStep, HeuristicClient, LlmClient, LlmFraming, Message, ToolCall};
pub use loop_mod::{run_agent_loop, AgentOutcome, Answer, TraceStep};
pub use provenance::{
    build_attestation, filter_audit, Attestation, AuditQuery, Producer, ProvenanceRecord,
    ProvenanceStore, ATTESTATION_VERSION, PROVENANCE_VERSION,
};
pub use qa::heuristic_answer;
pub use scheduler::AgentSupervisor;
pub use signal::{
    preview_signal, signal_id, FindingDto as SignalFindingDto, Signal, SignalChannel, SignalPatch,
    SignalPreview, SignalStore, SignalStoreSnapshot,
};
pub use silence::{
    build_index as build_silence_index,
    build_index_from_insights as build_silence_index_from_insights, methodology_version,
    SilenceIndex, SilenceSignal, SilenceSignalKind,
};
pub use tools::{FindingDto, Tool, ToolBelt};
pub use transparency::{
    build_composite_index, build_index_from_registry, default_registry, CompositeTransparencyIndex,
    SourceBreakdown, TransparencySignal, TransparencySignalRegistry,
};
pub use transparency_report::{
    build_report, render_markdown, ReportInsight, ReportOptions, ReportSignal, TransparencyReport,
    DEFAULT_PUBLISHER, REPORT_VERSION,
};
pub use unprecedentedness::{
    score as score_unprecedentedness, LastExceeded, NormalRange, Unprecedentedness, DEFAULT_BAND_K,
    MIN_HISTORY_POINTS,
};
