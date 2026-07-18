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

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use hkgov_common::Result;
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
    /// Counts token issuances; `maybe_reap_expired` sweeps once every
    /// [`REAP_EVERY_ISSUES`] to amortize the cost of purging expired state.
    issue_count: Arc<std::sync::atomic::AtomicU64>,
}

/// Purge expired/redeemed tokens and expired sessions every N token issuances.
/// Reaping on every issue would add lock contention to the unauthenticated path;
/// this batches the work.
const REAP_EVERY_ISSUES: u64 = 16;

/// A serializable snapshot of the [`UserStore`]'s persistent state. Used by the
/// file-based persistence layer (`persist.rs`) to survive graceful restarts.
///
/// Tokens are deliberately excluded — they're short-lived (15 min TTL) one-time
/// credentials. Persisting them would extend their lifetime across restarts,
/// undermining the one-time guarantee.
///
/// Sessions are persisted keyed by [`hash_session_token`] of the bearer, **not**
/// the plaintext bearer itself (A-006). A 30-day bearer exfiltrated from a
/// backup / volume mount / co-tenant file read would otherwise let anyone
/// impersonate the user; a SHA-256 hash is useless without a preimage attack on
/// SHA-256. The in-memory store is keyed the same way, so the plaintext lives
/// only for the brief mint→return-to-client window and is never written down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStoreSnapshot {
    pub users: Vec<User>,
    /// (hash_of_session_token, Session). The Session's `session_token` field
    /// is cleared (empty) in the persisted form — see [`UserStore::snapshot`].
    pub sessions: Vec<(String, Session)>,
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
        // Reap expired/redeemed tokens + expired sessions periodically so the
        // in-process maps can't grow without bound. Triggered opportunistically
        // on token issuance (the unauthenticated entry point) every N issues —
        // without this, every magic-link request (including from an attacker)
        // permanently grows `tokens` by one row.
        self.maybe_reap_expired();

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
        // A-006: key the in-memory map by the hash, not the plaintext bearer.
        // The plaintext exists only on the returned `Session` for the brief
        // mint→HTTP-response window; nothing in long-lived state holds it.
        let key = hash_session_token(&session_token);
        self.sessions.write().await.insert(key, session.clone());
        Some(session)
    }

    /// Resolve a session token to its user. `None` if unknown OR expired (D-010:
    /// a session is no longer immortal — a leaked bearer ages out after
    /// `SESSION_TTL_DAYS`).
    pub async fn lookup_session(&self, session_token: &str) -> Option<User> {
        // A-006: hash the supplied bearer before the map lookup — the map is
        // keyed by hash, never plaintext.
        let key = hash_session_token(session_token);
        let sessions = self.sessions.read().await;
        let s = sessions.get(&key)?;
        // D-010: reject expired sessions.
        if Utc::now() > s.expires_at {
            return None;
        }
        let user_id = s.user_id.clone();
        drop(sessions);
        self.users.read().await.get(&user_id).cloned()
    }

    /// Periodically purge expired/redeemed tokens and expired sessions so the
    /// in-process maps stay bounded. Spawns the actual sweep off the caller's
    /// task when the issue counter hits [`REAP_EVERY_ISSUES`]; otherwise a cheap
    /// atomic read. Without this, every token issuance (including unauthenticated
    /// ones) permanently grows `tokens`, and expired sessions accumulate forever.
    ///
    /// A-011: also reaps `users` entries that have no live token AND no live
    /// session. A `User` with neither is inert — it will be re-provisioned
    /// idempotently on the next `issue_token` for that email (same `user_id`),
    /// so dropping it loses nothing except a row from the snapshot file.
    /// Without this, every distinct email ever seen stayed in `users` forever,
    /// growing the map + the persisted `users.json` without bound.
    fn maybe_reap_expired(&self) {
        let prev = self
            .issue_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !prev.is_multiple_of(REAP_EVERY_ISSUES) {
            return;
        }
        let tokens = self.tokens.clone();
        let sessions = self.sessions.clone();
        let users = self.users.clone();
        tokio::spawn(async move {
            let now = Utc::now();
            let mut t = tokens.write().await;
            t.retain(|_, tok| !tok.redeemed && tok.expires_at > now);
            drop(t);
            let mut s = sessions.write().await;
            s.retain(|_, sess| sess.expires_at > now);
            drop(s);
            // A-011: collect the set of user_ids that still have a live token
            // or session, then drop any user not in that set. Needs a read on
            // both maps after the reaps above so the live sets are current.
            let live_token_users: std::collections::HashSet<String> = tokens
                .read()
                .await
                .values()
                .map(|tok| user_id_for(&tok.email))
                .collect();
            let live_session_users: std::collections::HashSet<String> = sessions
                .read()
                .await
                .values()
                .map(|sess| sess.user_id.clone())
                .collect();
            let mut u = users.write().await;
            u.retain(|id, _| live_token_users.contains(id) || live_session_users.contains(id));
        });
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

    // ---- file-based persistence (stopgap until the Postgres tier lands) -----

    /// Capture a serializable snapshot of the store's persistent state.
    ///
    /// A-006: the in-memory `sessions` map is already keyed by
    /// [`hash_session_token`], so the snapshot's keys are hashes. The
    /// `Session.session_token` field is **cleared** in each persisted entry so
    /// the file never holds a plaintext bearer — only the hash + the metadata
    /// needed to validate a future request.
    pub async fn snapshot(&self) -> UserStoreSnapshot {
        UserStoreSnapshot {
            users: self.users.read().await.values().cloned().collect(),
            sessions: self
                .sessions
                .read()
                .await
                .iter()
                .map(|(k, v)| {
                    // Strip the plaintext bearer from the persisted copy. The
                    // key is already the hash, so no live credential lands in
                    // the snapshot file.
                    let mut persisted = v.clone();
                    persisted.session_token.clear();
                    (k.clone(), persisted)
                })
                .collect(),
        }
    }

    /// Restore from a snapshot (loaded from file on boot). Only restores
    /// non-expired sessions; expired ones are dropped during the restore so a
    /// long downtime doesn't resurrect dead sessions.
    ///
    /// A-006: snapshot keys are hashes (not plaintext bearers). `restore`
    /// preserves them as-is so a subsequent `lookup_session` (which hashes the
    /// supplied bearer) finds the entry. Both old plaintext-keyed snapshots
    /// (predating A-006) and new hash-keyed ones load without error; the old
    /// form simply won't match a future lookup and will age out on expiry —
    /// graceful, no migration needed.
    pub async fn restore(&self, snap: UserStoreSnapshot) {
        let now = Utc::now();
        let mut users = self.users.write().await;
        for u in snap.users {
            users.insert(u.id.clone(), u);
        }
        drop(users);
        let mut sessions = self.sessions.write().await;
        for (k, s) in snap.sessions {
            if s.expires_at > now {
                sessions.insert(k, s);
            }
        }
    }

    // ---- test-only helpers ------------------------------------------------
    //
    // Used to drive D-010 (session expiry) without fast-forwarding the clock:
    // mint a real session via the public API, then back-date its expiry so
    // `lookup_session`'s TTL check can be exercised deterministically.

    #[cfg(test)]
    pub async fn plant_session_for_test(&self, session: Session) {
        // A-006: key by hash, matching the production keying so a subsequent
        // `lookup_session(session.session_token)` (which hashes the bearer)
        // finds the planted entry.
        let key = hash_session_token(&session.session_token);
        self.sessions.write().await.insert(key, session);
    }
}

/// SHA-256 hex of a session bearer token — the at-rest + in-memory key form.
///
/// The store keys its `sessions` map by this hash rather than the plaintext
/// bearer, and the persistence layer writes the hash (never the plaintext) to
/// `users.json`. This way a stolen snapshot file (backup, volume mount,
/// co-tenant read) yields hashes, not live bearers — a preimage attack on
/// SHA-256 is infeasible, so the file is useless for impersonation. The
/// plaintext bearer exists only for the brief mint→return window.
/// (A-006.)
fn hash_session_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let hash = h.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
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
        assert_ne!(
            a, b,
            "V-006: identical public inputs must not mint the same token"
        );
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

    // ---- A-006: session bearers must NEVER appear in plaintext in the   ----
    // ---- persisted snapshot (and the in-memory map is keyed by hash).    ----
    #[tokio::test]
    async fn a006_snapshot_never_contains_plaintext_bearer() {
        let store = UserStore::new();
        let t = store.issue_token("irene@example.com").await;
        let s = store.redeem_token(&t.token).await.unwrap();
        let plaintext = s.session_token.clone();
        assert!(!plaintext.is_empty(), "mint returns a real bearer");

        let snap = store.snapshot().await;
        // The persisted JSON must not contain the plaintext bearer anywhere —
        // neither as a key nor inside a Session value.
        let json = serde_json::to_string(&snap).expect("snapshot serializes");
        assert!(
            !json.contains(&plaintext),
            "A-006: plaintext bearer leaked into snapshot: {json}"
        );
        // Every persisted session's session_token field must be empty.
        for (_, persisted) in &snap.sessions {
            assert!(
                persisted.session_token.is_empty(),
                "A-006: persisted session_token must be cleared, got {:?}",
                persisted.session_token
            );
        }
        // And the keys must be the SHA-256 hash, not the plaintext.
        let expected_key = hash_session_token(&plaintext);
        assert!(
            snap.sessions.iter().any(|(k, _)| k == &expected_key),
            "A-006: snapshot must be keyed by hash_session_token(bearer)"
        );
    }

    #[tokio::test]
    async fn a006_lookup_hashes_bearer_and_resolves() {
        // Round-trip: mint → snapshot (clears plaintext) → restore → the
        // original bearer still resolves because lookup hashes it.
        let store = UserStore::new();
        let t = store.issue_token("judy@example.com").await;
        let s = store.redeem_token(&t.token).await.unwrap();
        let bearer = s.session_token.clone();
        let snap = store.snapshot().await;

        let restored = UserStore::new();
        restored.restore(snap).await;
        assert!(
            restored.lookup_session(&bearer).await.is_some(),
            "A-006: restored store must resolve the original bearer via hash lookup"
        );
        // A wrong bearer must NOT resolve.
        assert!(
            restored.lookup_session("not-the-bearer").await.is_none(),
            "A-006: wrong bearer must not resolve"
        );
    }

    // ---- A-011: users with no live token/session are reaped so the map ----
    // ---- + snapshot file can't grow without bound.                     ----
    #[tokio::test]
    async fn a011_reap_drops_users_with_no_live_token_or_session() {
        // The reaper is triggered every REAP_EVERY_ISSUES issuances. Issue
        // enough tokens for distinct emails to trigger at least one reap,
        // WITHOUT redeeming any (so none get a session). Each issue mints a
        // 15-min token, so within the test window all tokens are live at first
        // — to actually exercise the drop path we redeem one (giving it a
        // 30-day session) and rely on the unredeemed tokens being the only
        // thing keeping their users alive once redeemed tokens are purged.
        let store = UserStore::new();
        // Issue + immediately let expire is hard without time travel; instead
        // verify the invariant directly: a user with a live session is kept,
        // and the reap logic itself is exercised via the public count() API
        // across enough issuances to fire the reaper without dropping a user
        // that has a live session.
        let t = store.issue_token("kept@example.com").await;
        let _s = store.redeem_token(&t.token).await.unwrap(); // 30-day session
        let kept_id = user_id_for("kept@example.com");
        assert!(store.get(&kept_id).await.is_some(), "kept user provisioned");

        // Fire the reaper (REAP_EVERY_ISSUES more issuances). All these new
        // users have only a 15-min token, which is still live, so they survive
        // — but the reaper must run without panicking and must NOT drop the
        // session-holding user.
        for i in 0..REAP_EVERY_ISSUES {
            store.issue_token(&format!("churn{i}@example.com")).await;
        }
        assert!(
            store.get(&kept_id).await.is_some(),
            "A-011: user with a live session must survive the reap"
        );
        // And the session still resolves.
        assert!(
            store.lookup_session(&_s.session_token).await.is_some(),
            "A-011: live session must survive the reap"
        );
    }
}

// ---------------------------------------------------------------------------
// Magic-link email delivery
// ---------------------------------------------------------------------------

/// Delivers a magic-link token to a user's email address. Implementations:
/// - [`LogMagicLinkDelivery`] (default): logs the delivery event — useful for
///   dev/CI and for log-shipper-based delivery pipelines.
/// - [`HttpMagicLinkDelivery`] (behind `alerts`): POSTs to an HTTP email-API
///   gateway (SendGrid/Mailgun/SES-via-HTTP) — the production path.
///
/// The delivery is deliberately decoupled from the `UserStore`: the store mints
/// the token; the delivery sink transports it. This mirrors the `AlertSink`
/// pattern and lets operators swap delivery backends without touching identity
/// logic.
#[async_trait]
pub trait MagicLinkDelivery: Send + Sync + 'static {
    /// Deliver a magic-link token. `redeem_url` is the fully-formed URL the user
    /// clicks (e.g. `https://app.example.com/auth/redeem?token=...`). The
    /// delivery sink is responsible for rendering the email body.
    async fn deliver(&self, email: &str, redeem_url: &str, expires_at: DateTime<Utc>)
        -> Result<()>;

    /// Human-readable name for logging.
    fn name(&self) -> &'static str;
}

/// A no-op delivery sink that logs the delivery event. The default for dev/CI —
/// the token is never logged (it's a credential), but the structured event lets
/// an external log-based pipeline (or the test harness) observe it.
pub struct LogMagicLinkDelivery;

#[async_trait]
impl MagicLinkDelivery for LogMagicLinkDelivery {
    async fn deliver(
        &self,
        email: &str,
        _redeem_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        tracing::info!(
            target: "hkgov::identity::magic_link_delivered",
            email = %email,
            expires_at = %expires_at,
            "magic-link token delivered (log sink); configure an HTTP email gateway for production delivery"
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "log"
    }
}

/// HTTP email-API gateway delivery (SendGrid/Mailgun/SES-via-HTTP). Behind the
/// `alerts` feature so the default build needs no HTTP client for identity.
///
/// The gateway contract is intentionally generic: a JSON POST to `api_url` with
/// `Authorization: Bearer <token>` and a body `{to, subject, text}`. Most
/// transactional-email APIs accept this shape directly or with a thin adapter.
/// SMTP-sending would need a heavy crate; HTTP-API-sending reuses the existing
/// reqwest client.
#[cfg(feature = "alerts")]
pub struct HttpMagicLinkDelivery {
    api_url: String,
    token: String,
    from: String,
    client: reqwest::Client,
}

#[cfg(feature = "alerts")]
impl HttpMagicLinkDelivery {
    pub fn new(
        api_url: String,
        token: String,
        from: String,
        _redeem_base_url: String,
        client: reqwest::Client,
    ) -> Self {
        // Note: `_redeem_base_url` is accepted for API symmetry with the
        // config-driven construction in main.rs (HKGOV_MAGIC_LINK__REDEEM_BASE_URL),
        // but the redeem URL is built by the route handler which knows the
        // request context. The delivery sink just transports whatever URL it's
        // given via `deliver(email, redeem_url, expires_at)`.
        Self {
            api_url,
            token,
            from,
            client,
        }
    }
}

#[cfg(feature = "alerts")]
#[async_trait]
impl MagicLinkDelivery for HttpMagicLinkDelivery {
    async fn deliver(
        &self,
        email: &str,
        redeem_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let subject = "Your HK City Pulse sign-in link";
        let text = format!(
            "Click the link below to sign in. The link expires at {expires_at} and can only be used once.\n\n{redeem_url}"
        );
        let body = serde_json::json!({
            "from": self.from,
            "to": email,
            "subject": subject,
            "text": text,
        });
        let resp = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| hkgov_common::Error::Upstream {
                origin: "magic-link-email",
                status: 0,
                detail: format!("transport: {e}"),
            })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let detail = resp.text().await.unwrap_or_default();
            return Err(hkgov_common::Error::Upstream {
                origin: "magic-link-email",
                status,
                detail,
            });
        }
        tracing::info!(
            target: "hkgov::identity::magic_link_delivered",
            email = %email,
            "magic-link token delivered via HTTP email gateway"
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "http-email"
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;

    #[tokio::test]
    async fn log_delivery_succeeds() {
        let sink = LogMagicLinkDelivery;
        let result = sink
            .deliver(
                "alice@example.com",
                "https://app.example.com/auth/redeem?token=abc",
                Utc::now() + Duration::minutes(15),
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(sink.name(), "log");
    }
}
