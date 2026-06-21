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
        (status, body).into_response()
    }
}

fn kind_for(e: &Error) -> &'static str {
    match e {
        Error::Upstream { .. } => "upstream",
        Error::Decode { .. } => "decode",
        Error::UnknownSource(_) => "not_found",
        Error::Store(_) => "store",
        Error::Agent(_) => "agent",
        Error::Config(_) => "config",
        Error::Io(_) => "io",
        Error::Internal(_) => "internal",
    }
}
