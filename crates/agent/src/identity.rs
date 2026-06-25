//! Identity Tier (P-108) — the shared blocker for per-user state.
//!
//! The cheapest identity that unblocks P-102 (signals), P-104 (read state),
//! and P-105 (investigations): an email + magic-link token. No passwords, no
//! OAuth — a user enters their email, gets a one-time token, and exchanges it
//! for a stable session handle. The `User.id` is the principal the other
//! features key on as `owner`.
//!
//! ## Design
//!
//! - [`User`] — the principal: `{ id, email, created_at }`.
//! - [`UserStore`] — in-process `Arc<RwLock<BTreeMap>>`, volatile (no DB tier).
//! - [`Token`] — a one-time, expiring magic-link token tied to an email. Issued
//!   by [`UserStore::issue_token`], consumed by [`UserStore::redeem_token`].
//! - [`Session`] — a longer-lived handle (the `Authorization: Bearer` value)
//!   returned on redemption. [`UserStore::lookup_session`] resolves it to a
//!   `User`.
//!
//! Token + session ids are 32-byte CSPRNG hex strings. V-006 fix: they were
//! previously `Sha256(timestamp + counter + email)` — deterministic, with no
//! real entropy, so forgeable within a known issue window. They now draw from
//! the OS CSPRNG via the `rand` crate, so two tokens for the same email in the
//! same nanosecond are still unrelated 256-bit secrets.
//!
//! ## Scope
//!
//! v1 ships the store + the issue/redeem/lookup contract. The HTTP routes
//! (`POST /v1/auth/request-token`, `POST /v1/auth/redeem`, `GET /v1/auth/me`)
//! wire it into the API. Email *delivery* is out of scope for the in-process
//! store — `issue_token` returns the token directly (dev/CI) or hands it to an
//! email sink (when the `alerts` feature is wired, future work).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A registered user — the per-user principal that P-102/P-104/P-105 key on as
/// `owner`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Stable id: `u:{email_fingerprint}`. Same email → same id (idempotent).
    pub id: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

/// A one-time, expiring magic-link token. Issued for an email; redeemed once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    /// Already-redeemed tokens are rejected on a second attempt.
    pub redeemed: bool,
}

/// A session handle — the `Authorization: Bearer` value. Longer-lived than a
/// token; resolved to a `User` via [`UserStore::lookup_session`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_token: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    /// D-010: when this session expires. A session is NOT immortal — a leaked
    /// bearer must age out. Defaults to far-future for back-compat with any
    /// serialized session blob that predates the field (the in-process store is
    /// volatile, so in practice this is always set at mint time below).
    #[serde(default = "far_future")]
    pub expires_at: DateTime<Utc>,
}

/// How long a magic-link token is valid (15 min — short, since it's emailed).
const TOKEN_TTL_MINUTES: i64 = 15;

/// How long a redeemed session is valid (30 days — long enough to be useful,
/// short enough that a leaked bearer ages out). D-010.
const SESSION_TTL_DAYS: i64 = 30;

/// Serde default for `Session::expires_at` — far future, so a deserialized
/// legacy session (without the field) is treated as non-expiring rather than
/// instantly expired. The volatile store always sets a real value at mint.
fn far_future() -> DateTime<Utc> {
    Utc::now() + Duration::days(365 * 100)
}

/// In-process identity store. Mirrors the other v8 stores (InsightStore,
/// SignalStore, …) — `Arc<RwLock<BTreeMap>>`, volatile. A real deployment moves
/// this to the Postgres tier.
#[derive(Default)]
pub struct UserStore {
    users: Arc<RwLock<BTreeMap<String, User>>>,
    tokens: Arc<RwLock<BTreeMap<String, Token>>>,
    sessions: Arc<RwLock<BTreeMap<String, Session>>>,
    /// Monotonic counter mixed into token/session id hashing so two tokens
    /// issued in the same nanosecond for the same email still differ.
    counter: Arc<std::sync::atomic::AtomicU64>,
}

impl UserStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a one-time magic-link token for an email. If the email is new,
    /// a `User` is provisioned at the same time (idempotent on email). Returns
    /// the token — the caller delivers it (email sink in production; directly
    /// in dev/CI).
    pub async fn issue_token(&self, email: &str) -> Token {
        // Provision the user if new (idempotent on email).
        let user_id = user_id_for(email);
        let mut users = self.users.write().await;
        users.entry(user_id.clone()).or_insert(User {
            id: user_id,
            email: email.to_string(),
            created_at: Utc::now(),
        });
        drop(users);

        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let token = opaque_token(email, seq, "token");
        let t = Token {
            token: token.clone(),
            email: email.to_string(),
            expires_at: Utc::now() + Duration::minutes(TOKEN_TTL_MINUTES),
            redeemed: false,
        };
        self.tokens.write().await.insert(token, t.clone());
        t
    }

    /// Redeem a token for a session handle. Fails if the token is unknown,
    /// already redeemed, or expired. On success, marks the token redeemed and
    /// mints a fresh `Session`.
    pub async fn redeem_token(&self, token: &str) -> Option<Session> {
        let mut tokens = self.tokens.write().await;
        let t = tokens.get_mut(token)?;
        if t.redeemed {
            return None;
        }
        if Utc::now() > t.expires_at {
            return None;
        }
        t.redeemed = true;
        let user_id = user_id_for(&t.email);
        drop(tokens);
        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let session_token = opaque_token(&user_id, seq, "session");
        let now = Utc::now();
        let session = Session {
            session_token: session_token.clone(),
            user_id,
            created_at: now,
            // D-010: bound the session's lifetime so a leaked bearer ages out.
            expires_at: now + Duration::days(SESSION_TTL_DAYS),
        };
        self.sessions
            .write()
            .await
            .insert(session_token, session.clone());
        Some(session)
    }

    /// Resolve a session token to its user. `None` if unknown OR expired (D-010:
    /// a session is no longer immortal — a leaked bearer ages out after
    /// `SESSION_TTL_DAYS`).
    pub async fn lookup_session(&self, session_token: &str) -> Option<User> {
        let sessions = self.sessions.read().await;
        let s = sessions.get(session_token)?;
        // D-010: reject expired sessions.
        if Utc::now() > s.expires_at {
            return None;
        }
        let user_id = s.user_id.clone();
        drop(sessions);
        self.users.read().await.get(&user_id).cloned()
    }

    /// Look up a user by id.
    pub async fn get(&self, id: &str) -> Option<User> {
        self.users.read().await.get(id).cloned()
    }

    /// Look up a user by email.
    pub async fn get_by_email(&self, email: &str) -> Option<User> {
        self.get(&user_id_for(email)).await
    }

    pub async fn count(&self) -> usize {
        self.users.read().await.len()
    }

    // ---- test-only helpers ------------------------------------------------
    //
    // Used to drive D-010 (session expiry) without fast-forwarding the clock:
    // mint a real session via the public API, then back-date its expiry so
    // `lookup_session`'s TTL check can be exercised deterministically.

    #[cfg(test)]
    pub async fn plant_session_for_test(&self, session: Session) {
        self.sessions
            .write()
            .await
            .insert(session.session_token.clone(), session);
    }
}

/// Stable user id from an email: `u:{sha256(email)[:16]}`. Same email → same id
/// (case-insensitive), so re-issuing a token for the same address hits the same
/// user record.
pub fn user_id_for(email: &str) -> String {
    let mut h = Sha256::new();
    h.update(email.trim().to_ascii_lowercase().as_bytes());
    let hash = h.finalize();
    let hex: String = hash.iter().take(8).map(|b| format!("{:02x}", b)).collect();
    format!("u:{hex}")
}

/// An opaque, unguessable token string (32 bytes hex) drawn from the OS
/// CSPRNG. V-006 fix: previously this was `Sha256(timestamp + counter +
/// domain + subject)` — a deterministic hash with **no real entropy**, so
/// within a known issue window an attacker who could narrow the timestamp had
/// a computationally attackable token space. With OS-entropy bytes there is
/// nothing to predict: two tokens minted in the same nanosecond for the same
/// email are independent 256-bit secrets. The `subject`/`domain`/`seq` params
/// are kept in the signature (callers still pass them) but are no longer the
/// entropy source — they are mixed in only to avoid shrinking the input space
/// below the random bytes (defense-in-depth, not the secret).
fn opaque_token(subject: &str, seq: u64, domain: &str) -> String {
    use rand::RngCore;
    // 32 bytes of OS entropy = the secret. This is what makes the token
    // unguessable; everything below only adds (never subtracts) entropy.
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    // Mix the caller context in too — belt-and-braces so the hex is never
    // *less* identifying than the old hash, while the random bytes remain the
    // unforgeable core.
    let mut h = Sha256::new();
    h.update(bytes);
    h.update(b"\x00");
    h.update(subject.as_bytes());
    h.update(b"\x00");
    h.update(seq.to_le_bytes());
    h.update(b"\x00");
    h.update(domain.as_bytes());
    let hash = h.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn issue_token_provisions_user_idempotently() {
        let store = UserStore::new();
        store.issue_token("alice@example.com").await;
        store.issue_token("alice@example.com").await; // same email → same user
        assert_eq!(store.count().await, 1, "one user per email");
        let u = store.get_by_email("alice@example.com").await.unwrap();
        assert_eq!(u.email, "alice@example.com");
    }

    #[tokio::test]
    async fn redeem_valid_token_returns_session() {
        let store = UserStore::new();
        let t = store.issue_token("bob@example.com").await;
        let session = store.redeem_token(&t.token).await;
        assert!(session.is_some());
        let s = session.unwrap();
        assert_eq!(s.user_id, user_id_for("bob@example.com"));
        // The session resolves back to the user.
        let u = store.lookup_session(&s.session_token).await.unwrap();
        assert_eq!(u.email, "bob@example.com");
    }

    #[tokio::test]
    async fn redeemed_token_cannot_be_reused() {
        let store = UserStore::new();
        let t = store.issue_token("carol@example.com").await;
        let token = t.token.clone();
        assert!(store.redeem_token(&token).await.is_some());
        assert!(
            store.redeem_token(&token).await.is_none(),
            "double-spend rejected"
        );
    }

    #[tokio::test]
    async fn unknown_token_redeems_none() {
        let store = UserStore::new();
        assert!(store.redeem_token("not-a-real-token").await.is_none());
    }

    #[tokio::test]
    async fn unknown_session_looks_up_none() {
        let store = UserStore::new();
        assert!(store.lookup_session("nope").await.is_none());
    }

    #[test]
    fn user_id_is_stable_and_case_insensitive() {
        let a = user_id_for("Alice@Example.com");
        let b = user_id_for("alice@example.com");
        assert_eq!(a, b, "email case + trim normalized");
        let c = user_id_for("bob@example.com");
        assert_ne!(a, c);
        assert!(a.starts_with("u:"));
    }

    #[tokio::test]
    async fn two_tokens_for_same_email_differ() {
        let store = UserStore::new();
        let t1 = store.issue_token("dave@example.com").await;
        let t2 = store.issue_token("dave@example.com").await;
        assert_ne!(t1.token, t2.token, "per-issue tokens must differ");
    }

    // ---- V-006: tokens must be CSPRNG-generated, not timestamp-derived ------
    //
    // Before V-006, `opaque_token` was `Sha256(subject + seq + domain + now)`.
    // That hash had no real entropy: given the same (subject, seq, domain,
    // nanosecond timestamp) it reproduced the same token, and over a known
    // window the space was brute-forceable. The fix draws 32 bytes from the OS
    // CSPRNG first, so the token is a genuine 256-bit secret. These guards lock
    // the property: (1) two rapid issues for the same email never collide, and
    // (2) the token is not a pure function of public inputs.

    #[tokio::test]
    async fn v006_rapid_tokens_are_distinct_and_long() {
        // 50 back-to-back issues in the same nanosecond window — under the old
        // deterministic scheme many would share prefixes/structure; with OS
        // entropy every one is an independent 256-bit secret (64 hex chars).
        let store = UserStore::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let t = store.issue_token("v006@example.com").await;
            assert_eq!(t.token.len(), 64, "token is 32-byte hex (64 chars)");
            assert!(seen.insert(t.token.clone()), "token must be unique");
        }
        assert_eq!(seen.len(), 50);
    }

    #[test]
    fn v006_token_is_not_a_pure_function_of_public_inputs() {
        // Same (subject, seq, domain) twice → different outputs, because the
        // OS-random 32 bytes are the entropy source, not the public args.
        let a = opaque_token("x@y.z", 1, "token");
        let b = opaque_token("x@y.z", 1, "token");
        assert_ne!(a, b, "V-006: identical public inputs must not mint the same token");
    }

    #[tokio::test]
    async fn end_to_end_identity_flow() {
        let store = UserStore::new();
        // 1. User requests a token.
        let t = store.issue_token("eve@example.com").await;
        // 2. User redeems it.
        let s = store.redeem_token(&t.token).await.unwrap();
        // 3. The other features use the session to identify the user.
        let u = store.lookup_session(&s.session_token).await.unwrap();
        assert_eq!(u.email, "eve@example.com");
        // 4. The user id is the stable `owner` principal.
        assert!(u.id.starts_with("u:"));
    }

    // ---- D-010: sessions must expire, not be immortal ---------------------
    //
    // Before D-010 a `Session` had no `expires_at` and `lookup_session` did no
    // TTL check, so a leaked bearer was valid for the process lifetime. These
    // tests lock the fix: a fresh session is bounded to SESSION_TTL_DAYS, and
    // an expired session resolves to None.

    #[tokio::test]
    async fn d010_fresh_session_has_future_expiry_and_resolves() {
        let store = UserStore::new();
        let t = store.issue_token("frank@example.com").await;
        let s = store.redeem_token(&t.token).await.unwrap();
        // The minted session must expire in the future (specifically ~30d out).
        assert!(
            s.expires_at > Utc::now(),
            "fresh session must expire in the future"
        );
        let max_ttl = Duration::days(SESSION_TTL_DAYS) + Duration::seconds(5);
        assert!(
            s.expires_at - s.created_at <= max_ttl,
            "session TTL must be bounded to ~{} days",
            SESSION_TTL_DAYS
        );
        // And it must still resolve.
        assert!(store.lookup_session(&s.session_token).await.is_some());
    }

    #[tokio::test]
    async fn d010_expired_session_resolves_to_none() {
        let store = UserStore::new();
        // Provision a user + a real session, then back-date the expiry so the
        // TTL check fires. We can't fast-forward the clock, so plant a session
        // whose expires_at is already in the past.
        store.issue_token("grace@example.com").await;
        let user_id = user_id_for("grace@example.com");
        let expired = Session {
            session_token: "stale-bearer".into(),
            user_id,
            created_at: Utc::now() - Duration::days(SESSION_TTL_DAYS + 1),
            expires_at: Utc::now() - Duration::seconds(1), // expired 1s ago
        };
        store.plant_session_for_test(expired).await;
        // An expired session must NOT resolve — even though the user exists.
        assert!(
            store.lookup_session("stale-bearer").await.is_none(),
            "D-010: expired bearer must not resolve"
        );
    }

    #[tokio::test]
    async fn d010_far_future_default_keeps_legacy_sessions_alive() {
        // A Session deserialized without `expires_at` (legacy blob) gets the
        // far_future default and must still resolve. Guards back-compat.
        let store = UserStore::new();
        store.issue_token("heidi@example.com").await;
        let legacy = Session {
            session_token: "legacy-bearer".into(),
            user_id: user_id_for("heidi@example.com"),
            created_at: Utc::now(),
            expires_at: far_future(),
        };
        store.plant_session_for_test(legacy).await;
        assert!(store.lookup_session("legacy-bearer").await.is_some());
    }
}
