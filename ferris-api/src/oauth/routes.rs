//! `OAuth2` HTTP endpoints — a **login**, not a token dispenser.
//!
//! Wraps [`ferris_auth::OAuthManager`] behind axum routes:
//!
//! - `GET /api/oauth/providers` — list configured providers;
//! - `GET /api/oauth/{provider}/login?next=/desk` — start the flow: mint the
//!   one-time `state`, bind it to **this browser** in an `HttpOnly`
//!   `SameSite=Lax` cookie scoped to `/api/oauth`, and 302 to the provider;
//! - `GET /api/oauth/{provider}/callback?code=&state=` — verify the cookie,
//!   exchange the code, resolve the local account, open a **session** (cookie +
//!   `X-CSRF-Token`) and 303 to `next`.
//!
//! ## Why the callback no longer answers with tokens
//!
//! It used to return the provider's `access_token`/`refresh_token` as JSON in
//! the body of a `GET`, with no session, no user lookup and nothing tying the
//! callback to the browser that started the flow (the state travelled in an
//! `X-OAuth-State` response header). That is not a login: the Desk could not use
//! it, and any `code`/`state` pair could be replayed from another browser. The
//! tokens are now consumed server-side — the access token only lives long enough
//! to read the identity from the provider's userinfo endpoint — and never reach
//! the browser.
//!
//! Non-browser clients (API keys, bearer tokens, the Desk's own session) keep
//! using `POST /api/auth/login` and the REST API. Provider tokens for a *third
//! party* app would need that app's own client registration (its redirect URI
//! and PKCE verifier), which is a different feature; riding on the SSO state
//! store would hand out tokens with the SSO flow's trust, so it is deliberately
//! not offered here.
//!
//! ## Account resolution
//!
//! The provider proves an **email address**, nothing more. The local account is
//! looked up by that email (the same `email`/`username` lookup the password
//! login uses) and must already exist and be enabled: an unknown identity is
//! refused instead of being auto-provisioned, because a self-service OAuth
//! account would bypass whatever policy governs who may have an account. A local
//! account with MFA enabled is refused too — the second factor is not something
//! an external provider can satisfy — with a message pointing at the password
//! login.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use ferris_auth::oauth::{NEXT_COOKIE, STATE_COOKIE, clear_cookie, flow_cookie};
use ferris_auth::{OAuthConfig, OAuthManager, SessionConfig};
use ferris_core::{Error, Result, Value};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::warn;

use crate::session::{
    SessionHandlers, client_ip, cookie_value, set_cookie, set_csrf_token, start_session,
};

/// Where a login lands when the request asked for nothing (or asked for
/// something that is not a local path).
pub const DEFAULT_NEXT: &str = "/desk";

/// Shared OAuth state. A tokio mutex is used because `callback` awaits while
/// holding the manager (the state store and the token exchange must not
/// interleave for the same flow).
#[derive(Clone)]
pub struct OAuthState {
    pub manager: Arc<Mutex<OAuthManager>>,
    /// The login gate: the same instance the auth middleware and
    /// `POST /api/auth/login` use, so an OAuth login is immediately visible to
    /// every guarded route.
    pub session: SessionHandlers,
}

impl OAuthState {
    #[must_use]
    pub fn new(manager: OAuthManager, session: SessionHandlers) -> Self {
        Self {
            manager: Arc::new(Mutex::new(manager)),
            session,
        }
    }
}

/// Build the OAuth router from a configuration, with the state resolved so the
/// result is directly servable (`Router<()>`).
///
/// # Errors
///
/// [`Error::Config`]/[`Error::Auth`] when a provider is unusable (no redirect
/// URI, malformed auth URL).
pub fn oauth_router(config: OAuthConfig, session: SessionHandlers) -> Result<Router> {
    let manager = OAuthManager::new(config)?;
    Ok(oauth_router_with_manager(manager, session))
}

/// Build the OAuth router from an existing manager, resolved to `Router<()>`.
pub fn oauth_router_with_manager(manager: OAuthManager, session: SessionHandlers) -> Router {
    let state = OAuthState::new(manager, session);
    oauth_router_with_state().with_state(state)
}

/// The OAuth routes as a `Router<S>`, **without** resolving the state.
///
/// `ferris-server` embeds the OAuth surface next to the REST API, so it needs a
/// router that resolves `OAuthState` out of its own `AppState` (the axum 0.8
/// embedding pattern used for plugins):
///
/// ```ignore
/// // `AppState` holds the `OAuthState` and implements `FromRef<AppState> for
/// // OAuthState`, so the merge keeps one state type.
/// router = router.merge(ferris_api::oauth_router::<AppState>());
/// ```
pub fn oauth_router_with_state<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    OAuthState: axum::extract::FromRef<S>,
{
    Router::new()
        .route("/api/oauth/providers", get(list_providers))
        .route("/api/oauth/{provider}/login", get(login))
        .route("/api/oauth/{provider}/callback", get(callback))
}

async fn list_providers(State(state): State<OAuthState>) -> Response {
    let manager = state.manager.lock().await;
    Json(serde_json::json!({ "providers": manager.list_providers() })).into_response()
}

#[derive(Debug, Deserialize, Default)]
struct LoginQuery {
    /// Local path to land on after the session exists. Validated here and again
    /// on the callback, because a cookie is client-controlled input.
    next: Option<String>,
}

async fn login(
    State(state): State<OAuthState>,
    Path(provider): Path<String>,
    Query(query): Query<LoginQuery>,
) -> Response {
    let next = safe_next(query.next.as_deref());
    let mut manager = state.manager.lock().await;
    let (url, oauth_state) = match manager.authorize(&provider) {
        Ok(minted) => minted,
        Err(e) => return error_response(e),
    };
    let ttl = manager.state_ttl_seconds();
    drop(manager);

    let config = &state.session.config;
    let mut response = Redirect::temporary(url.as_str()).into_response();
    set_cookie(
        &mut response,
        &flow_cookie(STATE_COOKIE, &oauth_state, ttl, config.secure_cookies).to_string(),
    );
    set_cookie(
        &mut response,
        &flow_cookie(NEXT_COOKIE, &next, ttl, config.secure_cookies).to_string(),
    );
    no_store(&mut response);
    response
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: String,
    state: String,
}

async fn callback(
    State(state): State<OAuthState>,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Response {
    let config = state.session.config.clone();
    // The state the *browser* sent. A missing or mismatched cookie means this
    // callback did not start here: refuse before exchanging anything, so a
    // stolen/replayed `code`+`state` pair cannot mint a session in an attacker's
    // browser.
    let Some(bound) = cookie_value(&headers, STATE_COOKIE) else {
        return refuse("missing oauth state cookie", &config);
    };
    if !ferris_auth::crypto::constant_time_eq(bound.as_bytes(), query.state.as_bytes()) {
        return refuse("oauth state does not match this browser", &config);
    }
    let next = safe_next(cookie_value(&headers, NEXT_COOKIE).as_deref());

    let user_info = {
        let mut manager = state.manager.lock().await;
        // Single use: the manager removes the state before validating it, so a
        // replayed callback fails even with the cookie still in the jar.
        let tokens = match manager
            .callback(&provider, query.state.clone(), query.code)
            .await
        {
            Ok(tokens) => tokens,
            Err(e) => return error_response(e),
        };
        let Some(client) = manager.get_provider(&provider) else {
            return error_response(Error::Validation(ferris_core::ValidationError::Invalid(
                format!("Provider not found: {provider}"),
            )));
        };
        let user_info = match client.get_user_info(&tokens.access_token).await {
            Ok(info) => info,
            Err(e) => return error_response(e),
        };
        // The tokens are a credential: the access token was used once, above, and
        // neither it nor the refresh token is stored on the session or returned
        // to the browser — they are dropped with this block.
        user_info
    };

    let Some(email) = user_info.get("email").and_then(Value::as_str) else {
        warn!(
            provider = %provider,
            "oauth userinfo carried no `email`; cannot resolve a local account"
        );
        return refuse("the provider did not return an email address", &config);
    };

    let user = match state.session.user_store.find_by_login(email).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            warn!(
                provider = %provider,
                "oauth login for an email with no local account was refused"
            );
            return refuse("no Ferris account is linked to that identity", &config);
        }
        Err(e) => return error_response(e),
    };
    if !user.enabled {
        return refuse("this account is disabled", &config);
    }
    if user.mfa_enabled {
        // An external provider proves the first factor only; letting this
        // through would be an MFA bypass for every account that enabled it.
        return refuse(
            "this account uses two-factor authentication; sign in with your password",
            &config,
        );
    }

    let ip = client_ip(&headers);
    let session = match start_session(&state.session, &user, Some(ip)).await {
        Ok(session) => session,
        Err(e) => return error_response(e),
    };

    // 303, not 302: the callback was a GET and the browser must follow it
    // with a GET to the destination (a 302 leaves the method up to the client).
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, next.as_str())]).into_response();
    set_cookie(&mut response, &config.build_cookie(&session.id).to_string());
    // The redirect cannot carry the header across the navigation, but a
    // fetch-following client reads it here; the Desk's own bootstrap
    // (`GET /api/auth/session`) hands it the live token either way.
    set_csrf_token(&mut response, &session.csrf_token);
    set_cookie(
        &mut response,
        &clear_cookie(STATE_COOKIE, config.secure_cookies).to_string(),
    );
    set_cookie(
        &mut response,
        &clear_cookie(NEXT_COOKIE, config.secure_cookies).to_string(),
    );
    no_store(&mut response);
    response
}

/// A `next` target that is safe to redirect to: a local absolute path.
///
/// Anything else (`//evil.example`, a scheme, a path with control characters)
/// falls back to [`DEFAULT_NEXT`]. This is the only open-redirect guard on the
/// flow and it runs twice: on the way in (`login`) and on the way out
/// (`callback`), because the cookie is client-controlled input.
pub(super) fn safe_next(raw: Option<&str>) -> String {
    let Some(candidate) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_NEXT.to_string();
    };
    let local = candidate.starts_with('/')
        && !candidate.starts_with("//")
        && !candidate
            .chars()
            .any(|c| c.is_control() || c == '\\' || c == '\r' || c == '\n');
    if local {
        candidate.to_string()
    } else {
        DEFAULT_NEXT.to_string()
    }
}

/// Answer a refused flow: no session, no tokens, and the flow cookies deleted so
/// the browser does not retry with a dead state.
fn refuse(message: &str, config: &SessionConfig) -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        Json(crate::ApiError::new("oauth_refused", message)),
    )
        .into_response();
    set_cookie(
        &mut response,
        &clear_cookie(STATE_COOKIE, config.secure_cookies).to_string(),
    );
    set_cookie(
        &mut response,
        &clear_cookie(NEXT_COOKIE, config.secure_cookies).to_string(),
    );
    no_store(&mut response);
    response
}

pub(super) fn error_response(error: Error) -> Response {
    let status = match error {
        Error::Validation(_)
        | Error::Authz(ferris_core::AuthzError::InvalidPolicy(_))
        | Error::Config(_) => StatusCode::BAD_REQUEST,
        Error::Auth(_) => StatusCode::UNAUTHORIZED,
        Error::Document(ferris_core::DocumentError::NotFound { .. })
        | Error::Schema(ferris_core::SchemaError::DocTypeNotFound(_))
        | Error::File(ferris_core::FileError::NotFound(_)) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    // Same rule as the REST surface: a 5xx is our fault, so its text goes to the
    // log and the caller only learns that it failed. A 4xx (bad code, expired
    // state, unknown provider) keeps its message — the client has to act on it.
    let message = if status.is_server_error() {
        warn!(status = status.as_u16(), error = %error, "oauth flow failed");
        crate::session::INTERNAL_ERROR.to_string()
    } else {
        error.to_string()
    };
    let mut response = (status, Json(serde_json::json!({ "error": message }))).into_response();
    no_store(&mut response);
    response
}

/// A login response is per-request state: never let a proxy or the browser keep
/// it.
fn no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}
