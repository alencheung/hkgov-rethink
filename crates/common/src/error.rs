//! Crate-wide error model.
//!
//! We keep errors coarse at this layer and let the boundary (HTTP handler /
//! connector call site) translate them into HTTP statuses or retry decisions.
//! Anything unexpected falls back to [`Error::Internal`].

use thiserror::Error;

/// A non-fatal error surfaced from a connector or the pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Upstream returned a non-success HTTP status or transport failed.
    #[error("upstream error for {origin}: {status}: {detail}")]
    Upstream {
        origin: &'static str,
        status: u16,
        detail: String,
    },

    /// Body could not be parsed into the expected shape.
    #[error("decode error for {origin}: {backtrace}")]
    Decode {
        origin: &'static str,
        #[source]
        backtrace: serde_json::Error,
    },

    /// Caller asked for something that has no connector registered yet.
    #[error("unknown data source: {0}")]
    UnknownSource(String),

    /// The requested resource (e.g. an insight id) does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The request was malformed (bad query param, unsupported format, etc.).
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Cache / store miss or backing failure.
    #[error("store error: {0}")]
    Store(String),

    /// The AI-agent layer failed (loop exhaustion, framing failure, etc.).
    /// Mapped to 502 because the agent depends on upstream (LLM) availability.
    #[error("agent error: {0}")]
    Agent(String),

    #[error("configuration error: {0}")]
    Config(String),

    /// The caller has exceeded a rate limit. Carries the seconds the client
    /// SHOULD wait before retrying (emitted as the `Retry-After` header at the
    /// HTTP boundary). Mapped to 429 Too Many Requests.
    #[error("rate limited; retry after {0}s")]
    RateLimited(u64),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Convenience for the HTTP layer: rough status code mapping.
    pub fn status_code(&self) -> u16 {
        match self {
            Error::UnknownSource(_) | Error::NotFound(_) => 404,
            Error::BadRequest(_) => 400,
            Error::RateLimited(_) => 429,
            Error::Upstream { .. } | Error::Store(_) => 502,
            Error::Agent(_) => 502,
            Error::Decode { .. } => 502,
            Error::Config(_) => 500,
            Error::Io(_) | Error::Internal(_) => 500,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
