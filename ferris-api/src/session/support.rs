//! Shared plumbing for the session endpoints.
//!
//! The handlers live in [`super`]; this module holds the pieces they all use —
//! the "open a session" tail, the lockout keys, the structured failure log and
//! the small response constructors — so `session.rs` stays readable. Items are
//! `pub(crate)`; the ones other modules need are re-exported from [`super`].

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Extension},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use ferris_auth::login::{Backend, SessionUser};
use ferris_auth::middleware::{clear_account_lockout, record_account_failure};
use ferris_core::{Error, Value};
use tower_sessions::Session;
use tracing::warn;

use super::SessionHandlers;

/// Why [`start_session`] refused to open a session.
#[derive(Debug)]
pub(crate) enum StartSessionError {
    /// The `tower-sessions`/`axum-login` write failed.
    Login(axum_login::Error<Backend>),
    /// The account already holds the configured number of sessions.
    TooManySessions(u32),
}

impl From<axum_login::Error<Backend>> for StartSessionError {
    fn from(error: axum_login::Error<Backend>) -> Self {
        Self::Login(error)
    }
}

impl std::fmt::Display for StartSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Login(error) => write!(f, "session write failed: {error}"),
            Self::TooManySessions(max) => {
                write!(f, "maximum of {max} concurrent sessions reached")
            }
        }
    }
}

/// The shared tail of login and MFA verify: stamp the audit columns, push the
/// welcome notification, write the session's framework facts and log in.
///
/// This is the one definition of "what a login does" after the identity is
/// proven. The password step, the MFA step and the OAuth callback all go
/// through it, so no entry point can forget the audit stamp, the notification,
/// the absolute deadline or the CSRF token.
///
/// # Errors
///
/// `axum_login::Error` when the session record cannot be written. Recording the
/// login is best-effort and only logged.
pub(crate) async fn start_session(
    auth_session: &axum_login::AuthSession<Backend>,
    session: &Session,
    h: &SessionHandlers,
    user: &SessionUser,
    ip: Option<String>,
) -> Result<(), StartSessionError> {
    // Refuse a login beyond the configured concurrent-session cap, the same
    // rule the old store enforced: an extra login must not silently take an
    // older tab's session away. The current record is excluded when it already
    // belongs to this account (a re-login in the same browser).
    let max = h.config.max_sessions_per_user;
    if max > 0 {
        let store = h.session_store.as_ref();
        if let Some(store) = store {
            let active = store.count_user_sessions(&user.id).await.unwrap_or(0);
            let current_is_this_user = match session.id() {
                Some(id) => matches!(
                    store.session_user_id(&id.to_string()).await,
                    Ok(Some(owner)) if owner == user.id
                ),
                None => false,
            };
            let held = active.saturating_sub(u64::from(current_is_this_user));
            if held >= u64::from(max) {
                return Err(StartSessionError::TooManySessions(max));
            }
        }
    }
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
    // Every entry point (password, second factor, OAuth) lands here, so this is
    // the one place the `Activity Log` row cannot be forgotten. The host's sink
    // writes it asynchronously; no sink in a test is a no-op.
    ferris_core::audit::record(ferris_core::audit::AuditEvent::Login {
        user: user.id.clone(),
        ip: ip.clone().unwrap_or_else(|| "unknown".to_string()),
    });
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
    // Framework facts first; `login` then inserts its own identity and saves
    // the same record.
    ferris_auth::login::stamp_session(session, user.user(), &h.config)
        .await
        .map_err(|error| StartSessionError::Login(axum_login::Error::Session(error)))?;
    ferris_auth::login::ensure_csrf_token(session)
        .await
        .map_err(|error| StartSessionError::Login(axum_login::Error::Session(error)))?;
    auth_session.login(user).await.map_err(Into::into)
}

/// The JSON body of a successful login/session response.
pub(crate) async fn login_response(session: &Session, user: &ferris_core::User) -> Response {
    let csrf = match ferris_auth::login::csrf_token(session).await {
        Some(token) => token,
        None => {
            warn!("the session has no CSRF token after login");
            return internal_message(INTERNAL_ERROR);
        }
    };
    let mut response = Json(public_user(user)).into_response();
    set_csrf_token(&mut response, &csrf);
    // The cookie is written by `tower-sessions`; the absolute deadline is
    // already in the record.
    response
}

/// Portable identity the Desk renders. Mirrors `StoredUser::public_json`.
pub(super) fn public_user(user: &ferris_core::User) -> Value {
    Value::from(serde_json::json!({
        "id": user.id,
        "email": user.email,
        "username": user.username,
        "full_name": user.full_name,
        "roles": user.roles,
        "user_type": user.user_type,
        "language": user.language,
    }))
}

/// The address to record and key the lockout on: the TCP peer of the
/// connection, when the listener published one.
///
/// `X-Forwarded-For` and `X-Real-IP` are **not** consulted (API-06). They are
/// client-controlled, so accepting them let a caller choose the address the
/// lockout and the session row recorded — and rotate it to reset the per-IP
/// half of the lockout.
pub(super) fn connection_ip(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> Option<String> {
    let peer = peer?;
    Some(peer.0.ip().to_string())
}

/// Best-effort client IP for a request whose TCP peer is not available.
///
/// The OAuth callback's entry point receives only the headers and cannot be
/// answered from `X-Forwarded-For`/`X-Real-IP` for the reason [`connection_ip`]
/// documents, so it reports the neutral `"unknown"`.
pub(crate) fn client_ip(_headers: &HeaderMap) -> String {
    "unknown".to_string()
}

/// The lockout key for one account, derived from the **resolved** user id.
///
/// Keying on the id (not the submitted username) means the budget belongs to
/// the account: case, whitespace and email-vs-username spellings all land on
/// the same key, and a failed guess is counted against the real row (AUTH-04).
/// The `user:` namespace keeps it from ever colliding with a client-IP key.
pub(super) fn account_key(user_id: &str) -> String {
    format!("user:{}", user_id.trim().to_ascii_lowercase())
}

/// Record one refused login: the structured audit warning plus, for a resolved
/// account, the persisted failure counter.
///
/// An unresolved login (the submitted name matches no row) has nothing to
/// stamp: keying it by the submitted string would let an attacker fill the
/// store with arbitrary names.
pub(crate) async fn refuse_login(h: &SessionHandlers, user_id: Option<&str>, ip: Option<&str>) {
    audit_failed_login(user_id, ip);
    if let Some(id) = user_id {
        stamp_failure(h, id).await;
    }
}

/// Emit the structured record of a refused login.
///
/// The event carries the resolved user id (when there is one) and the peer
/// address, and nothing secret — no password, no TOTP code, not even the
/// submitted username. `Activity Log` gets a `login_failed` row through the
/// host's audit sink.
fn audit_failed_login(user_id: Option<&str>, ip: Option<&str>) {
    let user = user_id.unwrap_or("unknown").to_string();
    let ip = ip.unwrap_or("unknown").to_string();
    ferris_core::audit::record(ferris_core::audit::AuditEvent::LoginFailed {
        user: user.clone(),
        ip: ip.clone(),
    });
    match user_id {
        Some(id) => warn!(event = "login_failed", user.id = %id, ip = %ip, "login refused"),
        None => warn!(
            event = "login_failed",
            ip = %ip,
            "login refused for an unresolved account"
        ),
    }
}

/// Best-effort persisted failure counter (AUTH-04/AUTH-16).
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
pub(super) fn any_blocked(h: &SessionHandlers, keys: &[&str]) -> bool {
    keys.iter().any(|key| h.rate_limiter.is_blocked(key))
}

/// Record one failure against every key.
pub(super) fn record_failure(h: &SessionHandlers, keys: &[&str]) {
    for key in keys {
        h.rate_limiter.record_failure(key);
    }
}

/// Clear every key after a successful step.
pub(super) fn record_success(h: &SessionHandlers, keys: &[&str]) {
    for key in keys {
        h.rate_limiter.record_success(key);
    }
}

pub(super) fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(crate::ApiError::new("unauthorized", message)),
    )
        .into_response()
}

/// The opt-in refusal for an account whose email was never verified.
///
/// Only answered **after** the password (and the second factor, when
/// configured) is proven, so the message cannot enumerate accounts.
pub(super) fn email_not_verified() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(crate::ApiError::new(
            "email_not_verified",
            "verify your email address before signing in",
        )),
    )
        .into_response()
}

pub(super) fn rate_limited() -> Response {
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
        response.headers_mut().insert(ferris_auth::CSRF_HEADER, v);
    }
}

/// A `204`/other empty success that also expires the session cookie.
pub(super) fn cleared_cookie_response(h: &SessionHandlers, status: StatusCode) -> Response {
    let mut response = status.into_response();
    set_cookie(&mut response, &h.config.clear_cookie());
    response
}

/// A 500 whose body does not describe the failure.
pub(super) fn internal(e: Error) -> Response {
    // The concurrent-session cap is a client-visible refusal, not a server
    // fault: it has an HTTP status of its own and the message tells the user
    // the reason (kept for compatibility with the old session store).
    if matches!(e, Error::Auth(ferris_core::AuthError::TooManySessions(_))) {
        return crate::router::error_response(e);
    }
    tracing::error!(error = %e, "session endpoint failed");
    internal_message(INTERNAL_ERROR)
}

/// A 500 from a `tower-sessions`/`axum-login` failure.
pub(super) fn internal_login(error: axum_login::Error<Backend>) -> Response {
    tracing::error!(error = %error, "login backend failed");
    internal_message(INTERNAL_ERROR)
}

/// A 500 whose body is the one generic message.
pub(super) fn internal_message(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(crate::ApiError::new("internal", message)),
    )
        .into_response()
}

/// A 500 whose body is the one generic message.
pub(crate) fn internal_failure() -> Response {
    internal_message(INTERNAL_ERROR)
}

/// The one message a 5xx may say.
pub(crate) const INTERNAL_ERROR: &str = "internal error";
