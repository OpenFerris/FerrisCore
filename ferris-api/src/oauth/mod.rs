//! OAuth2 HTTP endpoints — a **login**, not a token dispenser.
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
//! store would hand out tokens under the SSO flow's trust, so it is deliberately
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
//!
//! Split into fragments: [`routes`] owns the handlers, [`tests`] the suite, and
//! this `mod.rs` only declares and re-exports so `crate::oauth::Thing` and
//! `ferris_api::oauth_router*` keep resolving.

mod routes;

#[cfg(test)]
mod tests;

pub use routes::{
    DEFAULT_NEXT, OAuthState, oauth_router, oauth_router_with_manager, oauth_router_with_state,
};
