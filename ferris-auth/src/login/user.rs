//! The session user: [`ferris_core::User`] plus its session validity stamp.
//!
//! `axum-login` stores an opaque `session_auth_hash` in the session record and
//! compares it with the one the backend computes on every request
//! (`AuthSession::from_session`). The hash is what makes a session invalidation
//! real: as soon as the value the backend derives changes, the stored session
//! no longer matches and `axum-login` flushes it.
//!
//! [`ferris_core::User`] has no place to keep that stamp (it is a
//! storage-agnostic identity), so [`SessionUser`] is the thin wrapper that
//! carries it. Handlers receive it from
//! `axum_login::AuthSession<Backend>::user()`, not from a bespoke extractor.

use axum_login::AuthUser;
use ferris_core::User;
use sha2::{Digest, Sha256};

/// Domain separator for [`session_fingerprint`].
///
/// Bumping this string invalidates every existing session once, which is the
/// intended leverage if the recipe below ever has to change.
pub const SESSION_FINGERPRINT_DOMAIN: &str = "ferris.auth.session.v1";

/// Hash the credential material a session is bound to.
///
/// The value is `SHA-256` over the domain separator and the following fields,
/// each prefixed with its length so concatenation cannot be ambiguous:
///
/// 1. the user id,
/// 2. the stored Argon2 PHC hash (empty when the account has none),
/// 3. the `mfa_enabled` flag (one byte),
/// 4. the TOTP seed as stored (empty when there is none).
///
/// Consequences, in order:
///
/// * **it is stable** across requests, replicas and restarts, because it only
///   depends on rows that live in the database;
/// * **a password change invalidates existing sessions**, because the engine
///   stores a fresh Argon2 hash (new salt) on every write;
/// * **enabling, disabling or rotating MFA invalidates them too**, because the
///   flag and the seed are part of the input;
/// * the session record never carries the hash or the seed itself, only this
///   32-byte digest.
///
/// The reverse is not claimed: two different rows can hash to the same value
/// only with a SHA-256 collision, which is out of scope.
#[must_use]
pub fn session_fingerprint(
    user_id: &str,
    password_hash: Option<&str>,
    mfa_enabled: bool,
    mfa_secret: Option<&str>,
) -> Vec<u8> {
    let mut hasher = Sha256::new();
    absorb(&mut hasher, SESSION_FINGERPRINT_DOMAIN.as_bytes());
    absorb(&mut hasher, user_id.as_bytes());
    absorb(&mut hasher, password_hash.unwrap_or_default().as_bytes());
    absorb(&mut hasher, &[u8::from(mfa_enabled)]);
    absorb(&mut hasher, mfa_secret.unwrap_or_default().as_bytes());
    hasher.finalize().to_vec()
}

/// Feed one length-prefixed field into `hasher`.
fn absorb(hasher: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
}

/// A logged-in user, as `axum-login` stores it in the session.
///
/// Dereferences to [`User`], so a handler reads `user.email`, `user.roles` and
/// the rest exactly as before.
#[derive(Clone)]
pub struct SessionUser {
    user: User,
    auth_hash: Vec<u8>,
}

/// `Debug` is redacted: `axum-login` compares `auth_hash` instead of a stored
/// session secret, so printing it would hand a reader the session's validity
/// stamp. The identity stays visible.
impl std::fmt::Debug for SessionUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionUser")
            .field("user", &self.user)
            .field("auth_hash", &"<redacted>")
            .finish()
    }
}

impl SessionUser {
    /// Pair a user with its session validity stamp.
    ///
    /// Callers should use [`crate::user_store::LoginRecord::session_auth_hash`]
    /// (or [`session_fingerprint`]) rather than inventing a stamp: the backend
    /// recomputes it on every request and a different recipe would log the user
    /// out immediately.
    #[must_use]
    pub const fn new(user: User, auth_hash: Vec<u8>) -> Self {
        Self { user, auth_hash }
    }

    /// The identity behind the session.
    #[must_use]
    pub const fn user(&self) -> &User {
        &self.user
    }

    /// Consume the wrapper and return the identity.
    #[must_use]
    pub fn into_user(self) -> User {
        self.user
    }
}

impl std::ops::Deref for SessionUser {
    type Target = User;

    fn deref(&self) -> &Self::Target {
        &self.user
    }
}

impl AsRef<User> for SessionUser {
    fn as_ref(&self) -> &User {
        &self.user
    }
}

impl From<SessionUser> for User {
    fn from(user: SessionUser) -> Self {
        user.user
    }
}

impl AuthUser for SessionUser {
    type Id = String;

    fn id(&self) -> Self::Id {
        self.user.id.clone()
    }

    fn session_auth_hash(&self) -> &[u8] {
        &self.auth_hash
    }
}
