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
) -> Result<Json<TokenResponse>, ApiError> {
    // A-011: validate the email before minting. Without this, any non-empty
    // trimmed string was accepted and provisioned a `User` — so a caller
    // varying the email could grow the `users` map (and every snapshot file)
    // without bound. Require a plausible `local@domain` shape and cap the
    // length (254 is RFC 5321's max). The API-key guard (when configured)
    // already gates this endpoint in prod; this is defense-in-depth.
    let email = req.email.trim();
    if !is_plausible_email(email) {
        return Err(ApiError(hkgov_common::Error::BadRequest(
            "email must be a plausible local@domain address of ≤254 chars".into(),
        )));
    }
    let t = state.users.issue_token(email).await;
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
    Ok(Json(TokenResponse {
        token,
        expires_at: t.expires_at,
    }))
}

/// A-011: lightweight email shape check. Not a full RFC 5322 validator (those
/// reject legitimate edge cases) — just enough to refuse garbage that would
/// grow the `users` map with junk: non-empty, ≤254 chars, exactly one `@`
/// with non-empty local + domain parts, and a domain with at least one dot.
fn is_plausible_email(email: &str) -> bool {
    if email.is_empty() || email.len() > 254 {
        return false;
    }
    // Exactly one '@'. rsplit_once would silently accept "a@b@c" (splitting on
    // the last @), so count explicitly.
    if email.matches('@').count() != 1 {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    // Domain must contain a dot and no whitespace; local must have no whitespace.
    if !domain.contains('.') || domain.contains(char::is_whitespace) {
        return false;
    }
    if local.contains(char::is_whitespace) {
        return false;
    }
    true
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- A-011: request-token must reject garbage emails, not provision --
    // ---- a User row for every distinct string the caller sends.          --

    #[test]
    fn plausible_email_accepts_normal_addresses() {
        assert!(is_plausible_email("alice@example.com"));
        assert!(is_plausible_email("bob.foo+tag@sub.example.co.uk"));
        assert!(is_plausible_email("x@y.io"));
    }

    #[test]
    fn plausible_email_rejects_garbage_and_oversize() {
        // The A-011 repro: anything non-empty used to be accepted.
        assert!(!is_plausible_email(""), "empty");
        assert!(!is_plausible_email("no-at-sign"), "no @");
        assert!(!is_plausible_email("@nodomain.com"), "empty local");
        assert!(!is_plausible_email("nolocalpart@"), "empty domain");
        assert!(!is_plausible_email("no-dot@domain"), "domain without dot");
        assert!(
            !is_plausible_email("white space@example.com"),
            "whitespace local"
        );
        assert!(!is_plausible_email("user@dom ain.com"), "whitespace domain");
        // Oversize (>254, RFC 5321 max).
        let long = format!("{}@example.com", "a".repeat(250));
        assert!(!is_plausible_email(&long), "over 254 chars");
        // Multiple @ — only one allowed.
        assert!(!is_plausible_email("a@b@c.com"), "multiple @");
    }
}
