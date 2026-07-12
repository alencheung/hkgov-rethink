//! Auth/identity handlers (P-108) — magic-link email + session.
//!
//! Extracted from the main routes module for navigability.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(super) struct RequestTokenRequest {
    pub(super) email: String,
}

#[derive(Serialize)]
pub(super) struct TokenResponse {
    /// The one-time token. V-005: **only** present when `api.dev_return_auth_token`
    /// is set (dev/CI). In production the token is delivered out-of-band (email)
    /// and this field is `None` (serialized as `null` / omitted) — returning it
    /// in the body meant anyone who could read the response (logs/MITM) could
    /// impersonate the email owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) token: Option<String>,
    /// When the token expires (RFC 3339). The client should re-request after.
    pub(super) expires_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn request_auth_token(
    State(state): State<AppState>,
    Json(req): Json<RequestTokenRequest>,
) -> Json<TokenResponse> {
    let t = state.users.issue_token(req.email.trim()).await;
    // V-005: only return the credential in the body when the operator has
    // explicitly opted into the dev/CI mode. Otherwise the token must be
    // delivered out-of-band via the magic-link delivery sink.
    let token = if state.settings.api.dev_return_auth_token {
        Some(t.token.clone())
    } else {
        None
    };
    // Deliver the magic link via the configured delivery sink (log-based by
    // default; HTTP email-gateway when HKGOV_MAGIC_LINK__API_URL is set). The
    // redeem URL carries the token to the user's email. When dev_return_auth_token
    // is on (dev/CI), the token is also in the response body so tests can redeem
    // without email; in production the token reaches the user only via delivery.
    let redeem_url = format!(
        "{}/auth/redeem?token={}",
        state.settings.api.api_prefix.trim_matches('/'),
        t.token
    );
    // The delivery sink handles failures gracefully (logs + returns Err), but
    // a delivery failure should NOT fail the request — the token is already
    // minted and the user may retry. Log the failure so operators can detect
    // a broken email gateway.
    if let Err(e) = state
        .magic_link_delivery
        .deliver(&t.email, &redeem_url, t.expires_at)
        .await
    {
        tracing::warn!(
            email = %t.email,
            error = %e,
            sink = %state.magic_link_delivery.name(),
            "magic-link delivery failed — token is valid but the user will not receive it"
        );
    }
    Json(TokenResponse {
        token,
        expires_at: t.expires_at,
    })
}

#[derive(Deserialize)]
pub(super) struct RedeemRequest {
    token: String,
}

#[derive(Serialize)]
pub(super) struct RedeemResponse {
    session_token: String,
    user: hkgov_agent::User,
}

pub(super) async fn redeem_auth_token(
    State(state): State<AppState>,
    Json(req): Json<RedeemRequest>,
) -> Result<Json<RedeemResponse>, ApiError> {
    let session = state.users.redeem_token(&req.token).await.ok_or_else(|| {
        ApiError(hkgov_common::Error::BadRequest(
            "token invalid, expired, or already used".into(),
        ))
    })?;
    let user = state.users.get(&session.user_id).await.ok_or_else(|| {
        ApiError(hkgov_common::Error::Internal(
            "session minted for unknown user".into(),
        ))
    })?;
    Ok(Json(RedeemResponse {
        session_token: session.session_token,
        user,
    }))
}

/// Resolve the `Authorization: Bearer {session}` header to the current user.
/// Returns 401 when no (valid) session is present, so a client gating UI on auth
/// gets a distinct status from a successful call (matching `require_principal`).
pub(super) async fn auth_me(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<hkgov_agent::User>, ApiError> {
    let session = bearer_token(&headers);
    let user = match session {
        Some(s) => state.users.lookup_session(&s).await,
        None => None,
    };
    user.map(Json).ok_or_else(|| {
        ApiError(hkgov_common::Error::Unauthorized(
            "no active session: send a valid Authorization: Bearer {session}".into(),
        ))
    })
}

/// Extract the `Bearer {token}` value from an Authorization header, if present.
pub(super) fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}
