//! Credential surface of the [`UserStore`] used by [`crate::login`].
//!
//! The login backend needs three things the generic account row deliberately
//! does not carry: the stored Argon2 hash (to verify and to fingerprint the
//! session), the TOTP seed, and a way to persist a freshly derived hash when
//! the stored one is stale. All three are `permlevel` 1 / masked fields, so
//! they are read here through the store's `System Manager` context — the same
//! trusted path [`super::UserStore::verify_login`] and the MFA surface use.
//!
//! Split out of `mod.rs`/`mfa.rs` so neither fragment grows past the size
//! limit and the credential material has exactly one reader.

use ferris_core::{Map, Result, Value};
use ferris_db::{Dialect, Stmt};

use crate::middleware::AccountLockout;

use super::{StoredUser, UserStore, document_to_stored};

/// The credential material a login needs, read in a single document lookup.
///
/// The `Debug` implementation would print `password_hash` and `mfa_secret`, so
/// this type is deliberately **not** `Debug` and is never logged or serialized.
#[derive(Clone)]
pub struct LoginRecord {
    /// The account row (id, roles, `mfa_enabled`, …) as the session builds it.
    pub user: StoredUser,
    /// The stored Argon2 PHC hash, as written by the document engine.
    pub password_hash: String,
    /// The TOTP seed (RFC 4648 Base32), when the account has one configured.
    /// It may exist while `user.mfa_enabled` is still off: that is the
    /// provisioning state the enable flow writes before the first code
    /// verifies.
    pub mfa_secret: Option<String>,
}

impl LoginRecord {
    /// The opaque, server-side value a session stores as its validity stamp.
    ///
    /// See [`crate::login::session_fingerprint`] for exactly what is hashed and
    /// why a password or MFA change invalidates existing sessions.
    #[must_use]
    pub fn session_auth_hash(&self) -> Vec<u8> {
        crate::login::session_fingerprint(
            &self.user.id,
            Some(&self.password_hash),
            self.user.mfa_enabled,
            self.mfa_secret.as_deref(),
        )
    }
}

impl UserStore {
    /// Document → credentials, or `None` for a disabled account. An account
    /// with no stored password is still a valid identity (OAuth); only the
    /// password verification refuses it.
    ///
    /// `mfa_secret` is a `Secret` field, so the document only carries the read
    /// mask. The plaintext is fetched through [`Self::mfa_secret`] and only when
    /// the account actually has a seed, so the common MFA-less login does not
    /// pay for a second read.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    async fn record_from_document(&self, document: &Value) -> Result<Option<LoginRecord>> {
        if !document
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            return Ok(None);
        }
        // A password-less account is valid: OAuth and other external identities
        // have no local password, and the session rehydration path
        // (`Backend::get_user`) must still load them. The password login path
        // simply fails verification against the empty hash.
        let password_hash = document
            .get("password")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let user = document_to_stored(document)?;
        let has_secret = document
            .get("mfa_secret")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let mfa_secret = if has_secret {
            self.mfa_secret(&user.id).await?
        } else {
            None
        };
        Ok(Some(LoginRecord {
            user,
            password_hash,
            mfa_secret,
        }))
    }

    /// Resolve the credentials for an email or username.
    ///
    /// Answers `Ok(None)` for an unknown, disabled or password-less account, so
    /// the caller cannot tell those apart from the login gate.
    ///
    /// # Errors
    ///
    /// Storage or document-decoding errors. A database failure is the only
    /// condition the login backend reports as a typed error.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn login_record(&self, login: &str) -> Result<Option<LoginRecord>> {
        let Some(document) = self.document_by_login(login).await? else {
            return Ok(None);
        };
        self.record_from_document(&document).await
    }

    /// Resolve the credentials for a user id (the session-rehydration path).
    ///
    /// # Errors
    ///
    /// Storage or document-decoding errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn login_record_by_id(&self, id: &str) -> Result<Option<LoginRecord>> {
        let Some(document) = self.document_by_id(id).await? else {
            return Ok(None);
        };
        self.record_from_document(&document).await
    }

    /// Replace the account's password with `password`.
    ///
    /// The value written is the **plaintext**: the document engine owns
    /// hashing (`Password` fieldtype), so passing a PHC string here would get
    /// it hashed a second time. Used by the login backend to re-derive a hash
    /// written under older Argon2 parameters.
    ///
    /// # Errors
    ///
    /// Validation (the current password policy), permission or storage errors
    /// from the engine. A no-op for an unknown user.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn set_password(&self, id: &str, password: &str) -> Result<()> {
        let mut fields = Map::new();
        fields.insert("password".to_string(), Value::from(password));
        self.patch(id, fields).await
    }

    /// The account's persisted lockout state; a missing row reads as unlocked.
    ///
    /// Reads through the engine so the row is materialised and decoded exactly
    /// like every other document read, including an account that does not exist
    /// yet.
    ///
    /// # Errors
    ///
    /// Storage or document-decoding errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn account_lockout(&self, id: &str) -> Result<AccountLockout> {
        let meta = crate::doctypes::user_meta();
        let ctx = crate::store_support::system_context(&meta);
        match self.engine.get(&ctx, id).await {
            Ok(document) => Ok(AccountLockout {
                failed_login_attempts: u32::try_from(
                    document
                        .get("failed_login_attempts")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        .clamp(0, i64::from(u32::MAX)),
                )
                .unwrap_or(u32::MAX),
                locked_until: document
                    .get("locked_until")
                    .and_then(Value::as_str)
                    .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                    .map(|until| until.with_timezone(&chrono::Utc)),
            }),
            Err(ferris_core::Error::Document(ferris_core::DocumentError::NotFound { .. })) => {
                Ok(AccountLockout::default())
            }
            Err(error) => Err(error),
        }
    }

    /// Count one failed login and lock the account once the budget is spent.
    ///
    /// The increment is a **single atomic `UPDATE`** (`failed_login_attempts =
    /// COALESCE(failed_login_attempts, 0) + 1`), so two concurrent failures
    /// cannot both read `n` and both write `n + 1`. `locked_until` is assigned
    /// first on purpose: MySQL evaluates a single-table `UPDATE`'s assignments
    /// left to right, so computing the deadline before the counter changes is
    /// what keeps its condition based on the previous count.
    ///
    /// The counter and the deadline follow the same rule as
    /// [`crate::RateLimiter`] and [`ferris_core::User::record_failed_login`], so
    /// the in-memory and persisted answers cannot disagree.
    ///
    /// # Errors
    ///
    /// Storage errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn record_failed_login(
        &self,
        id: &str,
        max_attempts: u32,
        lockout_minutes: u64,
    ) -> Result<AccountLockout> {
        let table = self.user_table()?;
        let name = self.user_column("name")?;
        let attempts = self.user_column("failed_login_attempts")?;
        let locked = self.user_column("locked_until")?;
        let deadline = chrono::Utc::now()
            + chrono::Duration::minutes(i64::try_from(lockout_minutes).unwrap_or(i64::MAX));
        let deadline = deadline.format("%Y-%m-%d %H:%M:%S").to_string();
        let placeholder = datetime_placeholder(self.db().dialect());
        let sql = format!(
            "UPDATE {table} SET \
             {locked} = CASE WHEN COALESCE({attempts}, 0) + 1 >= ? \
                             THEN {placeholder} ELSE {locked} END, \
             {attempts} = COALESCE({attempts}, 0) + 1 \
             WHERE {name} = ?"
        );
        self.db()
            .execute(&Stmt::new(
                sql,
                vec![
                    Value::from(i64::from(max_attempts)),
                    Value::from(deadline),
                    Value::from(id),
                ],
            ))
            .await?;
        self.account_lockout(id).await
    }

    /// Clear the persisted lockout after the **whole** login succeeded.
    ///
    /// One write, with no read before it: a completed login does not care what
    /// the previous counter was.
    ///
    /// # Errors
    ///
    /// Storage errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn clear_lockout(&self, id: &str) -> Result<()> {
        let table = self.user_table()?;
        let name = self.user_column("name")?;
        let attempts = self.user_column("failed_login_attempts")?;
        let locked = self.user_column("locked_until")?;
        self.db()
            .execute(&Stmt::new(
                format!("UPDATE {table} SET {attempts} = 0, {locked} = NULL WHERE {name} = ?"),
                vec![Value::from(id)],
            ))
            .await?;
        Ok(())
    }
}

/// The placeholder a `Datetime` column needs on each backend.
///
/// `locked_until` is native `TIMESTAMP`/`DATETIME` on Postgres and MySQL; the
/// space-separated string the write binds only lands there through the cast.
/// SQLite stores dates as text and takes the value directly.
const fn datetime_placeholder(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres => "CAST(? AS TIMESTAMP)",
        Dialect::MySQL => "CAST(? AS DATETIME)",
        Dialect::SQLite => "?",
    }
}

impl UserStore {
    /// Mark (or clear) the account's verified-email flag.
    ///
    /// [`crate::recovery`] owns the self-service flow; OAuth uses this once the
    /// provider proved the address.
    ///
    /// # Errors
    ///
    /// Storage errors from the engine.
    pub async fn set_email_verified(&self, id: &str, verified: bool) -> Result<()> {
        let mut fields = Map::new();
        fields.insert("email_verified".to_string(), Value::from(verified));
        self.patch(id, fields).await
    }
}
