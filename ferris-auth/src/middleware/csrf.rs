//! CSRF protection for cookie-authenticated mutating requests.
//!
//! The gate treats the session cookie as ambient browser authentication and
//! requires the session's token in [`CSRF_HEADER`]; a non-ambient credential
//! (a verified API key or a verified client certificate) is exempt. The cookie
//! reader lives here because it is the same decision the gate makes.

use std::result::Result as StdResult;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tower_sessions::Session;

use crate::crypto::constant_time_eq;
use crate::login::SessionConfig;

use super::state::{ApiKeyPrincipal, AuthState, extract_api_key};

/// The header carrying the CSRF token on cookie-authenticated mutating requests.
pub const CSRF_HEADER: HeaderName = HeaderName::from_static("x-csrf-token");

/// Read one cookie value out of a raw `Cookie` header.
///
/// One parser for every layer that authenticates from a cookie (the REST API,
/// the OAuth flow cookies, the realtime handshake): a second reader is a second
/// place for `name=` to be treated as a credential or for a name suffix to
/// match. An empty value reads as absent.
#[must_use]
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
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
pub fn session_cookie(config: &SessionConfig, headers: &HeaderMap) -> Option<String> {
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

/// Whether an `Authorization` header states the Frappe API-key scheme with a
/// well-formed `key:secret` pair.
///
/// **Shape only.** [`crate::middleware::require_auth`] runs before this gate and
/// verifies the credential against [`AuthState::api_keys`]; this predicate only
/// decides whether the request is a non-ambient credential for CSRF purposes.
/// It requires exactly two non-empty parts, so `token x` (no secret), `token x:`
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
/// checked. Requests authenticated with the Frappe
/// `Authorization: token <key>:<secret>` scheme (AUTH-01) are exempt: it is a
/// header the browser does not attach on its own, and
/// [`crate::middleware::require_auth`] verifies it before this gate runs. A
/// caller that proved itself with a verified client certificate and holds no
/// session cookie is exempt for the same reason (H1).
///
/// Everything else must present the session's CSRF token in the
/// [`CSRF_HEADER`] header. The token is read from the `tower-sessions` record
/// and compared in constant time. The session layer must be mounted outside
/// this gate; a missing `Session` extension fails closed.
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

    // `require_auth` already verified this request's API key and left the
    // marker. Re-verifying it here would repeat the lookup on every mutating
    // API call, which is the hot path this gate exists to protect.
    if request.extensions().get::<ApiKeyPrincipal>().is_some() {
        return Ok(next.run(request).await);
    }

    // This gate can also be mounted on a public auth route (logout) where
    // `require_auth` does not run, so a well-formed header still has to be
    // verified here rather than trusted by shape alone.
    if let Some(auth_header) = request.headers().get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && is_api_key_header(auth_str)
        && let Some(manager) = &state.api_keys
        && extract_api_key(auth_str, manager).await.is_some()
    {
        return Ok(next.run(request).await);
    }

    let Some(session) = request.extensions().get::<Session>() else {
        // No session layer means the gate cannot see any cookie. The only
        // caller that can have authenticated without one is a verified client
        // certificate (the API-key header was exempted above); anything else
        // fails closed.
        if request
            .extensions()
            .get::<ferris_core::ClientIdentity>()
            .is_some()
        {
            return Ok(next.run(request).await);
        }
        return Err(csrf_forbidden());
    };

    // A caller authenticated by a verified client certificate and carrying no
    // session cookie has no session for CSRF to protect: the certificate is not
    // ambient browser auth, so a cross-site form cannot ride it (H1 / Fase 2).
    // Enforced only when there is no cookie session, so a browser that *does*
    // hold a session still needs the token as before.
    if session.id().is_none()
        && request
            .extensions()
            .get::<ferris_core::ClientIdentity>()
            .is_some()
    {
        return Ok(next.run(request).await);
    }

    let Some(token) = request
        .headers()
        .get(&CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
    else {
        return Err(csrf_forbidden());
    };
    match crate::login::csrf_token(session).await {
        Some(stored) if constant_time_eq(stored.as_bytes(), token.as_bytes()) => {
            Ok(next.run(request).await)
        }
        _ => Err(csrf_forbidden()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
