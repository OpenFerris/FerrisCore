//! Wiring for the one session stack: `tower-sessions` + `axum-login`.
//!
//! Both the production server and the test harness build the same two pieces
//! here, so no call site can mount one without the other or with a different
//! cookie policy. The helper returns the concrete layer types, which is the
//! reason a caller needs neither `axum-login` nor `tower-sessions` in its own
//! dependency list.

use axum_login::{AuthManagerLayer, AuthManagerLayerBuilder, AuthSession};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use time::Duration;
use tower_sessions::cookie::SameSite;
use tower_sessions::{Expiry, Session, SessionManagerLayer};

use crate::User;

use super::keys::{
    ABSOLUTE_EXPIRY_KEY, CSRF_TOKEN_KEY, EMAIL_KEY, MFA_PENDING_KEY, ROLES_KEY, USER_ID_KEY,
};
use super::{Backend, DbSessionStore, SessionConfig};

/// The `tower-sessions` layer, configured from [`SessionConfig`].
///
/// * `HttpOnly` and `SameSite=Lax` are always on: the Desk is same-origin and
///   a cross-site form must not carry the session.
/// * The expiry handed to the store is the **sliding idle window**; the hard
///   lifetime is [`ABSOLUTE_EXPIRY_KEY`], enforced when the session is read.
#[must_use]
pub fn session_layer(
    config: &SessionConfig,
    store: DbSessionStore,
) -> SessionManagerLayer<DbSessionStore> {
    let mut layer = SessionManagerLayer::new(store)
        .with_name(config.cookie_name.clone())
        .with_http_only(true)
        .with_secure(config.secure_cookies)
        .with_same_site(SameSite::Lax)
        .with_path(config.cookie_path.clone())
        .with_expiry(Expiry::OnInactivity(Duration::minutes(
            i64::try_from(config.idle_timeout_minutes.max(1)).unwrap_or(i64::MAX),
        )));
    if let Some(domain) = &config.cookie_domain {
        layer = layer.with_domain(domain.clone());
    }
    layer
}

/// The `axum-login` manager layer over [`Backend`] and the session layer.
#[must_use]
pub fn auth_layer(
    config: &SessionConfig,
    store: DbSessionStore,
    backend: Backend,
) -> AuthManagerLayer<Backend, DbSessionStore> {
    AuthManagerLayerBuilder::new(backend, session_layer(config, store)).build()
}

/// Resolve the authenticated principal from request extensions.
///
/// `None` means "no live session": the layers are not mounted, there is no
/// authenticated user, or the absolute lifetime has elapsed. In the last case
/// the session record is flushed so the stale cookie cannot be reused.
pub async fn principal_from_extensions(extensions: &axum::http::Extensions) -> Option<User> {
    let auth = extensions.get::<AuthSession<Backend>>()?;
    let session = extensions.get::<Session>()?;
    if !session_is_live(session).await {
        let _ = session.flush().await;
        return None;
    }
    Some(auth.user().await?.into_user())
}

/// Whether the session is inside its absolute lifetime.
///
/// A session that carries no [`ABSOLUTE_EXPIRY_KEY`] is **not** live. The idle
/// window alone can be refreshed forever, so the hard deadline is the only
/// thing that bounds a session's total age; accepting a record without one
/// would leave a legacy or forged session valid indefinitely. [`stamp_session`]
/// writes the deadline at login, so only a record that never went through the
/// login path lacks it, and that record is refused.
pub async fn session_is_live(session: &Session) -> bool {
    match session.get::<i64>(ABSOLUTE_EXPIRY_KEY).await {
        Ok(Some(expires_at)) => Utc::now().timestamp() < expires_at,
        _ => false,
    }
}

/// The session's CSRF token, when it has one.
pub async fn csrf_token(session: &Session) -> Option<String> {
    session.get::<String>(CSRF_TOKEN_KEY).await.ok().flatten()
}

/// The CSRF token carried by a request's session extension, if any.
///
/// A helper for layers that only have `http::Extensions` (the public website
/// forms, observability) and must not depend on `tower-sessions` themselves.
/// `None` means "no session layer, or no token": callers treat that as a failed
/// proof rather than as "nothing to protect".
pub async fn csrf_from_extensions(extensions: &axum::http::Extensions) -> Option<String> {
    csrf_token(extensions.get::<Session>()?).await
}

/// Return the session's CSRF token, creating and storing one when absent.
///
/// # Errors
///
/// `tower_sessions::session::Error` when the session cannot be read or written.
pub async fn ensure_csrf_token(
    session: &Session,
) -> std::result::Result<String, tower_sessions::session::Error> {
    if let Some(token) = session.get::<String>(CSRF_TOKEN_KEY).await? {
        return Ok(token);
    }
    let token = uuid::Uuid::new_v4().to_string();
    session.insert(CSRF_TOKEN_KEY, &token).await?;
    Ok(token)
}

/// Stamp the session with the facts the framework maintains itself.
///
/// Called once, after `axum-login` accepted the credentials and before/with
/// [`AuthSession::login`](axum_login::AuthSession::login). The absolute
/// deadline is measured from now, not from the cookie creation, so a session
/// cannot be extended past `timeout_minutes` by activity.
///
/// # Errors
///
/// `tower_sessions::session::Error` when the session cannot be written.
pub async fn stamp_session(
    session: &Session,
    user: &User,
    config: &SessionConfig,
) -> std::result::Result<(), tower_sessions::session::Error> {
    session.insert(USER_ID_KEY, user.id.as_str()).await?;
    session.insert(EMAIL_KEY, user.email.as_str()).await?;
    session.insert(ROLES_KEY, user.roles.as_slice()).await?;
    let expires_at = Utc::now()
        .timestamp()
        .saturating_add(i64::try_from(config.timeout_minutes).unwrap_or(i64::MAX) * 60);
    session.insert(ABSOLUTE_EXPIRY_KEY, expires_at).await?;
    Ok(())
}

/// A password step that still owes its second factor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMfa {
    /// The account id the password proved.
    pub user_id: String,
    /// The challenge the Desk echoes back to `/api/auth/mfa/verify`.
    pub challenge: String,
    /// When the challenge stops being valid, as a Unix timestamp in seconds.
    pub expires_at: i64,
}

impl PendingMfa {
    /// Whether the challenge has expired as of `now`.
    #[must_use]
    pub const fn is_expired(&self, now: chrono::DateTime<Utc>) -> bool {
        self.expires_at <= now.timestamp()
    }
}

/// Replace the session's pending second factor.
///
/// # Errors
///
/// `tower_sessions::session::Error` when the session cannot be written.
pub async fn set_pending_mfa(
    session: &Session,
    pending: &PendingMfa,
) -> std::result::Result<(), tower_sessions::session::Error> {
    session.insert(MFA_PENDING_KEY, pending).await
}

/// Remove and return the session's pending second factor.
///
/// Single use: the value is gone before the code is checked, so a replayed
/// challenge cannot be presented twice.
///
/// # Errors
///
/// `tower_sessions::session::Error` when the session cannot be read/written.
pub async fn take_pending_mfa(
    session: &Session,
) -> std::result::Result<Option<PendingMfa>, tower_sessions::session::Error> {
    session.remove::<PendingMfa>(MFA_PENDING_KEY).await
}
