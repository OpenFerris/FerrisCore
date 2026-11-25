//! OAuth2 + userinfo HTTP endpoints — a **login**, not a token dispenser.
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
//! Non-browser clients (API keys, the Desk's own session) keep
//! using `POST /api/auth/login` and the REST API. Provider tokens for a *third
//! party* app would need that app's own client registration (its redirect URI
//! and PKCE verifier), which is a different feature; riding on the SSO state
//! store would hand out tokens with the SSO flow's trust, so it is deliberately
//! not offered here.
//!
//! ## What the provider proves
//!
//! Identity comes from the provider's **userinfo** endpoint only. An
//! `id_token` is not validated (no JWKS signature, no `iss`/`aud`/`nonce`), so
//! this is OAuth2 + userinfo, not OpenID Connect; a provider configured with
//! `oidc = true` is refused when the manager is built.
//!
//! ## Account resolution
//!
//! The provider proves an **email address**, and only a verified one: unless the
//! provider explicitly opted out (`require_verified_email = false`), userinfo
//! must carry `email_verified: true`. A missing claim is refused, because "the
//! provider did not say" is not "verified".
//!
//! The local account is looked up by that email (the same `email`/`username`
//! lookup the password login uses) and must already exist and be enabled: an
//! unknown identity is refused instead of being auto-provisioned, because a
//! self-service OAuth account would bypass whatever policy governs who may have
//! an account.
//!
//! A provider that returns a stable `sub` also binds it to the account on the
//! first login; a later login presenting a different subject is refused
//! (`this identity is linked to another account`) and an existing subject is
//! never overwritten. A local account with MFA enabled is refused too — the
//! second factor is not something an external provider can satisfy — with a
//! message pointing at the password login.
//!
//! ## Email verification policy
//!
//! When `[auth] require_email_verification` is on, the provider's
//! `email_verified: true` is the proof: it satisfies the local check, and the
//! callback marks the local `email_verified` column as verified before opening
//! the session (best-effort, with a warning if the write fails). A local
//! account that is still unverified and a provider that did not verify the
//! address is refused, same as a password login would be.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use axum_login::AuthSession;
use ferris_auth::login::{Backend, SessionUser};
use ferris_auth::oauth::{NEXT_COOKIE, STATE_COOKIE, clear_cookie, flow_cookie};
use ferris_auth::{OAuthConfig, OAuthManager, SessionConfig};
use ferris_core::{Error, Result, Value};
use serde::Deserialize;
use tokio::sync::Mutex;
use tower_sessions::Session;
use tracing::warn;

use crate::session::{
    SessionHandlers, client_ip, cookie_value, set_cookie, set_csrf_token, start_session,
};

/// Where a login lands when the request asked for nothing (or asked for
/// something that is not a local path).
pub const DEFAULT_NEXT: &str = "/desk";

/// Shared OAuth state.
///
/// The tokio mutex guards the manager's pending-state store only. A callback
/// takes the state (and a **cloned** client) under the lock and immediately
/// drops it; the token exchange and userinfo requests run on that clone with no
/// lock held, so a slow provider cannot block `/api/oauth/providers`, another
/// login, or another callback.
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

/// Build a self-contained OAuth router from an existing manager.
///
/// The router mounts the same `tower-sessions` + `axum-login` layer the server
/// uses, because the callback extracts `AuthSession` and `Session`. A caller
/// that embeds [`oauth_router_with_state`] in its own state already owns that
/// layer and must not mount it twice.
pub fn oauth_router_with_manager(manager: OAuthManager, session: SessionHandlers) -> Router {
    let store = session.session_store.clone();
    let config = session.config.clone();
    let backend = session.backend.clone();
    let state = OAuthState::new(manager, session);
    let router = oauth_router_with_state().with_state(state);
    match store {
        Some(store) => router.layer(ferris_auth::login::auth_layer(&config, store, backend)),
        None => router,
    }
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
    // The lock covers the state write only; the state store persists the entry
    // (memory or DB) before `authorize` answers the URL.
    let minted = {
        let mut manager = state.manager.lock().await;
        let minted = manager.authorize(&provider).await;
        let ttl = manager.state_ttl_seconds();
        minted.map(|(url, oauth_state)| (url, oauth_state, ttl))
    };
    let (url, oauth_state, ttl) = match minted {
        Ok(minted) => minted,
        Err(e) => return error_response(e),
    };

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
    auth_session: AuthSession<Backend>,
    session: Session,
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

    // Take the state (single use: the store removes it before validating, so a
    // replayed callback fails even with the cookie still in the jar) and a
    // cloned client. The guard is dropped here, before any request leaves the
    // process: a slow token endpoint must not hold the manager lock.
    let pending = {
        let mut manager = state.manager.lock().await;
        manager.take_pending(&provider, &query.state).await
    };
    let (client, pkce_verifier) = match pending {
        Ok(pending) => pending,
        Err(e) => return error_response(e),
    };
    let require_verified_email = client.require_verified_email();
    let tokens = match client.exchange_code(query.code, pkce_verifier).await {
        Ok(tokens) => tokens,
        Err(e) => return error_response(e),
    };
    let user_info = match client.get_user_info(&tokens.access_token).await {
        Ok(info) => info,
        Err(e) => return error_response(e),
    };
    // The tokens are a credential: the access token was used once, above, and
    // neither it nor the refresh token is stored on the session or returned to
    // the browser — they are dropped with this scope.

    // An unverified address must not resolve an account: whoever controls the
    // mailbox the provider has *not* confirmed could otherwise claim it. A
    // missing `email_verified` is refused too, unless the provider opted out.
    if require_verified_email
        && user_info.get("email_verified").and_then(Value::as_bool) != Some(true)
    {
        warn!(
            provider = %provider,
            "oauth userinfo did not report a verified email; refusing the login"
        );
        return refuse("the provider did not verify this email address", &config);
    }

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

    // A provider-verified address is the proof the opt-in local policy asks
    // for: it satisfies the check even while the local flag is still false, and
    // the callback records it before opening the session. A provider that did
    // not verify the address cannot satisfy the policy; that combination only
    // reaches here when the provider opted out of `require_verified_email`.
    let provider_verified = user_info.get("email_verified").and_then(Value::as_bool) == Some(true);
    if state.session.require_email_verification && !user.email_verified {
        if !provider_verified {
            warn!(
                provider = %provider,
                "oauth login refused: the email is not verified locally or by the provider"
            );
            return refuse("verify your email address before signing in", &config);
        }
        // Best effort: the provider already proved the address, so a failed
        // write is logged and the login continues.
        if let Err(error) = state
            .session
            .user_store
            .set_email_verified(&user.id, true)
            .await
        {
            warn!(
                user.id = %user.id,
                %error,
                "could not mark the oauth email as verified"
            );
        }
    }

    // Bind the provider's stable subject on the first login and compare it
    // afterwards: the email alone can be reassigned upstream, the subject
    // cannot. `bind_oauth_subject` never overwrites a non-empty value, so a
    // second provider identity for the same account is refused instead of
    // silently taking it over.
    if let Some(subject) = user_info
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
    {
        match state
            .session
            .user_store
            .bind_oauth_subject(&user.id, subject)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                warn!(
                    provider = %provider,
                    "oauth subject does not match the account's existing binding"
                );
                return refuse("this identity is linked to another account", &config);
            }
            Err(e) => return error_response(e),
        }
    }

    let ip = client_ip(&headers);
    let record = match state.session.user_store.login_record_by_id(&user.id).await {
        Ok(Some(record)) => record,
        Ok(None) => return refuse("this account is disabled", &config),
        Err(e) => return error_response(e),
    };
    let session_user = SessionUser::new(record.user.to_user(), record.session_auth_hash());
    if let Err(error) = start_session(
        &auth_session,
        &session,
        &state.session,
        &session_user,
        Some(ip),
    )
    .await
    {
        warn!(%error, "could not open the OAuth session");
        return crate::session::internal_failure();
    }

    // 303, not 302: the callback was a GET and the browser must follow it
    // with a GET to the destination (a 302 leaves the method up to the client).
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, next.as_str())]).into_response();
    // The session cookie is written by `tower-sessions`. The redirect cannot
    // carry the header across the navigation, but a fetch-following client
    // reads it here; the Desk's own bootstrap (`GET /api/auth/session`) hands
    // it the live token either way.
    if let Some(csrf) = ferris_auth::login::csrf_token(&session).await {
        set_csrf_token(&mut response, &csrf);
    }
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
