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
        let kind = kind_for(&self.0);
        // For 5xx: log the full detail (paths, config values, source errors) for
        // operators, but return a generic message to the client. The previous
        // `self.0.to_string()` body leaked internal details — e.g. a store error
        // surfaced a filesystem path, a config error surfaced the bad value — on
        // every 500/502/503. 4xx errors are user-facing (bad request, not found,
        // unauthorized) and keep their detailed message so the client can self-correct.
        if status.is_server_error() {
            tracing::error!(error = %self.0, kind = %kind, "server error");
            // D-031: StoreUnavailable is retryable and the client should know to
            // retry (it's not an internal fault). Surface a clear, non-leaky
            // message rather than the generic "internal server error" — the
            // dashboard's cite drawer / comparator use this to show "data
            // temporarily unavailable, retry" instead of an error toast.
            let message = match &self.0 {
                Error::StoreUnavailable(_) => {
                    "data temporarily unavailable (refresh in progress or cache cold); retry shortly"
                }
                _ => "internal server error",
            };
            let body = Json(json!({
                "error": { "kind": kind, "message": message }
            }));
            return (status, body).into_response();
        }
        let body = Json(json!({
            "error": {
                "kind": kind,
                "message": self.0.to_string(),
            }
        }));
        (status, body).into_response()
    }
}

fn kind_for(e: &Error) -> &'static str {
    match e {
        Error::Upstream { .. } => "upstream",
        Error::Decode { .. } => "decode",
        Error::UnknownSource(_) | Error::NotFound(_) => "not_found",
        Error::BadRequest(_) => "bad_request",
        Error::Unauthorized(_) => "unauthorized",
        Error::Store(_) => "store",
        Error::StoreUnavailable(_) => "store_unavailable",
        Error::Agent(_) => "agent",
        Error::Config(_) => "config",
        Error::Io(_) => "io",
        Error::Internal(_) => "internal",
    }
}
