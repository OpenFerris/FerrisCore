//! Authentication middleware for Axum.
//!
//! The public surface is declared here and re-exported from the three
//! submodules so every previous path (`middleware::AuthState`,
//! `middleware::require_csrf`, …) keeps resolving:
//!
//! * `state` — [`AuthState`], the extractors and [`resolve_principal`];
//! * `csrf` — [`CSRF_HEADER`], the cookie reader and [`require_csrf`];
//! * `lockout` — [`RateLimiter`], [`AccountLockout`] and the persisted
//!   lockout functions.
//!
//! The session-revocation port lives here because it is process-wide and does
//! not belong to any one gate.

mod csrf;
mod lockout;
mod state;

pub use csrf::{CSRF_HEADER, cookie_value, require_csrf, session_cookie};
pub use lockout::{
    AccountLockout, RateLimiter, account_lockout, clear_account_lockout, record_account_failure,
};
pub use state::{
    AuthState, AuthenticatedUser, OptionalUser, UserValidator, require_auth, resolve_principal,
};

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
///
/// The session endpoints call this after deleting the row(s): a realtime socket
/// has no way to observe the store by itself, so the close has to be pushed.
pub fn notify_session_revoked(user_id: &str, session_id: Option<&str>) {
    if let Some(hook) = SESSION_REVOKED_HOOK.get() {
        hook(user_id, session_id);
    }
}
