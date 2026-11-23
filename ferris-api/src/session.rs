//! Public session endpoints: `POST /api/auth/login`, `POST /api/auth/logout`,
//! `POST /api/auth/mfa/verify`, `GET /api/auth/session`.
//!
//! These are mounted **outside** the `require_auth` layer (they are the way in),
//! so the handlers resolve identity from the session cookie directly. The
//! shared [`SessionManager`] is the same instance the auth middleware reads, so
//! a login here is immediately visible to every guarded route.
//!
//! Since the 2026-10-08 re-audit, the login flow is the *roadmap gate* for the security work:
//!
//! - **Rate limiting** (A19): only failed attempts count towards a lockout
//!   (the [`RateLimiter`] counts failures, and a successful password step clears
//!   the key).
//! - **MFA** (M6): an account with MFA enabled answers the password step with
//!   `{"mfa_required": true, "challenge": <uuid>}`; the real session is only
//!   created by `POST /api/auth/mfa/verify` once a TOTP code or single-use
//!   backup code checks out.
//! - **CSRF** (A20): login and session responses expose the per-session token
//!   in the `X-CSRF-Token` header, which the desk client echoes back on
//!   mutating requests (`ferris-web/src/api/client.rs`); the server validates
//!   it in `ferris_auth::middleware::require_csrf`.
//!
//! Two deliberate single-process assumptions are recorded here rather than
//! fixed, because a single-node install is the supported shape today and both
//! are cheap to externalise later: the pending-MFA map below, and
//! the notification bus the login pushes through
//! ([`crate::notifications`]). Scaling the API past one process needs a shared
//! store for each.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Json,
    extract::{ConnectInfo, Extension, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ferris_core::i18n::Locale;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::warn;

use ferris_auth::middleware::{account_lockout, clear_account_lockout, record_account_failure};
use ferris_auth::{
    CSRF_HEADER, MfaConfig, MfaManager, RateLimiter, SessionConfig, SessionManager, TotpAlgorithm,
    TotpConfig, UserStore,
};
use ferris_core::{Error, Value};
use ferris_db::Db;

/// Identity + session plumbing for the Desk login flow.
#[derive(Clone)]
pub struct SessionHandlers {
    pub config: SessionConfig,
    pub session_manager: SessionManager,
    pub user_store: UserStore,
    /// Lockout keyed by the client IP and, once the submitted username is
    /// known, the account too. A failure counts against every key and a
    /// successful step clears them all.
    pub rate_limiter: RateLimiter,
    /// TOTP verification + backup code generation (the secret itself lives on
    /// the user row, not on the manager).
    pub mfa: MfaManager,
    /// Single-use login challenges awaiting their second factor.
    ///
    /// Process-local by design: a challenge minted here is visible
    /// only to this process, so a horizontally scaled deployment must move it
    /// to a shared store (or pin the client to one instance) before
    /// `mfa_required` works across instances.
    pub pending_mfa: Arc<Mutex<HashMap<String, PendingMfa>>>,
    /// The last TOTP time step spent by each account (AUTH-04/RT).
    ///
    /// A TOTP code is valid for a whole step (and, through the verification
    /// window, for the step either side), so without a record of what was
    /// already used an observed code can be replayed. Keyed by the resolved
    /// user id, never by the submitted name.
    pub totp_steps: Arc<Mutex<HashMap<String, u64>>>,
}

/// A login that passed the password check but still owes its second factor.
#[derive(Clone)]
pub struct PendingMfa {
    pub user_id: String,
    pub expires_at: DateTime<Utc>,
}

/// How long a `mfa_required` challenge stays valid.
const MFA_GRACE_SECONDS: i64 = 5 * 60;

/// The TOTP verification window, in time steps either side of now. Mirrors the
/// `window` handed to [`MfaManager::verify_totp`].
const TOTP_WINDOW: i64 = 1;

impl SessionHandlers {
    /// Build handlers sharing the auth middleware's session store.
    pub fn new(config: SessionConfig, session_manager: SessionManager, pool: Db) -> Self {
        // AUTH-03: `cleanup_expired` had no caller, so expired rows piled up
        // until the concurrent-session cap counted them as live. The manager is
        // shared by every handler and the middleware, so starting the sweeper
        // here covers the whole process; the first tick also sweeps at boot.
        let _ = session_manager.spawn_cleanup_task(std::time::Duration::from_secs(600));
        let defaults = ferris_core::AuthConfig::default();
        Self {
            config,
            session_manager,
            user_store: UserStore::new(pool),
            rate_limiter: RateLimiter::new(
                defaults.max_login_attempts,
                defaults.lockout_duration_minutes * 60,
            ),
            mfa: MfaManager::new(MfaConfig {
                issuer: defaults.mfa_issuer,
                ..MfaConfig::default()
            }),
            pending_mfa: Arc::new(Mutex::new(HashMap::new())),
            totp_steps: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Wire the rate limiter and MFA settings to the operator's `AuthConfig`.
    ///
    /// `ServerSettings` carries the resolved `ferris_core::AuthConfig`
    /// (`max_login_attempts`, `lockout_duration_minutes`, `mfa_enabled`,
    /// `mfa_issuer`), so this is where a deployment's policy becomes live.
    #[must_use]
    pub fn with_auth_config(mut self, auth: &ferris_core::AuthConfig) -> Self {
        self.rate_limiter =
            RateLimiter::new(auth.max_login_attempts, auth.lockout_duration_minutes * 60);
        self.mfa = MfaManager::new(MfaConfig {
            enabled: auth.mfa_enabled,
            issuer: auth.mfa_issuer.clone(),
            ..MfaConfig::default()
        });
        self
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct MfaVerifyRequest {
    pub challenge: String,
    pub code: String,
}

/// `POST /api/auth/login` — validate credentials, open a session (or demand
/// the second factor), set the session cookie and answer with the user's
/// public identity plus the session's `X-CSRF-Token`.
///
/// The address recorded for the lockout and the session row is the **TCP
/// peer** (`ConnectInfo`), never a forwarding header (API-06): see
/// [`connection_ip`].
pub async fn login(
    State(h): State<SessionHandlers>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(req): Json<LoginRequest>,
) -> Response {
    if req.username.trim().is_empty() || req.password.is_empty() {
        return unauthorized("empty credentials");
    }
    let ip = connection_ip(peer.as_ref());
    // Resolve the account before the lockout decision. The account key must be
    // the stored user id, not the string the client sent (AUTH-04): otherwise
    // `Ada@Example.com` and `ada@example.com` are two independent budgets, and
    // an attacker keeps the real account open by varying the spelling.
    let known = match h.user_store.find_by_login(&req.username).await {
        Ok(user) => user,
        Err(e) => return internal(e),
    };
    let account = known.as_ref().map(|user| account_key(&user.id));
    let mut keys: Vec<&str> = Vec::with_capacity(2);
    // Without a verified connection address there is no IP key at all: a shared
    // "unknown" bucket would let one attacker's failures lock everyone out.
    if let Some(ip) = ip.as_deref() {
        keys.push(ip);
    }
    if let Some(account) = account.as_deref() {
        keys.push(account);
    }
    if any_blocked(&h, &keys) {
        return rate_limited();
    }
    // The row is the cross-restart, cross-replica half of the lockout. It is
    // consulted before Argon2 so a locked account costs one read, not a hash.
    if let Some(user) = &known {
        match account_lockout(h.user_store.db(), &user.id).await {
            Ok(lockout) if lockout.is_locked(Utc::now()) => return rate_limited(),
            Ok(_) => {}
            Err(error) => {
                warn!(error = %error, "could not read the persisted login lockout");
                // The in-memory limiter still applies; a storage failure must
                // not be reported as "the account is not locked".
            }
        }
    }
    let user = match h
        .user_store
        .verify_login(&req.username, &req.password)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => {
            // Only a resolved account has a row to stamp: an unknown login has
            // nothing to lock, and keying it by the submitted string would let
            // an attacker fill the store with arbitrary names.
            if let Some(known) = &known {
                stamp_failure(&h, &known.id).await;
            }
            record_failure(&h, &keys);
            return unauthorized("invalid username or password");
        }
        Err(e) => return internal(e),
    };
    if !user.enabled {
        stamp_failure(&h, &user.id).await;
        record_failure(&h, &keys);
        return unauthorized("account disabled");
    }
    if user.mfa_enabled {
        // The account owes a second factor: never open a session here. If the
        // secret is gone (a partially-enabled row), fall through to a normal
        // session rather than locking the user out entirely.
        match h.user_store.mfa_secret(&user.id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                record_success(&h, &keys);
                return open_session(&h, user, ip).await;
            }
            Err(e) => return internal(e),
        }
        // Deliberately **not** cleared here (AUTH-04): the password step is only
        // half a login, and clearing the budget before the second factor is
        // exactly how an attacker with the password gets unlimited TOTP
        // guesses. `verify_mfa` clears it once the code is accepted, and
        // `start_session` clears the persisted counter.
        let challenge = uuid::Uuid::new_v4().to_string();
        {
            let mut pending = h.pending_mfa.lock().await;
            let now = Utc::now();
            pending.retain(|_, p| p.expires_at > now);
            pending.insert(
                challenge.clone(),
                PendingMfa {
                    user_id: user.id.clone(),
                    expires_at: now + ChronoDuration::seconds(MFA_GRACE_SECONDS),
                },
            );
        }
        return Json(serde_json::json!({
            "mfa_required": true,
            "challenge": challenge,
        }))
        .into_response();
    }
    record_success(&h, &keys);
    open_session(&h, user, ip).await
}

/// `POST /api/auth/mfa/verify` — complete the second factor and open the real
/// session. The challenge is single-use (removed before validation), so a
/// replayed `state`/`challenge` fails.
pub async fn verify_mfa(
    State(h): State<SessionHandlers>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(req): Json<MfaVerifyRequest>,
) -> Response {
    if req.challenge.trim().is_empty() || req.code.trim().is_empty() {
        return unauthorized("missing challenge or code");
    }
    let ip = connection_ip(peer.as_ref());
    if let Some(ip) = ip.as_deref()
        && h.rate_limiter.is_blocked(ip)
    {
        return rate_limited();
    }
    let challenge = req.challenge.clone();
    let pending = h.pending_mfa.lock().await.remove(&challenge);
    let Some(pending) = pending else {
        // The challenge id is random and one-shot, so before it resolves there
        // is no account to key on and the connection address is the only
        // signal available.
        if let Some(ip) = ip.as_deref() {
            h.rate_limiter.record_failure(ip);
        }
        return unauthorized("challenge expired or already used");
    };
    // From here the account is known, so a guess also counts against the
    // resolved id rather than the string the client sent (AUTH-04).
    let account = account_key(&pending.user_id);
    let mut keys: Vec<&str> = Vec::with_capacity(2);
    if let Some(ip) = ip.as_deref() {
        keys.push(ip);
    }
    keys.push(&account);
    if any_blocked(&h, &keys) {
        return rate_limited();
    }
    if pending.expires_at < Utc::now() {
        stamp_failure(&h, &pending.user_id).await;
        record_failure(&h, &keys);
        return unauthorized("challenge expired");
    }
    match account_lockout(h.user_store.db(), &pending.user_id).await {
        Ok(lockout) if lockout.is_locked(Utc::now()) => return rate_limited(),
        Ok(_) => {}
        Err(error) => warn!(error = %error, "could not read the persisted login lockout"),
    }
    let user = match h.user_store.find_by_id(&pending.user_id).await {
        Ok(Some(u)) if u.mfa_enabled => u,
        Ok(_) => {
            stamp_failure(&h, &pending.user_id).await;
            record_failure(&h, &keys);
            return unauthorized("MFA is not enabled for this account");
        }
        Err(e) => return internal(e),
    };
    // TOTP first, then a single-use backup code (hashed at rest).
    let totp_ok = match h.user_store.mfa_secret(&user.id).await {
        Ok(Some(secret)) => {
            let totp = TotpConfig {
                secret,
                digits: h.mfa.totp_digits(),
                period: h.mfa.totp_period(),
                algorithm: TotpAlgorithm::Sha1,
            };
            match h.mfa.verify_totp(&totp, &req.code, TOTP_WINDOW) {
                Ok(ok) => ok,
                Err(e) => return internal(e),
            }
        }
        Ok(None) => false,
        Err(e) => return internal(e),
    };
    if totp_ok {
        // A code stays valid for a whole step (and one step either side), so a
        // code observed once can be replayed until the clock leaves the
        // window. Spending the step here—keyed by the resolved account—makes
        // the second use a failure (AUTH-04).
        if !claim_totp_step(&h, &user.id).await {
            stamp_failure(&h, &user.id).await;
            record_failure(&h, &keys);
            return unauthorized("MFA code already used");
        }
    }
    let ok = totp_ok
        || match h
            .user_store
            .consume_mfa_backup_code(&user.id, &req.code)
            .await
        {
            Ok(consumed) => consumed,
            Err(e) => return internal(e),
        };
    if !ok {
        stamp_failure(&h, &user.id).await;
        record_failure(&h, &keys);
        return unauthorized("invalid MFA code");
    }
    record_success(&h, &keys);
    open_session(&h, user, ip).await
}

/// `POST /api/auth/logout` — delete the session and clear the cookie. The
/// route carries `require_csrf` so a cross-site form cannot log a user out.
///
/// Deleting the session is the real revocation: the stored row disappears, the
/// manager marks it revoked so the signed JWT minted for it stops
/// authenticating (AUTH-05), and the registered realtime listener closes the
/// sockets that were opened with it (RT-01).
pub async fn logout(State(h): State<SessionHandlers>, headers: HeaderMap) -> Response {
    if let Some(sid) = session_cookie(&h.config, &headers) {
        if let Err(e) = h.session_manager.delete_session(&sid).await {
            warn!(error = %e, "failed to delete session at logout");
        }
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(&mut response, &h.config.clear_cookie().to_string());
    response
}

/// `POST /api/auth/logout-all` — close **every** session of the signed-in user.
///
/// This is the answer to a stolen device or a compromised password: the
/// caller proves one live session, and every other session (plus the JWT and
/// socket each one backs) is revoked (AUTH-05). The caller's own session is
/// closed too, so it is a full sign-out.
///
/// The route is not registered in this crate: the composition root mounts the
/// auth endpoints (`ferris-server/src/router/build.rs`, next to
/// `/api/auth/logout`) and must add it there. The handler is public so that the
/// wiring change is the only thing left.
pub async fn logout_all(State(h): State<SessionHandlers>, headers: HeaderMap) -> Response {
    let Some(sid) = session_cookie(&h.config, &headers) else {
        return unauthorized("not signed in");
    };
    let Ok(Some(session)) = h.session_manager.get_session(&sid).await else {
        return unauthorized("session expired");
    };
    if let Err(e) = h
        .session_manager
        .delete_user_sessions(&session.user_id)
        .await
    {
        return internal(e);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(&mut response, &h.config.clear_cookie().to_string());
    response
}

/// `PUT /api/auth/language` — persist the caller's chosen locale.
///
/// The Desk sends the normalized tag from its language picker; an empty string
/// clears the preference so the account follows `Accept-Language` again. The
/// route is mounted with the CSRF gate, like logout: it changes the account.
#[derive(Debug, Deserialize)]
pub struct SetLanguageRequest {
    pub language: String,
}

pub async fn set_language(
    State(h): State<SessionHandlers>,
    headers: HeaderMap,
    Json(req): Json<SetLanguageRequest>,
) -> Response {
    let Some(sid) = session_cookie(&h.config, &headers) else {
        return unauthorized("not signed in");
    };
    let Ok(Some(s)) = h.session_manager.touch_session(&sid).await else {
        return unauthorized("session expired");
    };
    if s.is_expired() || s.is_idle_expired(h.config.idle_timeout_minutes) {
        return unauthorized("session expired");
    }
    if !matches!(h.user_store.find_by_id(&s.user_id).await, Ok(Some(_))) {
        return unauthorized("not signed in");
    }
    let raw = req.language.trim();
    let language = if raw.is_empty() {
        None
    } else {
        match Locale::parse(raw) {
            Some(locale) => Some(locale),
            None => return (StatusCode::BAD_REQUEST, "invalid language tag").into_response(),
        }
    };
    let value = language.as_ref().map(Locale::as_str);
    match h.user_store.set_language(&s.user_id, value).await {
        Ok(()) => Json(serde_json::json!({ "language": value })).into_response(),
        Err(error) => internal(error),
    }
}

/// `GET /api/auth/session` — the authenticated user, or `401` when the cookie
/// is missing/expired.
///
/// Also hands back the live `X-CSRF-Token` the client echoes on mutating
/// requests.
pub async fn session(State(h): State<SessionHandlers>, headers: HeaderMap) -> Response {
    let Some(sid) = session_cookie(&h.config, &headers) else {
        return unauthorized("not signed in");
    };
    let Ok(Some(s)) = h.session_manager.touch_session(&sid).await else {
        return unauthorized("session expired");
    };
    if s.is_expired() || s.is_idle_expired(h.config.idle_timeout_minutes) {
        return unauthorized("session expired");
    }
    let csrf = s.csrf_token.clone();
    // Prefer the users table (source of truth for name/roles); fall back to the
    // identity captured on the session when the user row is gone.
    if let Ok(Some(user)) = h.user_store.find_by_id(&s.user_id).await {
        let mut response = Json(user.public_json()).into_response();
        set_csrf_token(&mut response, &csrf);
        return response;
    }
    let email = s
        .get_data("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let roles = s
        .get_data("roles")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let user = Value::from(serde_json::json!({
        "id": s.user_id.clone(),
        "email": email,
        "username": null,
        "full_name": if email.is_empty() { s.user_id.clone() } else { email },
        "roles": roles,
        "user_type": "SystemUser",
    }));
    let mut response = Json(user).into_response();
    set_csrf_token(&mut response, &csrf);
    response
}

/// The shared tail of login and MFA verify: record the login, push the welcome
/// notification, open the session and answer with the cookie + CSRF header.
async fn open_session(
    h: &SessionHandlers,
    user: ferris_auth::StoredUser,
    ip: Option<String>,
) -> Response {
    let public = user.public_json();
    match start_session(h, &user, ip).await {
        Ok(session) => {
            let cookie = h.config.build_cookie(&session.id).to_string();
            let mut response = Json(public).into_response();
            set_cookie(&mut response, &cookie);
            set_csrf_token(&mut response, &session.csrf_token);
            response
        }
        Err(e) => internal(e),
    }
}

/// Open the session of an **already authenticated** user.
///
/// This is the one definition of "what a login does" after the identity is
/// proven: stamp the audit columns, push the welcome notification and create the
/// session. The password step, the MFA step and the OAuth callback all go
/// through it, so no entry point can forget the audit stamp or the notification
/// — which is exactly what the OAuth flow did while it answered with provider
/// tokens instead of a session.
///
/// # Errors
///
/// Storage errors from creating the session (propagated: without a session there
/// is no login). Recording the login is best-effort and only logged.
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
pub(crate) async fn start_session(
    h: &SessionHandlers,
    user: &ferris_auth::StoredUser,
    ip: Option<String>,
) -> ferris_core::Result<ferris_auth::Session> {
    if let Err(e) = h.user_store.record_login(&user.id).await {
        warn!(error = %e, "failed to record login");
    }
    // The whole login succeeded (password, and the second factor when the
    // account has one), so the persisted lockout counter is cleared here — the
    // one place every entry point goes through (AUTH-04).
    if let Err(e) = clear_account_lockout(h.user_store.db(), &user.id).await {
        warn!(error = %e, "failed to clear the login lockout");
    }
    // An empty address is "nothing was recorded"; a real address (the peer, or
    // the OAuth flow's literal `"unknown"`) is kept as it came.
    let ip = ip.filter(|ip| !ip.trim().is_empty());
    let full_name = user.full_name.clone();
    let user_id = user.id.clone();
    if crate::panel::notify_on_login(h.user_store.db(), &user_id).await {
        if let Err(e) = crate::panel::push_notification(
            h.user_store.db(),
            &user_id,
            &format!("Welcome back, {full_name}"),
            "Signed in to Ferris Desk",
            "/desk",
        )
        .await
        {
            warn!(error = %e, "failed to push welcome notification");
        }
    }
    h.session_manager.create_session(user_id, ip, None).await
}

/// The address to record and key the lockout on: the TCP peer of the
/// connection, when the listener published one.
///
/// `X-Forwarded-For` and `X-Real-IP` are **not** consulted (API-06). They are
/// client-controlled, so accepting them let a caller choose the address the
/// lockout and the session row recorded — and rotate it to reset the per-IP
/// half of the lockout. The trusted-proxy decision itself lives in
/// `ferris_server::observability::trusted_client_ip`, which this crate cannot
/// call (the dependency points from the server to the API), so no proxy list is
/// duplicated here: a request contributes only its own connection address.
///
/// Under the in-process TLS listener the peer is the server's `TlsClientInfo`,
/// whose type `ferris-api` cannot name, so the extractor finds nothing and the
/// answer is `None` — fail closed. The account half of the lockout still
/// applies; there is simply no IP key rather than a spoofable one.
fn connection_ip(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> Option<String> {
    let peer = peer?;
    Some(peer.0.ip().to_string())
}

/// Best-effort client IP for a request whose TCP peer is not available.
///
/// This is the OAuth callback's entry point, which receives only the headers.
/// It cannot be answered from `X-Forwarded-For`/`X-Real-IP` for the reason
/// [`connection_ip`] documents, so it reports the neutral `"unknown"`: the
/// session row then says the address was not verifiable, instead of repeating a
/// value the caller chose. The callback's own request does carry the peer at the
/// listener; wiring it through would need the same extractor `login` uses and is
/// left to that route (see `auditoria/parches/E1b.md`).
pub(crate) fn client_ip(_headers: &HeaderMap) -> String {
    "unknown".to_string()
}

/// The lockout key for one account, derived from the **resolved** user id.
///
/// Keying on the id (not the submitted username) means the budget belongs to
/// the account: case, whitespace and email-vs-username spellings all land on
/// the same key, and a failed guess is counted against the real row (AUTH-04).
/// The `user:` namespace keeps it from ever colliding with a client-IP key.
fn account_key(user_id: &str) -> String {
    format!("user:{}", user_id.trim().to_ascii_lowercase())
}

/// Spend the current TOTP step for `user_id`, answering `false` when the step
/// (or one inside the verification window) was already used.
///
/// `verify_totp` accepts the step either side of now, so a code observed at
/// step `S` remains valid while the clock is at `S+1`. Recording `S + window`
/// makes the milestone the first step at which the code can no longer be
/// presented, which is what turns "replay" into a refusal instead of a second
/// login a minute later.
async fn claim_totp_step(h: &SessionHandlers, user_id: &str) -> bool {
    let step =
        u64::try_from(Utc::now().timestamp().max(0)).unwrap_or(0) / h.mfa.totp_period().max(1);
    claim_totp_step_at(h, user_id, step).await
}

/// [`claim_totp_step`] with the step supplied, so a test never has to wait for
/// the clock to advance.
async fn claim_totp_step_at(h: &SessionHandlers, user_id: &str, step: u64) -> bool {
    let mut steps = h.totp_steps.lock().await;
    if steps.get(user_id).is_some_and(|last| *last >= step) {
        return false;
    }
    steps.insert(
        user_id.to_string(),
        step.saturating_add(u64::try_from(TOTP_WINDOW).unwrap_or(0)),
    );
    true
}

/// Best-effort persisted failure counter (AUTH-04/AUTH-16).
///
/// The in-memory limiter is the fast path; the row is what a restart and the
/// other replicas read. A storage failure is logged, never allowed to turn a
/// failed login into a success.
async fn stamp_failure(h: &SessionHandlers, user_id: &str) {
    if let Err(error) = record_account_failure(
        h.user_store.db(),
        user_id,
        h.rate_limiter.max_attempts(),
        h.rate_limiter.lockout_minutes(),
    )
    .await
    {
        warn!(error = %error, "failed to persist the login lockout");
    }
}

/// True when any of `keys` is locked out.
fn any_blocked(h: &SessionHandlers, keys: &[&str]) -> bool {
    keys.iter().any(|key| h.rate_limiter.is_blocked(key))
}

/// Record one failure against every key.
fn record_failure(h: &SessionHandlers, keys: &[&str]) {
    for key in keys {
        h.rate_limiter.record_failure(key);
    }
}

/// Clear every key after a successful step.
fn record_success(h: &SessionHandlers, keys: &[&str]) {
    for key in keys {
        h.rate_limiter.record_success(key);
    }
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(crate::ApiError::new("unauthorized", message)),
    )
        .into_response()
}

fn rate_limited() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(crate::ApiError::new(
            "rate_limited",
            "too many failed attempts; try again later",
        )),
    )
        .into_response()
}

// One cookie parser for the whole surface: the auth crate owns it, so the REST
// API, the OAuth flow cookies and the realtime handshake cannot drift.
pub(crate) use ferris_auth::middleware::cookie_value;

/// Pull the session id out of a raw `Cookie` header.
pub(crate) fn session_cookie(config: &SessionConfig, headers: &HeaderMap) -> Option<String> {
    ferris_auth::middleware::session_cookie(config, headers)
}

/// Append one `Set-Cookie` value.
///
/// `append`, not `insert`: a response can carry several cookies (the OAuth
/// callback sets the session and deletes the two flow cookies), and `insert`
/// would silently drop all but the last.
pub(crate) fn set_cookie(response: &mut Response, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        response.headers_mut().append(header::SET_COOKIE, v);
    }
}

/// Expose the session's CSRF token to the client.
pub(crate) fn set_csrf_token(response: &mut Response, token: &str) {
    if let Ok(v) = HeaderValue::from_str(token) {
        response.headers_mut().insert(CSRF_HEADER, v);
    }
}

/// A 500 whose body does not describe the failure.
///
/// The error is logged with its full text (that is where the operator looks);
/// the client gets [`INTERNAL_ERROR`] and nothing else, because the text carries
/// storage details, table names and wrapped causes.
fn internal(e: Error) -> Response {
    // The concurrent-session cap is a client-visible refusal, not a server
    // fault: it has an HTTP status of its own and the message tells the user
    // to close another tab. Collapsing it into the anonymous 500 hid the
    // reason and made the Desk's re-login dialog show "internal error" for a
    // session limit the user could actually fix.
    if matches!(e, Error::Auth(ferris_core::AuthError::TooManySessions(_))) {
        return crate::router::error_response(e);
    }
    tracing::error!(error = %e, "session endpoint failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(crate::ApiError::new("internal", INTERNAL_ERROR)),
    )
        .into_response()
}

/// The one message a 5xx may say.
pub(crate) const INTERNAL_ERROR: &str = "internal error";

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderName;

    /// The concurrent-session cap is a 429 with the server's message, not the
    /// generic 500: the re-login dialog shows exactly this response.
    #[test]
    fn too_many_sessions_is_a_429_not_an_internal_error() {
        let response = internal(Error::Auth(ferris_core::AuthError::TooManySessions(5)));
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn extracts_session_cookie() {
        let config = SessionConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; ferris_session=abc123; x=y"),
        );
        assert_eq!(session_cookie(&config, &headers).as_deref(), Some("abc123"));

        let empty = HeaderMap::new();
        assert_eq!(session_cookie(&config, &empty), None);
    }

    /// API-06: a forwarding header is client-controlled, so it must never
    /// become the address the session records or the lockout keys on.
    #[test]
    fn forwarding_headers_are_not_a_source_of_client_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("203.0.113.9, 10.0.0.2"),
        );
        headers.insert(
            HeaderName::from_static("x-real-ip"),
            HeaderValue::from_static("10.0.0.2"),
        );
        // The neutral answer, not the header's first hop: the caller chooses
        // the header, so it cannot name the address that gets recorded.
        assert_eq!(client_ip(&headers), "unknown");
        assert_eq!(client_ip(&HeaderMap::new()), "unknown");
    }

    /// The lockout address is the peer the listener published, and there is
    /// none when it published nothing — never a fallback the caller can pick.
    #[test]
    fn the_lockout_address_is_the_connection_peer() {
        let peer = Extension(ConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4242))));
        assert_eq!(connection_ip(Some(&peer)).as_deref(), Some("203.0.113.9"));
        assert_eq!(connection_ip(None), None);
    }

    /// A 500 never carries the internal text: it can name tables, paths or a
    /// wrapped cause, and the login endpoint is unauthenticated.
    #[tokio::test]
    async fn internal_errors_do_not_leak_the_reason() {
        let response = internal(Error::Database(ferris_core::error::DatabaseError::Query(
            "select * from tab_user where password = 'x'".to_string(),
        )));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], INTERNAL_ERROR);
        assert!(!text.contains("tab_user"), "{text}");
    }

    /// AUTH-04: a TOTP step is spent once. `verify_totp` accepts the step
    /// either side of now, so the guard records the first step at which the
    /// code can no longer be replayed.
    #[tokio::test]
    async fn a_totp_step_cannot_be_replayed() {
        let pool = ferris_db::memory_db().await.unwrap();
        let config = SessionConfig::default();
        let h = SessionHandlers::new(config.clone(), SessionManager::new(config), pool);

        assert!(claim_totp_step_at(&h, "user-1", 100).await);
        assert!(
            !claim_totp_step_at(&h, "user-1", 100).await,
            "the same step is a replay"
        );
        assert!(
            !claim_totp_step_at(&h, "user-1", 101).await,
            "the acceptance window still covers the code for one more step"
        );
        assert!(
            claim_totp_step_at(&h, "user-1", 102).await,
            "past the window a fresh code is accepted again"
        );
        // Another account is unaffected.
        assert!(claim_totp_step_at(&h, "user-2", 100).await);
    }

    /// AUTH-05: `logout-all` closes every session of the caller, not just the
    /// one presented.
    #[tokio::test]
    async fn logout_all_closes_every_session_of_the_caller() {
        let pool = ferris_db::memory_db().await.unwrap();
        let config = SessionConfig::default();
        let manager = SessionManager::new(config.clone());
        let first = manager
            .create_session("multi@example.com".to_string(), None, None)
            .await
            .unwrap();
        let second = manager
            .create_session("multi@example.com".to_string(), None, None)
            .await
            .unwrap();
        let other = manager
            .create_session("other@example.com".to_string(), None, None)
            .await
            .unwrap();
        let h = SessionHandlers::new(config.clone(), manager.clone(), pool);

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{}={}", config.cookie_name, first.id)).unwrap(),
        );
        let response = logout_all(State(h), headers).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        assert!(manager.get_session(&first.id).await.unwrap().is_none());
        assert!(manager.get_session(&second.id).await.unwrap().is_none());
        assert!(
            manager.get_session(&other.id).await.unwrap().is_some(),
            "another account's sessions are untouched"
        );
    }

    /// The rate limiter counts failures per key and clears on success — the
    /// contract the login handler relies on.
    #[tokio::test]
    async fn with_auth_config_keeps_a_working_lockout_default() {
        let pool = ferris_db::memory_db().await.unwrap();
        let h = SessionHandlers::new(
            SessionConfig::default(),
            SessionManager::new(SessionConfig::default()),
            pool,
        )
        .with_auth_config(&ferris_core::AuthConfig {
            max_login_attempts: 2,
            lockout_duration_minutes: 15,
            ..ferris_core::AuthConfig::default()
        });
        assert!(!h.rate_limiter.is_blocked("203.0.113.9"));
        h.rate_limiter.record_failure("203.0.113.9");
        assert!(!h.rate_limiter.is_blocked("203.0.113.9"));
        h.rate_limiter.record_failure("203.0.113.9");
        assert!(h.rate_limiter.is_blocked("203.0.113.9"));
        h.rate_limiter.record_success("203.0.113.9");
        assert!(!h.rate_limiter.is_blocked("203.0.113.9"));
    }
}
