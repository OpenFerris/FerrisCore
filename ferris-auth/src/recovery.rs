//! Password reset and email verification.
//!
//! Both flows are the same primitive: mint a 32-byte random token, persist only
//! its SHA-256 hex digest with a deadline, hand the plaintext to the caller
//! **once**, and accept the token exactly once before it expires.
//!
//! # Sending the email is not this crate's job
//!
//! These services never send mail and must not depend on `ferris-email`: they
//! return the token to the caller (the API layer), which renders the message and
//! sends it through [`ferris_core::ports::MailerPort`]. Keeping the transport
//! out of here means the token never has to sit in a second component's state,
//! and a deployment without a mailer can still run the flows.
//!
//! # Storage
//!
//! `User.reset_password_key`, `User.reset_password_expires_at`,
//! `User.email_verification_key` and `User.email_verification_expires_at` are
//! ordinary hidden columns at `permlevel` 1. Each key column stores
//! `sha256_hex(token)`, never the token, so a leaked row cannot be turned into a
//! working link; the presented token is re-hashed and compared with
//! [`crate::crypto::constant_time_eq`].
//!
//! # No enumeration
//!
//! [`PasswordResetService::request_reset`] answers `Ok(None)` for an unknown or
//! disabled account instead of failing, so the HTTP layer can answer the same
//! `204` it answers for a real account and no status code, error or message
//! reveals whether the address has an account. Only the side effect differs:
//! for a known account the caller also receives a token and sends the mail.
//!
//! # Single use
//!
//! Confirmation first **clears the pending columns with one conditional
//! `UPDATE ... WHERE name = ? AND key = ?`**. Only the statement that clears a
//! live digest wins, so two concurrent confirmations cannot both succeed, and a
//! second attempt with the same token finds nothing. If the engine then rejects
//! the new password (policy), the token stays consumed on purpose: the caller
//! must request a new link rather than retry a credential that failed.
//!
//! All timestamps are RFC 3339 strings, parsed with
//! [`chrono::DateTime::parse_from_rfc3339`] through [`expires_at`].

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use ferris_core::{Map, Result, Value};
use ferris_db::Stmt;
use rand::RngCore;
use std::fmt;

use crate::crypto::constant_time_eq;
use crate::user_store::{StoredUser, UserStore};

/// Bytes of entropy in a token: 256 bits, far beyond guessable.
const TOKEN_BYTES: usize = 32;

/// The `User` column holding the reset token digest.
pub const RESET_KEY_FIELD: &str = "reset_password_key";
/// The `User` column holding the reset deadline.
pub const RESET_EXPIRES_FIELD: &str = "reset_password_expires_at";
/// The `User` column holding the verification token digest.
pub const VERIFICATION_KEY_FIELD: &str = "email_verification_key";
/// The `User` column holding the verification deadline.
pub const VERIFICATION_EXPIRES_FIELD: &str = "email_verification_expires_at";

/// A freshly issued password-reset token.
///
/// The plaintext `token` is a live credential: it is returned once so the caller
/// can put it in a link, and its `Debug` output is redacted so it cannot reach a
/// log line through a `?` on a struct.
#[derive(Clone)]
pub struct ResetToken {
    /// The account the token belongs to.
    pub user_id: String,
    /// The plaintext token, to be embedded in the reset link. Never stored.
    pub token: String,
    /// When the token stops being accepted.
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for ResetToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResetToken")
            .field("user_id", &self.user_id)
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A freshly issued email-verification token. Same redaction rule as
/// [`ResetToken`].
#[derive(Clone)]
pub struct VerificationToken {
    /// The account the token belongs to.
    pub user_id: String,
    /// The plaintext token, to be embedded in the verification link.
    pub token: String,
    /// When the token stops being accepted.
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for VerificationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerificationToken")
            .field("user_id", &self.user_id)
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Password-reset flow over the [`UserStore`].
#[derive(Debug, Clone)]
pub struct PasswordResetService {
    store: UserStore,
}

impl PasswordResetService {
    /// Wrap an already-initialised store.
    #[must_use]
    pub const fn new(store: UserStore) -> Self {
        Self { store }
    }

    /// The store the service writes through.
    #[must_use]
    pub const fn store(&self) -> &UserStore {
        &self.store
    }

    /// Issue a reset token for `email`.
    ///
    /// `ttl` is how long the link stays valid. Unknown, disabled and
    /// non-matching accounts answer `Ok(None)`; they are indistinguishable to
    /// the caller, which is what keeps the endpoint from enumerating users.
    /// An existing pending token is replaced.
    ///
    /// # Errors
    ///
    /// Storage errors (the caller should still answer a generic `204`).
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn request_reset(&self, email: &str, ttl: Duration) -> Result<Option<ResetToken>> {
        let email = email.trim();
        if email.is_empty() {
            return Ok(None);
        }
        let Some(user) = self.store.find_by_login(email).await? else {
            return Ok(None);
        };
        // `find_by_login` also matches usernames; a reset must target an
        // address, and only the account that owns it may receive the link.
        if !user.enabled || !user.email.eq_ignore_ascii_case(email) {
            return Ok(None);
        }

        let token = random_token();
        let expires_at = Utc::now() + ttl;
        let mut fields = Map::new();
        fields.insert(RESET_KEY_FIELD.to_string(), Value::from(digest_of(&token)));
        fields.insert(
            RESET_EXPIRES_FIELD.to_string(),
            Value::from(expires_at.to_rfc3339()),
        );
        self.store.patch(&user.id, fields).await?;
        Ok(Some(ResetToken {
            user_id: user.id,
            token,
            expires_at,
        }))
    }

    /// Spend a reset token and set the account's new password.
    ///
    /// Answers `Ok(None)` for an unknown, expired, already-used or
    /// disabled-account token. The token is consumed by an atomic clear before
    /// the password is written, so it can never be used twice.
    ///
    /// # Errors
    ///
    /// Storage errors, and the engine's validation error when the new password
    /// violates the policy (the token stays consumed).
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn confirm_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<Option<StoredUser>> {
        let Some(document) = self.pending_document(RESET_KEY_FIELD, token).await? else {
            return Ok(None);
        };
        let id = text(&document, "name");
        if id.is_empty() || !enabled(&document) || !live(&document, RESET_EXPIRES_FIELD) {
            return Ok(None);
        }
        if !clear_pending(
            &self.store,
            &id,
            RESET_KEY_FIELD,
            RESET_EXPIRES_FIELD,
            token,
        )
        .await?
        {
            // Another confirmation consumed the token first.
            return Ok(None);
        }
        self.store.set_password(&id, new_password).await?;
        self.store.find_by_id(&id).await
    }

    /// The user document whose `field` holds the digest of `token`.
    ///
    /// The digest is used as the filter *and* compared again in constant time,
    /// so a malformed or mismatched stored value fails closed.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    async fn pending_document(&self, field: &str, token: &str) -> Result<Option<Value>> {
        pending_document(&self.store, field, token).await
    }
}

/// Email-verification flow over the [`UserStore`].
#[derive(Debug, Clone)]
pub struct EmailVerificationService {
    store: UserStore,
}

impl EmailVerificationService {
    /// Wrap an already-initialised store.
    #[must_use]
    pub const fn new(store: UserStore) -> Self {
        Self { store }
    }

    /// The store the service writes through.
    #[must_use]
    pub const fn store(&self) -> &UserStore {
        &self.store
    }

    /// Issue a verification token for `user_id`.
    ///
    /// Idempotent in the safe direction: an already-verified account answers
    /// `Ok(None)` (there is nothing to prove), and asking again simply replaces
    /// the pending token. An unknown or disabled account also answers
    /// `Ok(None)`, so the caller cannot probe for accounts.
    ///
    /// # Errors
    ///
    /// Storage errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn request_verification(
        &self,
        user_id: &str,
        ttl: Duration,
    ) -> Result<Option<VerificationToken>> {
        let Some(document) = self.store.load(user_id).await? else {
            return Ok(None);
        };
        if !enabled(&document) || verified(&document) {
            return Ok(None);
        }

        let token = random_token();
        let expires_at = Utc::now() + ttl;
        let mut fields = Map::new();
        fields.insert(
            VERIFICATION_KEY_FIELD.to_string(),
            Value::from(digest_of(&token)),
        );
        fields.insert(
            VERIFICATION_EXPIRES_FIELD.to_string(),
            Value::from(expires_at.to_rfc3339()),
        );
        self.store.patch(user_id, fields).await?;
        Ok(Some(VerificationToken {
            user_id: user_id.to_string(),
            token,
            expires_at,
        }))
    }

    /// Mark the address verified and consume the token.
    ///
    /// Answers `Ok(None)` for an unknown, expired or already-used token. An
    /// account that is already verified stays verified; its token was cleared
    /// by the first success, so a second attempt simply finds nothing.
    ///
    /// # Errors
    ///
    /// Storage errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn confirm_verification(&self, token: &str) -> Result<Option<StoredUser>> {
        if token.trim().is_empty() {
            return Ok(None);
        }
        let Some(document) = pending_document(&self.store, VERIFICATION_KEY_FIELD, token).await?
        else {
            return Ok(None);
        };
        let id = text(&document, "name");
        if id.is_empty() || !enabled(&document) || !live(&document, VERIFICATION_EXPIRES_FIELD) {
            return Ok(None);
        }
        if !clear_pending(
            &self.store,
            &id,
            VERIFICATION_KEY_FIELD,
            VERIFICATION_EXPIRES_FIELD,
            token.trim(),
        )
        .await?
        {
            return Ok(None);
        }
        let mut fields = Map::new();
        fields.insert("email_verified".to_string(), Value::from(true));
        self.store.patch(&id, fields).await?;
        self.store.find_by_id(&id).await
    }
}

/// A new URL-safe, unpadded 32-byte token.
fn random_token() -> String {
    let mut buffer = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

/// The user document whose `field` holds the digest of `token`.
///
/// The digest is used as the filter *and* compared again in constant time, so a
/// malformed or mismatched stored value fails closed.
///
/// # Errors
///
/// Storage errors from the lookup.
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
async fn pending_document(store: &UserStore, field: &str, token: &str) -> Result<Option<Value>> {
    let token = token.trim();
    if token.is_empty() {
        return Ok(None);
    }
    let digest = digest_of(token);
    let Some(document) = store.first_by(field, &digest).await? else {
        return Ok(None);
    };
    if !stored_digest_matches(document.get(field), &digest) {
        return Ok(None);
    }
    Ok(Some(document))
}

/// The stored form of `token`.
fn digest_of(token: &str) -> String {
    crate::crypto::sha256_hex(token.as_bytes())
}

/// Whether a stored digest column holds exactly the digest of the presented
/// token, compared in constant time.
fn stored_digest_matches(stored: Option<&Value>, computed: &str) -> bool {
    stored.and_then(Value::as_str).is_some_and(|stored| {
        stored.len() == computed.len()
            && stored.bytes().all(|byte| byte.is_ascii_hexdigit())
            && constant_time_eq(stored.as_bytes(), computed.as_bytes())
    })
}

/// A string column of a user document.
fn text(document: &Value, name: &str) -> String {
    document
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The `Check` column, defaulting to enabled like `document_to_stored` does.
fn enabled(document: &Value) -> bool {
    document
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// The `email_verified` flag.
fn verified(document: &Value) -> bool {
    document
        .get("email_verified")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether the RFC 3339 deadline in `field` is still in the future.
///
/// A missing or unparseable deadline counts as expired: fail closed.
fn live(document: &Value, field: &str) -> bool {
    expires_at(document, field).is_some_and(|expires_at| expires_at > Utc::now())
}

/// Parse an RFC 3339 timestamp column.
fn expires_at(document: &Value, field: &str) -> Option<DateTime<Utc>> {
    document
        .get(field)
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc))
}

/// Clear `key_field`/`expires_field` for `id` **only** when the key still holds
/// the digest of `token`.
///
/// One conditional statement, so it is the single-use gate: `true` means this
/// call consumed the token, `false` means it was already consumed or replaced.
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
async fn clear_pending(
    store: &UserStore,
    id: &str,
    key_field: &str,
    expires_field: &str,
    token: &str,
) -> Result<bool> {
    let table = store.user_table()?;
    let name = store.user_column("name")?;
    let key = store.user_column(key_field)?;
    let expires = store.user_column(expires_field)?;
    let affected = store
        .db()
        .execute(&Stmt::new(
            format!(
                "UPDATE {table} SET {key} = NULL, {expires} = NULL \
                 WHERE {name} = ? AND {key} = ?"
            ),
            vec![Value::from(id), Value::from(digest_of(token))],
        ))
        .await?;
    Ok(affected.rows() > 0)
}

#[cfg(test)]
mod tests;
