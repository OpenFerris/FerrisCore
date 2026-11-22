//! Cookies that bind an OAuth callback to the browser that started the flow.
//!
//! The provider's redirect is a **cross-site top-level GET**, so the binding
//! cannot live in a header (`X-OAuth-State` was readable by any script and lost
//! on navigation) or in `SameSite=Strict` (the browser would not send it back).
//! It is a short-lived, `HttpOnly`, `SameSite=Lax` cookie scoped to
//! `/api/oauth`, so:
//!
//! - only this origin's `/api/oauth/*` requests see it, and no script can read
//!   it — the state never travels through JavaScript;
//! - a callback that did not start in this browser (an attacker replaying a
//!   `code`/`state` pair, or a different tab/profile) fails the comparison and
//!   is refused before the code is exchanged;
//! - the cookie dies with the flow: single use, cleared by the callback, and
//!   `Max-Age` matches the manager's server-side state TTL.
//!
//! [`NEXT_COOKIE`] carries the post-login destination for the same reason: the
//! provider echoes only `state` and `code`, and the redirect URI is fixed, so
//! the destination cannot ride through the provider. It is validated on the way
//! in and on the way out (see `ferris_api::oauth`), never trusted.

use cookie::{Cookie, SameSite};

/// The one-time `state` minted by [`OAuthManager::authorize`](crate::oauth::OAuthManager::authorize).
pub const STATE_COOKIE: &str = "ferris_oauth_state";

/// The Desk path to land on once the session exists (`/desk` when absent).
pub const NEXT_COOKIE: &str = "ferris_oauth_next";

/// Both cookies are scoped to the OAuth surface, not the whole site.
const COOKIE_PATH: &str = "/api/oauth";

/// Build the `Set-Cookie` value for one flow cookie.
///
/// `secure` follows the session configuration: a deployment behind plain HTTP
/// on localhost cannot use `Secure`, and the OAuth state is still protected by
/// `HttpOnly` + `SameSite` + the short TTL.
#[must_use]
pub fn flow_cookie(name: &str, value: &str, max_age_seconds: u64, secure: bool) -> Cookie<'static> {
    Cookie::build(name.to_string(), value.to_string())
        .path(COOKIE_PATH)
        // Required: a cross-site top-level navigation sends `Lax`, not `Strict`.
        .same_site(SameSite::Lax)
        .http_only(true)
        .secure(secure)
        .max_age(cookie::time::Duration::seconds(
            i64::try_from(max_age_seconds).unwrap_or(i64::MAX),
        ))
        .finish()
}

/// Build the `Set-Cookie` value that deletes one flow cookie.
#[must_use]
pub fn clear_cookie(name: &str, secure: bool) -> Cookie<'static> {
    Cookie::build(name.to_string(), String::new())
        .path(COOKIE_PATH)
        .same_site(SameSite::Lax)
        .http_only(true)
        .secure(secure)
        .max_age(cookie::time::Duration::ZERO)
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_cookie_is_http_only_lax_and_scoped_to_the_flow() {
        let cookie = flow_cookie(STATE_COOKIE, "abc", 600, true);
        let rendered = cookie.to_string();
        assert!(rendered.starts_with("ferris_oauth_state=abc"), "{rendered}");
        assert!(rendered.contains("HttpOnly"), "{rendered}");
        assert!(rendered.contains("SameSite=Lax"), "{rendered}");
        assert!(rendered.contains("Secure"), "{rendered}");
        assert!(rendered.contains("Path=/api/oauth"), "{rendered}");
        assert!(rendered.contains("Max-Age=600"), "{rendered}");
    }

    /// `Secure` off is the localhost/dev case; the rest of the attributes must
    /// not follow it away.
    #[test]
    fn the_state_cookie_without_tls_keeps_the_other_attributes() {
        let rendered = flow_cookie(STATE_COOKIE, "abc", 60, false).to_string();
        assert!(!rendered.contains("Secure"), "{rendered}");
        assert!(rendered.contains("HttpOnly"), "{rendered}");
        assert!(rendered.contains("SameSite=Lax"), "{rendered}");
    }

    #[test]
    fn clearing_matches_the_cookie_scope() {
        let rendered = clear_cookie(NEXT_COOKIE, true).to_string();
        assert!(rendered.starts_with("ferris_oauth_next="), "{rendered}");
        assert!(rendered.contains("Max-Age=0"), "{rendered}");
        assert!(rendered.contains("Path=/api/oauth"), "{rendered}");
    }
}
