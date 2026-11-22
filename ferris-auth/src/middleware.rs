use crate::{CorsPolicy, JwtManager, SessionManager, User};
use async_trait::async_trait;
/// Authentication middleware for Axum
use axum::{
    extract::{FromRef, FromRequestParts, Request, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use std::result::Result as StdResult;
use std::sync::Arc;

/// Validates a user against the source of truth (typically the database).
///
/// Returning `None` (or a disabled user) rejects the request, so a token
/// issued before an account was disabled stops working immediately.
#[async_trait]
pub trait UserValidator: Send + Sync {
    async fn validate(&self, user_id: &str) -> Option<User>;
}

/// Authentication state
#[derive(Clone)]
pub struct AuthState {
    pub jwt_manager: JwtManager,
    pub session_manager: SessionManager,
    pub config: crate::session::SessionConfig,
    /// Optional source-of-truth check. When set, claims/session roles are not
    /// trusted and disabled users are rejected.
    pub user_validator: Option<Arc<dyn UserValidator>>,
    /// Optional API-key verifier. When set, the middleware accepts the Frappe
    /// `Authorization: token <key>:<secret>` scheme (AUTH2); without it that
    /// header is simply not a credential.
    pub api_keys: Option<Arc<crate::api_key::ApiKeyManager>>,
}

impl AuthState {
    pub fn new(
        jwt_manager: JwtManager,
        session_manager: SessionManager,
        config: crate::session::SessionConfig,
    ) -> Self {
        Self {
            jwt_manager,
            session_manager,
            config,
            user_validator: None,
            api_keys: None,
        }
    }

    pub fn with_user_validator(mut self, validator: Arc<dyn UserValidator>) -> Self {
        self.user_validator = Some(validator);
        self
    }

    /// Enable the `Authorization: token <key>:<secret>` API-key scheme.
    #[must_use]
    pub fn with_api_keys(mut self, manager: Arc<crate::api_key::ApiKeyManager>) -> Self {
        self.api_keys = Some(manager);
        self
    }
}

/// Extract authenticated user
#[derive(Clone)]
pub struct AuthenticatedUser(pub User);

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::WWW_AUTHENTICATE, "Bearer"),
        ],
        "Authentication required",
    )
        .into_response()
}

/// Resolve the user from the source of truth when a validator is configured,
/// otherwise fall back to the provided claims/session data.
async fn resolve_user(
    auth: &AuthState,
    user_id: &str,
    fallback_email: String,
    fallback_roles: Vec<String>,
) -> Option<User> {
    if let Some(validator) = &auth.user_validator {
        let user = validator.validate(user_id).await?;
        if !user.enabled {
            return None;
        }
        return Some(user);
    }
    Some(User {
        id: user_id.to_string(),
        email: fallback_email.clone(),
        full_name: fallback_email,
        user_type: crate::user::UserType::SystemUser,
        enabled: true,
        roles: fallback_roles,
        ..Default::default()
    })
}

/// Resolve the authenticated principal from a request, if any.
///
/// This is **the** credential-resolution decision: it tries the `Authorization`
/// header (Bearer JWT, then the Frappe `token <key>:<secret>` API-key scheme),
/// then the session cookie, and applies the source-of-truth check in every
/// case. `AuthenticatedUser` and [`require_auth`] both call it, so an added
/// credential source or a changed expiry rule cannot silently apply to only one
/// of them — the duplication that let the two drift (AUTH10).
///
/// `None` means "not authenticated": no usable credential, an invalid/expired
/// token, or a user the validator rejected.
pub async fn resolve_principal(headers: &HeaderMap, auth: &AuthState) -> Option<User> {
    if let Some(auth_header) = headers.get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
    {
        // Bearer JWT.
        if let Ok(token) = JwtManager::extract_token(auth_str)
            && let Ok(claims) = auth.jwt_manager.validate_access_token(&token)
            && token_session_is_live(auth, &claims).await
        {
            return resolve_user(
                auth,
                &claims.sub,
                claims.email.clone(),
                claims.roles.clone(),
            )
            .await;
        }

        // Frappe `token <key>:<secret>` API key (AUTH2). Only tried when a
        // manager is configured, so the header is not a credential otherwise.
        if let Some(manager) = &auth.api_keys
            && let Some(key) = extract_api_key(auth_str, manager).await
        {
            // The key names its owner; resolve that user through the same
            // source of truth, so a disabled account is still refused.
            return resolve_user(auth, &key.user, String::new(), Vec::new()).await;
        }
    }

    // Session cookie (uses the configured cookie name).
    let jar = CookieJar::from_headers(headers);
    if let Some(cookie) = jar.get(&auth.config.cookie_name)
        && let Ok(Some(session)) = auth.session_manager.get_session(cookie.value()).await
        && !session.is_expired()
        && !session.is_idle_expired(auth.config.idle_timeout_minutes)
    {
        let email = session
            .get_data("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let roles = session
            .get_data("roles")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        return resolve_user(auth, &session.user_id, email, roles).await;
    }

    None
}

/// Verify that the session a bearer token names has not been revoked.
///
/// `validate_access_token` proves the signature and the expiry, but a JWT is
/// stateless: `logout` cannot un-sign it. The token therefore names its
/// `session_id` and this check asks the shared [`SessionManager`] whether that
/// session still exists (AUTH-05), so deleting the session also kills the
/// token. A token minted without a session (`session_id` empty) is left to the
/// signature and the user check.
///
/// A storage failure fails closed: an unverifiable revocation must not grant
/// access, and the reason is logged so it is not mistaken for a plain 401.
async fn token_session_is_live(auth: &AuthState, claims: &crate::jwt::Claims) -> bool {
    match auth
        .session_manager
        .session_allows_token(&claims.session_id, &claims.sub)
        .await
    {
        Ok(live) => live,
        Err(error) => {
            tracing::warn!(error = %error, "session revocation check failed; refusing the token");
            false
        }
    }
}

/// Verify an `Authorization: token <key>:<secret>` header against the API-key
/// manager, returning the key on success.
///
/// Returns `None` for a header that is not the `token` scheme, for credentials
/// that do not verify, and for a storage failure — in every case the caller
/// falls through to the next credential source (a session cookie may still
/// authenticate the request). A storage failure is logged so it is not silently
/// indistinguishable from "wrong secret".
async fn extract_api_key(
    auth_str: &str,
    manager: &crate::api_key::ApiKeyManager,
) -> Option<crate::api_key::ApiKey> {
    let credentials = auth_str
        .strip_prefix("token ")
        .or_else(|| auth_str.strip_prefix("Token "))?
        .trim();
    match manager.authenticate(credentials).await {
        Ok(Some(key)) => Some(key),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!("API key verification failed: {error}");
            None
        }
    }
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    AuthState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let auth = AuthState::from_ref(state);
        resolve_principal(&parts.headers, &auth)
            .await
            .map(Self)
            .ok_or_else(unauthorized)
    }
}

/// Optional authenticated user (for public endpoints that can benefit from auth)
pub struct OptionalUser(pub Option<User>);

impl<S> FromRequestParts<S> for OptionalUser
where
    AuthState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        // Try to authenticate but don't fail if not authenticated
        if let Ok(auth_user) = AuthenticatedUser::from_request_parts(parts, state).await {
            return Ok(OptionalUser(Some(auth_user.0)));
        }
        Ok(OptionalUser(None))
    }
}

/// Require authentication middleware
pub async fn require_auth(
    State(state): State<AuthState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> StdResult<Response, Response> {
    if resolve_principal(request.headers(), &state).await.is_some() {
        return Ok(next.run(request).await);
    }
    Err(unauthorized())
}

/// CORS headers for the auth endpoints, from an explicit allow-list.
///
/// [`CorsPolicy`] owns the decision (see its module docs for why reflecting the
/// request's `Origin` was a hole); this middleware only applies it. Mount it
/// with `from_fn_with_state(policy, auth_cors)`, where the policy was built from
/// the deployment's `enable_cors` + `cors_origins`.
///
/// Without a policy it emits nothing, which is the browser's default:
/// same-origin only.
pub async fn auth_cors(
    State(policy): State<CorsPolicy>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut response = next.run(request).await;
    for (name, value) in policy.headers_for(origin.as_deref()) {
        response.headers_mut().insert(name, value);
    }
    response
}

/// The header carrying the CSRF token on cookie-authenticated mutating requests.
pub const CSRF_HEADER: HeaderName = HeaderName::from_static("x-csrf-token");

/// Read one cookie value out of a raw `Cookie` header.
///
/// One parser for every layer that authenticates from a cookie (the REST API,
/// the OAuth flow cookies, the realtime handshake): a second reader is a second
/// place for `name=` to be treated as a credential or for a name suffix to
/// match. An empty value reads as absent.
#[must_use]
pub fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let all = headers.get(header::COOKIE)?.to_str().ok()?;
    all.split(';').find_map(|part| {
        let mut it = part.trim().splitn(2, '=');
        if it.next()?.trim() != name {
            return None;
        }
        let value = it.next()?.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The session id in `headers`, or `None` when the cookie is absent/empty.
#[must_use]
pub fn session_cookie(
    config: &crate::session::SessionConfig,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    cookie_value(headers, &config.cookie_name)
}

/// Reject a mutating request whose CSRF check failed.
fn csrf_forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, "no-store")],
        "CSRF token missing or invalid",
    )
        .into_response()
}

/// Pull the session id out of a raw `Cookie` header.
fn session_id_from_cookie(
    config: &crate::session::SessionConfig,
    headers: &HeaderMap,
) -> Option<String> {
    let jar = CookieJar::from_headers(headers);
    jar.get(&config.cookie_name).map(|c| c.value().to_string())
}

/// Whether an `Authorization` header states the Frappe API-key scheme with a
/// well-formed `key:secret` pair.
///
/// **Shape only.** [`require_auth`] runs before this gate and verifies the
/// credential against [`AuthState::api_keys`]; this predicate only decides
/// whether the request is a non-ambient credential for CSRF purposes. It
/// requires exactly two non-empty parts, so `token x` (no secret), `token x:`
/// (empty secret) and `token x:y:z` do not exempt the request — otherwise
/// `Authorization: token anything` would be a CSRF bypass on its own.
fn is_api_key_header(auth_str: &str) -> bool {
    let Some(credentials) = auth_str
        .strip_prefix("token ")
        .or_else(|| auth_str.strip_prefix("Token "))
    else {
        return false;
    };
    let mut parts = credentials.trim().split(':');
    let (Some(key), Some(secret), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !key.trim().is_empty() && !secret.trim().is_empty()
}

/// CSRF protection for cookie-authenticated mutating requests.
///
/// Safe methods (`GET`/`HEAD`/`OPTIONS`) have no side effects and are never
/// checked. Requests authenticated with a bearer JWT are exempt: a cross-site
/// attacker cannot make a victim's browser attach the header without first
/// stealing the token, so enforcing it there would add nothing but friction.
/// The same holds for the Frappe `Authorization: token <key>:<secret>` scheme
/// (AUTH-01): it is a header the browser does not attach on its own, and
/// [`require_auth`] verifies it before this gate runs.
/// Everything else must present the session's CSRF token in the
/// [`CSRF_HEADER`] header; the token is compared in constant time against the
/// session the cookie names (see [`crate::SessionManager::validate_csrf_token`]).
///
/// Mounted **inside** `require_auth` where it can rely on `AuthState`, so it is
/// the internal layer of the API router rather than a global gate.
pub async fn require_csrf(
    State(state): State<AuthState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> StdResult<Response, Response> {
    match *request.method() {
        Method::GET | Method::HEAD | Method::OPTIONS => return Ok(next.run(request).await),
        _ => {}
    }

    // Bearer JWT callers are exempt (see the doc comment above).
    if let Some(auth_header) = request.headers().get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Ok(token) = JwtManager::extract_token(auth_str)
        && state.jwt_manager.validate_access_token(&token).is_ok()
    {
        return Ok(next.run(request).await);
    }

    // API-key callers are exempt for the same reason: the credential is an
    // explicit header, not ambient browser state. `require_auth` has already
    // rejected a malformed or unknown key, so no extra verification here.
    if let Some(auth_header) = request.headers().get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && is_api_key_header(auth_str)
    {
        return Ok(next.run(request).await);
    }

    let sid = session_id_from_cookie(&state.config, request.headers());
    // A caller authenticated by a verified client certificate and carrying no
    // session cookie has no session for CSRF to protect: the certificate is not
    // ambient browser auth, so a cross-site form cannot ride it (H1 / Fase 2).
    // Enforced only when there is no cookie session, so a browser that *does*
    // hold a session still needs the token as before.
    if sid.is_none()
        && request
            .extensions()
            .get::<ferris_core::ClientIdentity>()
            .is_some()
    {
        return Ok(next.run(request).await);
    }
    let token = request
        .headers()
        .get(&CSRF_HEADER)
        .and_then(|v| v.to_str().ok());
    let (Some(sid), Some(token)) = (sid, token) else {
        return Err(csrf_forbidden());
    };
    match state.session_manager.validate_csrf_token(&sid, token).await {
        Ok(true) => Ok(next.run(request).await),
        _ => Err(csrf_forbidden()),
    }
}

/// Rate limiting for auth endpoints.
///
/// Fail-counting rather than the old coarse check-per-attempt: callers ask
/// [`RateLimiter::is_blocked`] before validating credentials, then report the
/// outcome with [`RateLimiter::record_failure`] / [`RateLimiter::record_success`]
/// so only **failed** attempts count towards a lockout and a successful login
/// clears the key. The window is the configured lockout duration.
#[derive(Clone)]
pub struct RateLimiter {
    attempts: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>,
    >,
    max_attempts: u32,
    window_seconds: u64,
}

impl RateLimiter {
    pub fn new(max_attempts: u32, window_seconds: u64) -> Self {
        Self {
            attempts: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            max_attempts,
            window_seconds,
        }
    }

    /// Whether `key` has exhausted its attempts inside the window.
    ///
    /// Prunes entries whose window elapsed as a side effect, so an expired
    /// lockout drops out on its own.
    #[must_use]
    pub fn is_blocked(&self, key: &str) -> bool {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        attempts.retain(|_, (_, timestamp)| {
            now.duration_since(*timestamp).as_secs() < self.window_seconds
        });
        attempts
            .get(key)
            .is_some_and(|(count, _)| *count >= self.max_attempts)
    }

    /// Record a failed attempt, extending the lockout window.
    pub fn record_failure(&self, key: &str) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        attempts.retain(|_, (_, timestamp)| {
            now.duration_since(*timestamp).as_secs() < self.window_seconds
        });
        let entry = attempts.entry(key.to_string()).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    /// Clear `key` after a successful authentication.
    pub fn record_success(&self, key: &str) {
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key);
    }

    /// The configured failure budget, so a caller can persist the same policy
    /// it enforces in memory (AUTH-04).
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// The configured lockout duration, in minutes.
    #[must_use]
    pub const fn lockout_minutes(&self) -> u64 {
        self.window_seconds / 60
    }
}

/// The persisted half of the login lockout (AUTH-04/AUTH-16).
///
/// [`RateLimiter`] is process-local, so a restart or a second replica forgets
/// every failure. The `User` document already declares
/// `failed_login_attempts` and `locked_until`, so the account half of the
/// lockout is stored on the row the login is about: the counter survives a
/// restart, every replica reads the same deadline, and the Desk can show why
/// the account is refusing attempts.
///
/// The temporary home is deliberate: the write belongs in a
/// `UserStore::lockout`/`set_lockout` pair next to `record_login`
/// (`ferris-auth/src/user_store/mod.rs`), which this change may not touch. The
/// surface here is public for exactly that migration — when the methods land on
/// the store, make them delegate to these functions (or move the bodies) and
/// keep the engine plumbing in one place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountLockout {
    pub failed_login_attempts: u32,
    pub locked_until: Option<chrono::DateTime<chrono::Utc>>,
}

impl AccountLockout {
    /// Whether the account is locked as of `now`.
    #[must_use]
    pub fn is_locked(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.locked_until.is_some_and(|until| until > now)
    }
}

fn lockout_engine(db: &ferris_db::Db) -> ferris_document::DocumentEngine {
    ferris_document::DocumentEngine::new(db.clone())
}

/// Read the persisted lockout of `user_id`; a missing row reads as "unlocked".
///
/// # Errors
///
/// Storage or document-decoding errors.
pub async fn account_lockout(
    db: &ferris_db::Db,
    user_id: &str,
) -> ferris_core::Result<AccountLockout> {
    let engine = lockout_engine(db);
    let meta = crate::doctypes::user_meta();
    let ctx = crate::store_support::system_context(&meta);
    match engine.get(&ctx, user_id).await {
        Ok(document) => Ok(AccountLockout {
            failed_login_attempts: u32::try_from(
                document
                    .get("failed_login_attempts")
                    .and_then(ferris_core::Value::as_i64)
                    .unwrap_or(0)
                    .clamp(0, i64::from(u32::MAX)),
            )
            .unwrap_or(u32::MAX),
            locked_until: document
                .get("locked_until")
                .and_then(ferris_core::Value::as_str)
                .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                .map(|until| until.with_timezone(&chrono::Utc)),
        }),
        Err(ferris_core::Error::Document(ferris_core::DocumentError::NotFound { .. })) => {
            Ok(AccountLockout::default())
        }
        Err(error) => Err(error),
    }
}

/// Record one failed login against the user row.
///
/// The counter and the deadline follow the same rule as
/// [`RateLimiter`]/[`ferris_core::User::record_failed_login`], so the in-memory
/// and persisted answers cannot disagree.
///
/// # Errors
///
/// Storage errors.
pub async fn record_account_failure(
    db: &ferris_db::Db,
    user_id: &str,
    max_attempts: u32,
    lockout_minutes: u64,
) -> Result<AccountLockout, ferris_core::Error> {
    let mut lockout = account_lockout(db, user_id).await?;
    lockout.failed_login_attempts = lockout.failed_login_attempts.saturating_add(1);
    if lockout.failed_login_attempts >= max_attempts {
        lockout.locked_until = Some(
            chrono::Utc::now()
                + chrono::Duration::minutes(i64::try_from(lockout_minutes).unwrap_or(i64::MAX)),
        );
    }
    write_account_lockout(db, user_id, lockout).await?;
    Ok(lockout)
}

/// Clear the persisted lockout after the **whole** login succeeded.
///
/// # Errors
///
/// Storage errors.
pub async fn clear_account_lockout(
    db: &ferris_db::Db,
    user_id: &str,
) -> Result<(), ferris_core::Error> {
    let current = account_lockout(db, user_id).await?;
    if current == AccountLockout::default() {
        return Ok(());
    }
    write_account_lockout(db, user_id, AccountLockout::default()).await
}

async fn write_account_lockout(
    db: &ferris_db::Db,
    user_id: &str,
    lockout: AccountLockout,
) -> Result<(), ferris_core::Error> {
    let engine = lockout_engine(db);
    let meta = crate::doctypes::user_meta();
    let ctx = crate::store_support::system_context(&meta);
    match engine.get(&ctx, user_id).await {
        Ok(_) => {}
        // A deleted account has nothing to stamp; the login already failed.
        Err(ferris_core::Error::Document(ferris_core::DocumentError::NotFound { .. })) => {
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    let mut fields = ferris_core::Map::new();
    fields.insert(
        "failed_login_attempts".to_string(),
        ferris_core::Value::from(i64::from(lockout.failed_login_attempts)),
    );
    fields.insert(
        "locked_until".to_string(),
        lockout
            .locked_until
            .map_or(ferris_core::Value::Null, |until| {
                ferris_core::Value::from(until.to_rfc3339())
            }),
    );
    engine
        .update(&ctx, user_id, ferris_core::Value::Object(fields))
        .await?;
    Ok(())
}

/// A listener told that a session (or every session of a user) was revoked.
///
/// The realtime engine registers one so a `logout` closes the live sockets
/// instead of leaving them receiving documents until the token expires
/// (RT-01). The port lives here because `ferris-auth` owns session lifecycle
/// and must not depend on the optional realtime crate.
pub type SessionRevokedHook = dyn Fn(&str, Option<&str>) + Send + Sync;

static SESSION_REVOKED_HOOK: std::sync::OnceLock<std::sync::Arc<SessionRevokedHook>> =
    std::sync::OnceLock::new();

/// Install the process-wide revocation listener. The first installation wins,
/// matching the realtime handler's own semantics.
pub fn install_session_revoked_hook(hook: std::sync::Arc<SessionRevokedHook>) -> bool {
    SESSION_REVOKED_HOOK.set(hook).is_ok()
}

/// Tell the installed listener about a revocation. `None` for `session_id`
/// means every session of `user_id` was closed.
pub(crate) fn notify_session_revoked(user_id: &str, session_id: Option<&str>) {
    if let Some(hook) = SESSION_REVOKED_HOOK.get() {
        hook(user_id, session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JwtConfig;
    use axum::http::HeaderValue;

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(3, 60);
        assert!(!limiter.is_blocked("test_key"));

        // Three failures cross the threshold and block the key.
        limiter.record_failure("test_key");
        limiter.record_failure("test_key");
        assert!(!limiter.is_blocked("test_key"));
        limiter.record_failure("test_key");
        assert!(limiter.is_blocked("test_key"));

        // A success clears the key again.
        limiter.record_success("test_key");
        assert!(!limiter.is_blocked("test_key"));

        // Unrelated keys are never blocked by someone else's failures.
        limiter.record_failure("other_key");
        assert!(!limiter.is_blocked("test_key"));
    }

    fn auth_state_with_api_keys(manager: crate::api_key::ApiKeyManager) -> AuthState {
        AuthState::new(
            JwtManager::new(JwtConfig::new("ferris-auth-test-secret-0123456789ab").unwrap())
                .unwrap(),
            SessionManager::default(),
            crate::session::SessionConfig::default(),
        )
        .with_api_keys(Arc::new(manager))
    }

    fn headers_with_authorization(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        headers
    }

    /// AUTH2: the Frappe `Authorization: token <key>:<secret>` scheme now
    /// actually authenticates. Before this, `ApiKeyManager` had no caller and no
    /// request path accepted a key.
    #[tokio::test]
    async fn api_key_header_authenticates_the_key_owner() {
        let manager = crate::api_key::ApiKeyManager::new();
        let (record, secret) = manager
            .create("keyowner@example.com", None, None)
            .await
            .unwrap();
        let state = auth_state_with_api_keys(manager);

        let headers = headers_with_authorization(&format!("token {}:{}", record.key, secret));
        let user = resolve_principal(&headers, &state)
            .await
            .expect("valid API key resolves a principal");
        assert_eq!(user.id, "keyowner@example.com");

        // A wrong secret authenticates nobody.
        let headers = headers_with_authorization(&format!("token {}:wrong", record.key));
        assert!(resolve_principal(&headers, &state).await.is_none());

        // Without a configured manager, the `token` scheme is not a credential.
        let no_keys = AuthState::new(
            JwtManager::new(JwtConfig::new("ferris-auth-test-secret-0123456789ab").unwrap())
                .unwrap(),
            SessionManager::default(),
            crate::session::SessionConfig::default(),
        );
        let headers = headers_with_authorization(&format!("token {}:{}", record.key, secret));
        assert!(resolve_principal(&headers, &no_keys).await.is_none());
    }

    /// AUTH10: `AuthenticatedUser` and `require_auth` now share
    /// `resolve_principal`, so the session-cookie path resolves the same
    /// principal through both. Here we prove the shared helper itself resolves a
    /// cookie session.
    #[tokio::test]
    async fn resolve_principal_reads_the_session_cookie() {
        let sessions = SessionManager::default();
        let session = sessions
            .create_session("cookie@example.com".to_string(), None, None)
            .await
            .unwrap();
        let config = crate::session::SessionConfig::default();
        let cookie_name = config.cookie_name.clone();
        let state = AuthState::new(
            JwtManager::new(JwtConfig::new("ferris-auth-test-secret-0123456789ab").unwrap())
                .unwrap(),
            sessions,
            config,
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{cookie_name}={}", session.id)).unwrap(),
        );
        let user = resolve_principal(&headers, &state)
            .await
            .expect("cookie session resolves a principal");
        assert_eq!(user.id, "cookie@example.com");
    }

    /// AUTH-05: `logout` deletes the session, and the JWT minted for it must
    /// stop authenticating immediately. Signature and expiry cannot express
    /// that, so `resolve_principal` asks the shared session store.
    #[tokio::test]
    async fn a_logged_out_session_stops_authenticating_its_bearer_token() {
        let sessions = SessionManager::default();
        let session = sessions
            .create_session("jwt@example.com".to_string(), None, None)
            .await
            .unwrap();
        let jwt = JwtManager::new(JwtConfig::new("ferris-auth-test-secret-0123456789ab").unwrap())
            .unwrap();
        let token = jwt
            .generate_tokens("jwt@example.com", "jwt@example.com", &[], None, &session.id)
            .unwrap()
            .access_token;
        let config = crate::session::SessionConfig::default();
        let state = AuthState::new(jwt, sessions, config);

        let headers = headers_with_authorization(&format!("Bearer {token}"));
        assert!(
            resolve_principal(&headers, &state).await.is_some(),
            "a live session backs the token"
        );

        state
            .session_manager
            .delete_session(&session.id)
            .await
            .expect("logout deletes the session");
        assert!(
            resolve_principal(&headers, &state).await.is_none(),
            "the token of a revoked session must not authenticate"
        );
    }

    /// AUTH-04/AUTH-16: the account lockout is written to the user row, so a
    /// restart or another replica reads the same deadline instead of starting
    /// from zero.
    #[tokio::test]
    async fn the_account_lockout_is_persisted_on_the_user() {
        use chrono::Utc;

        let db = ferris_db::memory_db().await.unwrap();
        let store = crate::UserStore::new(db.clone());
        store.init().await.unwrap();
        let user = store
            .create(
                "lockout@example.com",
                "Lockout",
                None,
                crate::UserType::SystemUser,
                "Secret-Password1",
                &["User".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(
            account_lockout(&db, &user.id).await.unwrap(),
            AccountLockout::default(),
            "a fresh account is not locked"
        );

        let first = record_account_failure(&db, &user.id, 2, 15).await.unwrap();
        assert_eq!(first.failed_login_attempts, 1);
        assert!(!first.is_locked(Utc::now()));

        let second = record_account_failure(&db, &user.id, 2, 15).await.unwrap();
        assert!(second.is_locked(Utc::now()), "the budget is exhausted");

        // Reading again is what a restarted process does.
        assert!(
            account_lockout(&db, &user.id)
                .await
                .unwrap()
                .is_locked(Utc::now())
        );

        clear_account_lockout(&db, &user.id).await.unwrap();
        assert_eq!(
            account_lockout(&db, &user.id).await.unwrap(),
            AccountLockout::default(),
            "a completed login clears the counter"
        );
    }

    /// AUTH-01: a well-formed `token <key>:<secret>` header is a non-ambient
    /// credential and is exempt from the CSRF gate. A truncated or arbitrary
    /// header must not be: otherwise `Authorization: token anything` would be a
    /// CSRF bypass, and a cookie-only mutation would go through without the
    /// session's token.
    ///
    /// The cookie-without-token refusal itself is covered end to end by
    /// `ferris_api::router::tests::csrf::cookie_mutations_carry_the_session_csrf_token`.
    #[test]
    fn api_key_authorization_exempts_csrf_only_when_well_formed() {
        assert!(is_api_key_header("token k:s"));
        assert!(is_api_key_header("Token k:s"));
        assert!(is_api_key_header("token  k : s "));

        assert!(!is_api_key_header("token k"));
        assert!(!is_api_key_header("token :s"));
        assert!(!is_api_key_header("token k:"));
        assert!(!is_api_key_header("token k:s:extra"));
        assert!(!is_api_key_header("token "));
        assert!(!is_api_key_header("Token"));
        assert!(!is_api_key_header("Bearer k:s"));
        assert!(!is_api_key_header(""));
    }
}
