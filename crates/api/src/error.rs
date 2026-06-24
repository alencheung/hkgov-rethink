//! HTTP error mapping.
//!
//! Converts [`hkgov_common::Error`] into a JSON problem-details-style body with
//! the right status code. Keeps handlers free of status-code plumbing.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use hkgov_common::Error;
use serde_json::json;

pub struct ApiError(pub Error);

impl std::fmt::Debug for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ApiError").field(&self.0).finish()
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = Json(json!({
            "error": {
                "kind": kind_for(&self.0),
                "message": self.0.to_string(),
            }
        }));

        // 429 responses carry the standard `Retry-After` header (seconds) and a
        // machine-readable `X-RateLimit-Remaining: 0` so well-behaved clients
        // can back off without parsing the message body.
        if let Error::RateLimited(secs) = &self.0 {
            return (
                status,
                [
                    (axum::http::header::RETRY_AFTER, secs.to_string()),
                    (
                        axum::http::HeaderName::from_static("x-ratelimit-remaining"),
                        "0".to_string(),
                    ),
                ],
                body,
            )
                .into_response();
        }
        (status, body).into_response()
    }
}

fn kind_for(e: &Error) -> &'static str {
    match e {
        Error::Upstream { .. } => "upstream",
        Error::Decode { .. } => "decode",
        Error::UnknownSource(_) | Error::NotFound(_) => "not_found",
        Error::BadRequest(_) => "bad_request",
        Error::RateLimited(_) => "rate_limited",
        Error::Store(_) => "store",
        Error::Agent(_) => "agent",
        Error::Config(_) => "config",
        Error::Io(_) => "io",
        Error::Internal(_) => "internal",
    }
}
