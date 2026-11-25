//! Session login on top of `axum-login` + `tower-sessions`.
//!
//! This is **the** session/auth stack: the server mounts
//! [`wiring::auth_layer`], the routes log in through
//! `axum_login::AuthSession<Backend>`, and the session records live in
//! [`DbSessionStore`] (`tab_session_store`). There is no second store and no
//! bearer token: a cookie session is the only browser credential, alongside the
//! Frappe API key for machine callers.
//!
//! # What it provides
//!
//! * [`SessionUser`] — [`ferris_core::User`] plus the `axum_login::AuthUser`
//!   implementation. `id()` is the user id; `session_auth_hash()` is
//!   [`session_fingerprint`], a SHA-256 over the id and the account's credential
//!   material. A password or MFA change therefore invalidates every existing
//!   session.
//! * [`Backend`] — `axum_login::AuthnBackend`. It resolves the account through
//!   [`crate::UserStore`], verifies Argon2 off the async workers, checks TOTP
//!   with `totp-rs` when `mfa_enabled` is set, refuses a replayed TOTP step, and
//!   re-derives a password hash written under older Argon2 parameters.
//! * [`DbSessionStore`] — `tower_sessions::SessionStore` (and
//!   `ExpiredDeletion`) over [`ferris_db::Db`], table `tab_session_store`.
//! * [`SessionConfig`] — the cookie name/attributes and the idle/absolute
//!   lifetimes. `__Host-ferris_session` is selected when TLS is configured.
//! * [`wiring`] — the two layers, the per-session keys (CSRF, owner, absolute
//!   deadline, pending MFA) and the extension resolver the middleware and the
//!   realtime handshake share.
//!
//! # Credential failures never say why
//!
//! Unknown account, wrong password, missing/wrong/replayed TOTP code: all of
//! them answer `Ok(None)`. Unknown accounts additionally burn one Argon2
//! verification against a fixed dummy hash so response *time* does not reveal
//! whether the login exists. [`LoginError`] is only for infrastructure failures
//! (the store could not be read).
//!
//! # Mounting
//!
//! 1. Create the store eagerly (`ensure_table`, `spawn_cleanup_task`).
//! 2. `Backend::init()` creates the user table and installs the Argon2 hasher.
//! 3. Apply [`wiring::auth_layer`] **outside** every auth gate and, when the
//!    socket.io engine is mounted, outside it too, so the handshake request
//!    carries the `AuthSession` and `Session` extensions.
//!
//! # TOTP replay protection is persisted
//!
//! `totp-rs` returns the time step a code matched and explicitly leaves
//! single-use enforcement to the caller. The account row carries
//! `mfa_last_step`, and [`Backend`] spends a matched step through
//! [`crate::UserStore::claim_totp_step`]: one atomic conditional
//! `UPDATE ... SET mfa_last_step = ? WHERE name = ? AND (mfa_last_step IS NULL
//! OR mfa_last_step < ?)`. The login is accepted only when that statement moved
//! the account forward, so a code cannot be replayed — not on this instance,
//! not against another replica, and not after a restart. Tracking the last
//! *spent* step rather than the current one keeps the next code usable for as
//! long as its window is valid.
//!
//! # Password rehashing
//!
//! After a successful verify, [`crate::password::needs_rehash`] compares the
//! stored parameters with [`crate::PasswordConfig::default`] — the parameters
//! the store installs as the process-wide hasher. A stale hash is re-derived
//! through the document engine (which owns the `Password` fieldtype) before the
//! session stamp is computed, so the freshly issued session matches the new
//! row. A failed rehash is logged and the login still succeeds.

mod backend;
mod config;
pub mod keys;
mod session_store;
mod user;
pub mod wiring;

#[cfg(test)]
mod tests;

pub use backend::{Backend, Credentials, LoginError};
pub use config::{INSECURE_COOKIE_NAME, SECURE_COOKIE_NAME, SessionConfig};
pub use session_store::{DbSessionStore, SESSION_TABLE, SessionIdentity};
pub use user::{SESSION_FINGERPRINT_DOMAIN, SessionUser, session_fingerprint};
pub use wiring::{
    PendingMfa, auth_layer, csrf_from_extensions, csrf_token, ensure_csrf_token,
    principal_from_extensions, session_is_live, session_layer, set_pending_mfa, stamp_session,
    take_pending_mfa,
};
