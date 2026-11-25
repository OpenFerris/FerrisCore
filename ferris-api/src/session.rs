//! Public session endpoints: `POST /api/auth/login`, `POST /api/auth/logout`,
//! `POST /api/auth/logout-all`, `POST /api/auth/mfa/verify`,
//! `GET /api/auth/session` and `PUT /api/auth/language`.
//!
//! These are mounted **outside** the `require_auth` layer (they are the way in)
//! and read identity from `axum_login::AuthSession<Backend>`. The session
//! record and its cookie belong to `tower_sessions`; this module only decides
//! *when* to call `authenticate`, `login` and `logout`, and adds the framework's
//! per-session facts (CSRF token, absolute deadline, pending MFA).
//!
//! # The login contract the Desk expects
//!
//! * `POST /api/auth/login` with `{"username","password"}` answers either the
//!   public user JSON (session cookie + `X-CSRF-Token`) or
//!   `{"mfa_required": true, "challenge": "<id>"}` when the account has MFA.
//! * `POST /api/auth/mfa/verify` with `{"challenge","code"}` consumes the
//!   pending state from the session, verifies a TOTP or single-use backup code,
//!   and only then opens the session.
//! * Both failure paths answer `401` with the generic credential message; the
//!   handler never says which factor failed.
//!
//! # Opt-in email verification
//!
//! `[auth] require_email_verification` (default `false`) gates a login on the
//! account's `email_verified` flag. When it is on, the password login (the MFA
//! challenge, the final MFA verify and the non-MFA path) answers a `403`
//! — "verify your email address before signing in" — and opens no session. The
//! check runs *after* the credentials are proven, so it cannot be used to
//! enumerate accounts. An OAuth login whose provider reported a verified email
//! satisfies the policy and marks the local account verified.
//!
//! # Lockout
//!
//! The process-local [`RateLimiter`] and the persisted counter on the user row
//! are unchanged: a failed password/TOTP attempt stamps both, and the one place
//! every successful entry point funnels through ([`start_session`]) clears the
//! row. MFA is never "half cleared": the password step leaves the budget
//! consumed, so an attacker holding the password still has a bounded number of
//! TOTP guesses.
//!
//! # One resolution per login
//!
//! `login` reads the account **once**, through `UserStore::login_record`. That
//! record answers the lockout question, the MFA decision and the password check
//! (`Backend::authenticate_record`), so the same row is not looked up a second
//! time on the way in; an unresolved name is charged the same Argon2 work
//! through `Backend::equalise_unknown_login` so it stays indistinguishable from
//! a wrong password. The pending challenge keeps the resolved id and
//! `verify_mfa` reads that one record (identity, TOTP secret and session stamp).
//!
//! # Single-use TOTP steps
//!
//! A valid code spends its time step through `UserStore::claim_totp_step`, an
//! atomic conditional update on the user row. The claim is persisted, so a code
//! observed once is refused for the rest of its step on **every** process and
//! replica, answering "MFA code already used". Nothing is tracked in memory.
//!
//! # Failed-login record
//!
//! `ferris_core::audit` has no failed-login variant (and this crate does not
//! extend the Core taxonomy), so a refusal emits a structured
//! `tracing::warn!` carrying the resolved user id — never the password, the TOTP
//! code or even the submitted name.
//!
//! # Host mailer
//!
//! [`SessionHandlers`] carries an optional [`MailerPort`] (`with_mailer`) so the
//! recovery and invitation flows wired in a later wave send mail through the
//! same state as the rest of the auth surface. The handlers here never send
//! mail; a test leaves it `None`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Json,
    extract::{ConnectInfo, Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_login::AuthSession;
use chrono::Utc;
use ferris_core::i18n::Locale;
use ferris_core::ports::MailerPort;
use serde::Deserialize;
use tower_sessions::Session;
use tracing::warn;

use ferris_auth::login::{Backend, PendingMfa as StoredPendingMfa, SessionConfig, SessionUser};
use ferris_auth::middleware::account_lockout;
use ferris_auth::{
    DbSessionStore, MfaConfig, MfaManager, RateLimiter, StoredUser, TotpAlgorithm, TotpConfig,
    UserStore,
};
use ferris_core::Error;
use ferris_db::Db;

mod support;

pub(crate) use support::{
    INTERNAL_ERROR, client_ip, cookie_value, internal_failure, set_cookie, set_csrf_token,
    start_session,
};
use support::{
    StartSessionError, account_key, any_blocked, cleared_cookie_response, connection_ip,
    email_not_verified, internal, internal_login, internal_message, login_response, public_user,
    rate_limited, record_failure, record_success, refuse_login, unauthorized,
};

/// Identity + login plumbing for the Desk.
#[derive(Clone)]
pub struct SessionHandlers {
    /// The cookie/lifetime policy, shared with the mounted auth layer.
    pub config: SessionConfig,
    /// The one `tower-sessions` store, for "sign out everywhere".
    pub session_store: Option<DbSessionStore>,
    pub user_store: UserStore,
    /// The backend the mounted `axum-login` layer uses. Kept here so a
    /// standalone OAuth router can mount the same layer the server does.
    pub backend: Backend,
    /// Lockout keyed by the client IP and, once the submitted username is
    /// known, the account too. A failure counts against every key and a
    /// successful step clears them all.
    pub rate_limiter: RateLimiter,
    /// TOTP verification + backup code generation (the secret itself lives on
    /// the user row, not on the manager).
    pub mfa: MfaManager,
    /// The host mailer, when one is attached.
    ///
    /// Wired by the server through [`SessionHandlers::with_mailer`] for the
    /// recovery and invitation flows. The session endpoints themselves never
    /// send mail, so a test can leave it `None`.
    pub mailer: Option<Arc<dyn MailerPort>>,
    /// Whether a login refuses an account whose email was never verified.
    ///
    /// Opt-in through `[auth] require_email_verification` and `false` unless the
    /// operator asks for it. When on, the refusal happens only after the
    /// password (and the second factor, when configured) is proven, so it never
    /// reveals whether an account exists.
    pub require_email_verification: bool,
}

impl SessionHandlers {
    /// Build handlers over the same database the auth backend reads.
    #[must_use]
    pub fn new(config: SessionConfig, session_store: DbSessionStore, pool: Db) -> Self {
        let defaults = ferris_core::AuthConfig::default();
        Self {
            config,
            session_store: Some(session_store),
            user_store: UserStore::new(pool.clone()),
            backend: Backend::new(pool),
            rate_limiter: RateLimiter::new(
                defaults.max_login_attempts,
                defaults.lockout_duration_minutes * 60,
            ),
            mfa: MfaManager::new(MfaConfig {
                issuer: defaults.mfa_issuer,
                ..MfaConfig::default()
            }),
            mailer: None,
            require_email_verification: false,
        }
    }

    /// Build handlers with no store (unit tests that never call logout-all).
    #[must_use]
    pub fn without_store(config: SessionConfig, pool: Db) -> Self {
        let mut handlers = Self::new(config, DbSessionStore::new(pool.clone()), pool);
        handlers.session_store = None;
        handlers
    }

    /// Wire the rate limiter and MFA settings to the operator's `AuthConfig`.
    ///
    /// `ServerSettings` carries the resolved `ferris_core::AuthConfig`
    /// (`max_login_attempts`, `lockout_duration_minutes`, `mfa_issuer`), so this is where a deployment's policy becomes live.
    #[must_use]
    pub fn with_auth_config(mut self, auth: &ferris_core::AuthConfig) -> Self {
        self.rate_limiter =
            RateLimiter::new(auth.max_login_attempts, auth.lockout_duration_minutes * 60);
        self.mfa = MfaManager::new(MfaConfig {
            issuer: auth.mfa_issuer.clone(),
            ..MfaConfig::default()
        });
        self.require_email_verification = auth.require_email_verification;
        self
    }

    /// Attach the host mailer used by the mail-sending auth flows.
    ///
    /// The server wires the same [`MailerPort`] the email vertical uses; the
    /// session handlers only hand it on, so no credential or transport detail
    /// reaches this crate.
    #[must_use]
    pub fn with_mailer(mut self, mailer: Arc<dyn MailerPort>) -> Self {
        self.mailer = Some(mailer);
        self
    }

    /// The attached host mailer, if any.
    #[must_use]
    pub fn mailer(&self) -> Option<&Arc<dyn MailerPort>> {
        self.mailer.as_ref()
    }
}

/// `POST /api/auth/login` body.
///
/// `Debug` is manual and redacted: a handler or a test that formats the request
/// must never publish the password. The username stays, because it is the only
/// field an operator can correlate a failure with.
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct MfaVerifyRequest {
    pub challenge: String,
    pub code: String,
}

/// How long a `mfa_required` challenge stays valid.
const MFA_GRACE_SECONDS: i64 = 5 * 60;

/// The TOTP verification window, in time steps either side of now. Mirrors the
/// `window` handed to [`MfaManager::verify_totp`].
const TOTP_WINDOW: i64 = 1;

/// Whether the opt-in policy refuses `user` for an unverified email.
///
/// Read it only **after** the password (and the second factor, when configured)
/// is proven: answering "verify your email address" earlier would tell a caller
/// that the account exists.
fn email_unverified(h: &SessionHandlers, user: &StoredUser) -> bool {
    h.require_email_verification && !user.email_verified
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
    auth_session: AuthSession<Backend>,
    session: Session,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(req): Json<LoginRequest>,
) -> Response {
    if req.username.trim().is_empty() || req.password.is_empty() {
        return unauthorized("empty credentials");
    }
    let ip = connection_ip(peer.as_ref());
    // Resolve the account **once**. `login_record` carries the identity, the
    // stored password hash and the TOTP secret, so the password check below and
    // the MFA decision need no further read of the same row.
    //
    // The account key must be the stored user id, not the string the client
    // sent (AUTH-04): otherwise `Ada@Example.com` and `ada@example.com` are two
    // independent budgets, and an attacker keeps the real account open by
    // varying the spelling.
    let resolved = match h.user_store.login_record(&req.username).await {
        Ok(record) => record,
        Err(e) => return internal(e),
    };
    let account = resolved.as_ref().map(|record| account_key(&record.user.id));
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
    if let Some(record) = &resolved {
        match account_lockout(h.user_store.db(), &record.user.id).await {
            Ok(lockout) if lockout.is_locked(Utc::now()) => return rate_limited(),
            Ok(_) => {}
            Err(error) => {
                warn!(error = %error, "could not read the persisted login lockout");
                // The in-memory limiter still applies; a storage failure must
                // not be reported as "the account is not locked".
            }
        }
    }

    // An MFA-enabled account cannot be authenticated in one call: the backend
    // rightly refuses a password with no code. Verify the password on its own,
    // then store the short-lived challenge in *this* session. Nothing global,
    // nothing shared between processes.
    if let Some(record) = resolved.as_ref().filter(|record| record.user.mfa_enabled) {
        match auth_session
            .backend()
            .authenticate_record(record.clone(), &req.password)
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => {
                refuse_login(&h, Some(&record.user.id), ip.as_deref()).await;
                record_failure(&h, &keys);
                return unauthorized("invalid username or password");
            }
            Err(e) => return internal_login(axum_login::Error::Backend(e)),
        }
        // A row with `mfa_enabled` but no secret is a provisioning accident.
        // Refuse it instead of silently opening a password-only session: the
        // account asked for a second factor and cannot get one.
        if record.mfa_secret.is_none() {
            warn!(user.id = %record.user.id, "MFA is enabled without a TOTP secret; refusing the login");
            refuse_login(&h, Some(&record.user.id), ip.as_deref()).await;
            record_failure(&h, &keys);
            return unauthorized("account cannot complete two-factor login");
        }
        // The email-verification policy runs after the password is proven, so
        // the refusal tells nothing to a caller that could not authenticate:
        // no pending challenge is minted for an account that could never
        // complete the login.
        if email_unverified(&h, &record.user) {
            warn!(user.id = %record.user.id, "email verification is required; refusing the login");
            return email_not_verified();
        }
        let challenge = uuid::Uuid::new_v4().to_string();
        let pending = StoredPendingMfa {
            user_id: record.user.id.clone(),
            challenge: challenge.clone(),
            expires_at: Utc::now().timestamp() + MFA_GRACE_SECONDS,
        };
        if let Err(error) = ferris_auth::login::set_pending_mfa(&session, &pending).await {
            warn!(%error, "could not store the pending MFA challenge");
            return internal_message(INTERNAL_ERROR);
        }
        return Json(serde_json::json!({
            "mfa_required": true,
            "challenge": challenge,
        }))
        .into_response();
    }

    let user = match resolved {
        Some(record) => {
            let user_id = record.user.id.clone();
            let email_verified = record.user.email_verified;
            match auth_session
                .backend()
                .authenticate_record(record, &req.password)
                .await
            {
                Ok(Some(user)) => {
                    // The policy is checked only after the password is proven,
                    // so a caller that could not authenticate learns nothing
                    // about the account's verification state.
                    if h.require_email_verification && !email_verified {
                        warn!(user.id = %user_id, "email verification is required; refusing the login");
                        return email_not_verified();
                    }
                    user
                }
                Ok(None) => {
                    refuse_login(&h, Some(&user_id), ip.as_deref()).await;
                    record_failure(&h, &keys);
                    return unauthorized("invalid username or password");
                }
                Err(e) => return internal_login(axum_login::Error::Backend(e)),
            }
        }
        None => {
            // Nothing was resolved: the login matches no row, or the account is
            // disabled (which `login_record` treats as unresolvable). There is
            // no login to lock and no record to verify, so spend the same Argon2
            // work as a real account and refuse. Keying a failure on the
            // submitted name would let an attacker fill the store with arbitrary
            // names.
            auth_session
                .backend()
                .equalise_unknown_login(&req.password)
                .await;
            refuse_login(&h, None, ip.as_deref()).await;
            record_failure(&h, &keys);
            return unauthorized("invalid username or password");
        }
    };
    record_success(&h, &keys);
    match start_session(&auth_session, &session, &h, &user, ip).await {
        Ok(()) => login_response(&session, user.user()).await,
        Err(StartSessionError::TooManySessions(max)) => {
            crate::router::error_response(Error::Auth(ferris_core::AuthError::TooManySessions(max)))
        }
        Err(StartSessionError::Login(error)) => {
            warn!(%error, "could not open the session");
            internal_message(INTERNAL_ERROR)
        }
    }
}

/// `POST /api/auth/mfa/verify` — complete the second factor and open the real
/// session. The pending challenge is single-use: it is removed from the session
/// before the code is checked, so a replayed `challenge` fails.
pub async fn verify_mfa(
    State(h): State<SessionHandlers>,
    auth_session: AuthSession<Backend>,
    session: Session,
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
    let pending = match ferris_auth::login::take_pending_mfa(&session).await {
        Ok(Some(pending)) => pending,
        Ok(None) => {
            // The challenge id is random and one-shot, so before it resolves
            // there is no account to key on and the connection address is the
            // only signal available.
            if let Some(ip) = ip.as_deref() {
                h.rate_limiter.record_failure(ip);
            }
            return unauthorized("challenge expired or already used");
        }
        Err(error) => {
            warn!(%error, "could not read the pending MFA challenge");
            return internal_message(INTERNAL_ERROR);
        }
    };
    if !ferris_auth::crypto::constant_time_eq(
        pending.challenge.as_bytes(),
        req.challenge.as_bytes(),
    ) {
        if let Some(ip) = ip.as_deref() {
            h.rate_limiter.record_failure(ip);
        }
        return unauthorized("challenge expired or already used");
    }
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
    if pending.is_expired(Utc::now()) {
        refuse_login(&h, Some(&pending.user_id), ip.as_deref()).await;
        record_failure(&h, &keys);
        return unauthorized("challenge expired");
    }
    match account_lockout(h.user_store.db(), &pending.user_id).await {
        Ok(lockout) if lockout.is_locked(Utc::now()) => return rate_limited(),
        Ok(_) => {}
        Err(error) => warn!(error = %error, "could not read the persisted login lockout"),
    }
    // One record for the whole second step: the identity, the TOTP secret and
    // the session stamp. The password step already rehashed the stored hash (if
    // it was stale), so the stamp read here matches the row as it is now.
    let record = match h.user_store.login_record_by_id(&pending.user_id).await {
        Ok(Some(record)) if record.user.mfa_enabled => record,
        Ok(_) => {
            refuse_login(&h, Some(&pending.user_id), ip.as_deref()).await;
            record_failure(&h, &keys);
            return unauthorized("MFA is not enabled for this account");
        }
        Err(e) => return internal(e),
    };
    let user = &record.user;
    // TOTP first, then a single-use backup code (hashed at rest).
    let totp_ok = match record.mfa_secret.as_deref() {
        Some(secret) => {
            let totp = TotpConfig {
                secret: secret.to_string(),
                digits: h.mfa.totp_digits(),
                period: h.mfa.totp_period(),
                algorithm: TotpAlgorithm::Sha1,
            };
            match h.mfa.verify_totp(&totp, &req.code, TOTP_WINDOW) {
                Ok(ok) => ok,
                Err(e) => return internal(e),
            }
        }
        None => false,
    };
    if totp_ok {
        // A code stays valid for a whole step (and one step either side), so a
        // code observed once could be replayed until the clock leaves the
        // window. `claim_totp_step` is an atomic conditional `UPDATE` on the
        // account row: the second use is refused here *and* on every other
        // process or replica (AUTH-04), with nothing kept in memory.
        let step =
            u64::try_from(Utc::now().timestamp().max(0)).unwrap_or(0) / h.mfa.totp_period().max(1);
        match h.user_store.claim_totp_step(&user.id, step).await {
            Ok(true) => {}
            Ok(false) => {
                refuse_login(&h, Some(&user.id), ip.as_deref()).await;
                record_failure(&h, &keys);
                return unauthorized("MFA code already used");
            }
            Err(e) => return internal(e),
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
        refuse_login(&h, Some(&user.id), ip.as_deref()).await;
        record_failure(&h, &keys);
        return unauthorized("invalid MFA code");
    }
    // Password and second factor are proven; now the opt-in policy can refuse
    // without revealing anything to a caller that did not authenticate.
    if email_unverified(&h, &record.user) {
        warn!(user.id = %record.user.id, "email verification is required; refusing the login");
        return email_not_verified();
    }
    let session_user = SessionUser::new(record.user.to_user(), record.session_auth_hash());
    record_success(&h, &keys);
    match start_session(&auth_session, &session, &h, &session_user, ip).await {
        Ok(()) => login_response(&session, session_user.user()).await,
        Err(StartSessionError::TooManySessions(max)) => {
            crate::router::error_response(Error::Auth(ferris_core::AuthError::TooManySessions(max)))
        }
        Err(StartSessionError::Login(error)) => {
            warn!(%error, "could not open the session");
            internal_message(INTERNAL_ERROR)
        }
    }
}

/// `POST /api/auth/logout` — flush the session and clear the cookie. The route
/// carries `require_csrf` so a cross-site form cannot log a user out.
///
/// Flushing the record is the revocation: the next request with the old cookie
/// finds no session and `axum-login` refuses it. The realtime listener is told
/// so live sockets close too (RT-01).
pub async fn logout(
    State(h): State<SessionHandlers>,
    auth_session: AuthSession<Backend>,
    session: Session,
) -> Response {
    let owner = auth_session.user().await.map(|user| user.id.clone());
    let session_id = session.id().map(|id| id.to_string());
    if let Err(error) = auth_session.logout().await {
        warn!(%error, "failed to flush the session at logout");
    }
    if let Some(user) = owner {
        ferris_core::audit::record(ferris_core::audit::AuditEvent::Logout {
            user: user.clone(),
            ip: "unknown".to_string(),
        });
        ferris_auth::middleware::notify_session_revoked(&user, session_id.as_deref());
    }
    cleared_cookie_response(&h, StatusCode::NO_CONTENT)
}

/// `POST /api/auth/logout-all` — close **every** session of the signed-in user.
///
/// This is the answer to a stolen device or a compromised password: the caller
/// proves one live session, and every other session (and the socket each one
/// backs) is revoked. The caller's own session is closed too, so it is a full
/// sign-out.
pub async fn logout_all(
    State(h): State<SessionHandlers>,
    auth_session: AuthSession<Backend>,
) -> Response {
    let Some(user) = auth_session.user().await else {
        return unauthorized("not signed in");
    };
    let user_id = user.id.clone();
    match &h.session_store {
        Some(store) => {
            if let Err(error) = store.delete_user_sessions(&user_id).await {
                warn!(%error, "failed to delete every session of the user");
                return internal_message(INTERNAL_ERROR);
            }
        }
        None => {
            // Without the shared store the only session the handler can prove
            // is the one it is holding; still refuse to claim a global logout.
            warn!("logout-all called without a session store");
            return internal_message(INTERNAL_ERROR);
        }
    }
    // The store already removed this session; the manager's in-memory record is
    // flushed as well so the response cannot resurrect it.
    let _ = auth_session.logout().await;
    // One notification with no session id means "every socket of this user".
    ferris_auth::middleware::notify_session_revoked(&user_id, None);
    cleared_cookie_response(&h, StatusCode::NO_CONTENT)
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
    auth_session: AuthSession<Backend>,
    Json(req): Json<SetLanguageRequest>,
) -> Response {
    let Some(user) = auth_session.user().await else {
        return unauthorized("not signed in");
    };
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
    match h.user_store.set_language(&user.id, value).await {
        Ok(()) => Json(serde_json::json!({ "language": value })).into_response(),
        Err(error) => internal(error),
    }
}

/// `GET /api/auth/session` — the authenticated user, or `401` when there is no
/// live session.
///
/// Also hands back the live `X-CSRF-Token` the client echoes on mutating
/// requests, creating one if this session predates the token.
pub async fn session(auth_session: AuthSession<Backend>, session: Session) -> Response {
    if !ferris_auth::login::session_is_live(&session).await {
        // A record with no absolute deadline is not an authenticated session,
        // but it can be a pending MFA challenge that has not been stamped yet.
        // Only revoke a session that carried a deadline and passed it: flushing
        // an unstamped record would drop the challenge and the account could
        // never finish the second factor.
        let expired = matches!(
            session
                .get::<i64>(ferris_auth::login::keys::ABSOLUTE_EXPIRY_KEY)
                .await,
            Ok(Some(_))
        );
        if expired {
            let _ = session.flush().await;
        }
        return unauthorized("session expired");
    }
    let Some(user) = auth_session.user().await else {
        return unauthorized("not signed in");
    };
    let csrf = match ferris_auth::login::ensure_csrf_token(&session).await {
        Ok(token) => token,
        Err(error) => {
            warn!(%error, "could not read the session CSRF token");
            return internal_message(INTERNAL_ERROR);
        }
    };
    let mut response = Json(public_user(&user)).into_response();
    set_csrf_token(&mut response, &csrf);
    response
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{LoginRequest, SessionConfig, SessionHandlers};

    /// A mailer that accepts every message: the handlers only store the port.
    struct NoopMailer;

    impl ferris_core::ports::MailerPort for NoopMailer {
        fn send(&self, _message: &ferris_core::ports::EmailMessage) -> ferris_core::Result<()> {
            Ok(())
        }
    }

    /// A formatted login request must keep the username and never the password.
    #[test]
    fn login_request_debug_redacts_the_password() {
        let request = LoginRequest {
            username: "ada@example.com".to_string(),
            password: "hunter2".to_string(),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("ada@example.com"), "{debug}");
        assert!(!debug.contains("hunter2"), "{debug}");
    }

    /// The host mailer is optional until the server wires one.
    #[tokio::test]
    async fn the_host_mailer_is_optional_and_replaceable() {
        let pool = ferris_db::memory_db().await.unwrap();
        let handlers = SessionHandlers::without_store(SessionConfig::default(), pool);
        assert!(handlers.mailer().is_none());
        let handlers = handlers.with_mailer(Arc::new(NoopMailer));
        assert!(handlers.mailer().is_some());
    }
}
