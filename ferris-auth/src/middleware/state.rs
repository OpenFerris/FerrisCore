//! Authentication state, extractors and the credential-resolution decision.
//!
//! [`AuthState`] is the value every gate shares; [`resolve_principal`] is the
//! one place that turns a request into a [`User`], and [`AuthenticatedUser`] /
//! [`OptionalUser`] / [`require_auth`] are only thin wrappers over it.

use crate::User;
use async_trait::async_trait;
use axum::{
    extract::{FromRef, FromRequestParts, Request, State},
    http::{Extensions, HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::result::Result as StdResult;
use std::sync::Arc;

use crate::login::{DbSessionStore, SessionConfig, principal_from_extensions};

/// Validates a user against the source of truth (typically the database).
///
/// Returning `None` (or a disabled user) rejects the request, so a token
/// issued before an account was disabled stops working immediately.
#[async_trait]
pub trait UserValidator: Send + Sync {
    async fn validate(&self, user_id: &str) -> Option<User>;
}

/// Authentication state shared by every gate.
///
/// The browser credential is the signed `tower-sessions` cookie; the only other
/// credential is the Frappe `Authorization: token <key>:<secret>` API key. There
/// is no bearer token.
#[derive(Clone)]
pub struct AuthState {
    /// Cookie name and lifetime, matching the mounted session layer.
    pub config: SessionConfig,
    /// The one session store. Kept here so layers that hold only `AuthState`
    /// (the realtime handshake, the website-form CSRF check) can answer
    /// "is this session still live?" without a second store.
    pub session_store: Option<DbSessionStore>,
    /// Optional source-of-truth check for non-session credentials (the API key
    /// path) and the realtime role revalidation.
    pub user_validator: Option<Arc<dyn UserValidator>>,
    /// Optional API-key verifier. When set, the middleware accepts the Frappe
    /// `Authorization: token <key>:<secret>` scheme (AUTH2); without it that
    /// header is simply not a credential.
    pub api_keys: Option<Arc<crate::api_key::ApiKeyManager>>,
}

impl AuthState {
    /// Build the state from the cookie/lifetime configuration.
    #[must_use]
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            session_store: None,
            user_validator: None,
            api_keys: None,
        }
    }

    /// Attach the `tower-sessions` store used by this process.
    #[must_use]
    pub fn with_session_store(mut self, store: DbSessionStore) -> Self {
        self.session_store = Some(store);
        self
    }

    #[must_use]
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

impl Default for AuthState {
    fn default() -> Self {
        Self::new(SessionConfig::default())
    }
}

/// Extract authenticated user
#[derive(Clone)]
pub struct AuthenticatedUser(pub User);

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CACHE_CONTROL, "no-store")],
        "Authentication required",
    )
        .into_response()
}

/// Resolve the user from the source of truth when a validator is configured,
/// otherwise fall back to the API key's owner.
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
/// This is **the** credential-resolution decision. It tries the Frappe
/// `token <key>:<secret>` API key first (an explicit header the browser never
/// attaches by itself), then the `tower-sessions` cookie through
/// [`principal_from_extensions`]. `AuthenticatedUser`, [`OptionalUser`] and
/// [`require_auth`] all call it, so an added credential source or a changed
/// expiry rule cannot silently apply to only one of them (AUTH10).
///
/// `None` means "not authenticated": no usable credential, an expired/absolute
/// expired session, or a user the validator rejected.
pub async fn resolve_principal(
    headers: &HeaderMap,
    extensions: &Extensions,
    auth: &AuthState,
) -> Option<User> {
    resolve_principal_with_credential(headers, extensions, auth)
        .await
        .map(|(user, _)| user)
}

/// The marker left in the request extensions when an API key authenticated it.
///
/// The CSRF gate must not verify the same key a second time: `require_auth`
/// already did, on every request, and an API key now costs one fast digest
/// check plus a lookup. Presence of this marker is the proof.
#[derive(Clone, Debug)]
pub(super) struct ApiKeyPrincipal {
    #[allow(dead_code)] // kept for diagnostics; the gate only needs presence.
    pub(super) key_id: String,
}

/// [`resolve_principal`], also reporting which credential authenticated.
async fn resolve_principal_with_credential(
    headers: &HeaderMap,
    extensions: &Extensions,
    auth: &AuthState,
) -> Option<(User, Option<ApiKeyPrincipal>)> {
    if let Some(auth_header) = headers.get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(manager) = &auth.api_keys
        && let Some(key) = extract_api_key(auth_str, manager).await
    {
        // The key names its owner; resolve that user through the same source
        // of truth, so a disabled account is still refused.
        let user = resolve_user(auth, &key.user, String::new(), Vec::new()).await?;
        return Some((user, Some(ApiKeyPrincipal { key_id: key.id })));
    }

    // The cookie session: `axum-login` rehydrated the user through
    // `Backend::get_user` on this request, and that path already rejects a
    // disabled account and reads the current roles. A second `validate` here
    // was the same database row read twice per request.
    principal_from_extensions(extensions)
        .await
        .map(|user| (user, None))
}

/// Verify an `Authorization: token <key>:<secret>` header against the API-key
/// manager, returning the key on success.
///
/// Returns `None` for a header that is not the `token` scheme, for credentials
/// that do not verify, and for a storage failure — in every case the caller
/// falls through to the next credential source (a session cookie may still
/// authenticate the request). A storage failure is logged so it is not silently
/// indistinguishable from "wrong secret".
pub(super) async fn extract_api_key(
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
        // `require_auth` (or an earlier extractor) already resolved this
        // request. Reusing the extension turns the common path into zero
        // credential lookups instead of a session read per extractor.
        if let Some(cached) = parts.extensions.get::<AuthenticatedUser>() {
            return Ok(cached.clone());
        }
        let auth = AuthState::from_ref(state);
        let Some((user, key)) =
            resolve_principal_with_credential(&parts.headers, &parts.extensions, &auth).await
        else {
            return Err(unauthorized());
        };
        if let Some(key) = key {
            parts.extensions.insert(key);
        }
        let user = AuthenticatedUser(user);
        parts.extensions.insert(user.clone());
        Ok(user)
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
        // Same cache as `AuthenticatedUser`: an outer gate has usually resolved
        // the request already.
        if let Some(cached) = parts.extensions.get::<AuthenticatedUser>() {
            return Ok(OptionalUser(Some(cached.0.clone())));
        }
        if let Ok(auth_user) = AuthenticatedUser::from_request_parts(parts, state).await {
            return Ok(OptionalUser(Some(auth_user.0)));
        }
        Ok(OptionalUser(None))
    }
}

/// Require authentication middleware
pub async fn require_auth(
    State(state): State<AuthState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> StdResult<Response, Response> {
    // A nested gate (the API router plus a module router) must not resolve the
    // same credential twice.
    if request.extensions().get::<AuthenticatedUser>().is_some() {
        return Ok(next.run(request).await);
    }
    if let Some((user, key)) =
        resolve_principal_with_credential(request.headers(), request.extensions(), &state).await
    {
        if let Some(key) = key {
            request.extensions_mut().insert(key);
        }
        request.extensions_mut().insert(AuthenticatedUser(user));
        return Ok(next.run(request).await);
    }
    Err(unauthorized())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn auth_state_with_api_keys(manager: crate::api_key::ApiKeyManager) -> AuthState {
        AuthState::new(SessionConfig::default()).with_api_keys(Arc::new(manager))
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
        let extensions = Extensions::new();

        let headers = headers_with_authorization(&format!("token {}:{}", record.key, secret));
        let user = resolve_principal(&headers, &extensions, &state)
            .await
            .expect("valid API key resolves a principal");
        assert_eq!(user.id, "keyowner@example.com");

        // A wrong secret authenticates nobody.
        let headers = headers_with_authorization(&format!("token {}:wrong", record.key));
        assert!(
            resolve_principal(&headers, &extensions, &state)
                .await
                .is_none()
        );

        // Without a configured manager, the `token` scheme is not a credential.
        let no_keys = AuthState::new(SessionConfig::default());
        let headers = headers_with_authorization(&format!("token {}:{}", record.key, secret));
        assert!(
            resolve_principal(&headers, &extensions, &no_keys)
                .await
                .is_none()
        );
    }

    /// Without the session/auth layers there is no request extension to read,
    /// so a cookie alone cannot authenticate. This is the fail-closed half of
    /// the new resolver: the old custom store is gone, and nothing falls back to
    /// it.
    #[tokio::test]
    async fn a_cookie_without_the_axum_login_layer_authenticates_nobody() {
        let state = AuthState::new(SessionConfig::default());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("ferris_session=some-id"),
        );
        assert!(
            resolve_principal(&headers, &Extensions::new(), &state)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_authenticated_user_in_the_extensions_is_reused() {
        let mut parts = axum::http::Request::builder()
            .body(())
            .unwrap()
            .into_parts()
            .0;
        let mut user = User::system_user("cached@local".to_string(), "Cached".to_string());
        user.id = "cached@local".to_string();
        parts.extensions.insert(AuthenticatedUser(user.clone()));

        let extracted = AuthenticatedUser::from_request_parts(&mut parts, &AuthState::default())
            .await
            .expect("the cached principal is returned");
        assert_eq!(extracted.0.id, "cached@local");

        let optional = OptionalUser::from_request_parts(&mut parts, &AuthState::default())
            .await
            .expect("OptionalUser is infallible");
        assert_eq!(
            optional.0.map(|user| user.id),
            Some("cached@local".to_string())
        );
    }
}
