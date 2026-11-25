//! Session keys shared by login, logout, CSRF and the realtime handshake.
//!
//! All of them live inside the single `tower-sessions` record. `axum-login`
//! owns its own private key for the authenticated identity; these are the
//! framework's additions, used where a component needs one fact without going
//! through an `AuthSession` (the CSRF gate, "sign out everywhere", a socket
//! handshake) or where the absolute lifetime has to outlive the idle window.
//!
//! The values are deliberately small and non-secret except the CSRF token,
//! which is a capability: it is compared in constant time and never logged.

/// The account id behind the session (`String`).
///
/// Denormalised out of `axum-login`'s record so
/// [`crate::login::DbSessionStore`] can index sessions by user and so a
/// component holding only the session record can answer "whose is this?".
pub const USER_ID_KEY: &str = "ferris.user_id";

/// The account's email at login time (`String`).
///
/// The realtime handshake resolves a socket from the session record alone; it
/// has no `axum-login` layer in front of the upgrade, so the identity has to be
/// in the record. It is revalidated against the account row on every
/// maintenance tick, exactly like the REST path.
pub const EMAIL_KEY: &str = "ferris.email";

/// The account's roles at login time (`Vec<String>`), for the same handshake.
pub const ROLES_KEY: &str = "ferris.roles";

/// The per-session CSRF token (`String`).
pub const CSRF_TOKEN_KEY: &str = "ferris.csrf_token";

/// The absolute expiry as a Unix timestamp in seconds (`i64`).
///
/// `tower-sessions` only carries one expiry, used for the sliding idle window.
/// This key is the second, hard limit: the auth middleware refuses a session
/// past it even if the idle window keeps being refreshed.
pub const ABSOLUTE_EXPIRY_KEY: &str = "ferris.absolute_expiry";

/// A password step awaiting its second factor.
///
/// Serialised from [`crate::login::PendingMfa`]: the account id, the challenge
/// the Desk echoes back and the instant it stops being valid.
pub const MFA_PENDING_KEY: &str = "ferris.mfa_pending";
