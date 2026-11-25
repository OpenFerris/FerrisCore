//! MFA surface of the [`UserStore`](super::UserStore).
//!
//! The store is the source of truth for the login gate (`mfa_enabled`), the
//! TOTP secret awaiting activation, and the single-use backup code **hashes**.
//! Kept apart from the login/identity surface in `super` so neither fragment
//! grows past the size limit.

use ferris_core::secret::is_secret_envelope;
use ferris_core::{DocumentError, Error, Map, Result, SecretError, SerializationError, Value};
use ferris_db::Stmt;

use super::UserStore;

impl UserStore {
    /// The user's TOTP secret, whether the account is already enabled or the
    /// secret is pending activation (the enable flow persists it first and only
    /// flips the flag once the first code verifies).
    ///
    /// `mfa_secret` is a `Secret` field: the engine seals it on write, ordinary
    /// reads only ever see the mask, and the plaintext comes back exclusively
    /// through [`ferris_document::DocumentEngine::read_secret`] with
    /// `reveal_secrets()`.
    ///
    /// A row written before the field was declared `Secret` still holds the
    /// seed in the clear. That legacy value is **returned unchanged** so an
    /// existing account keeps working; the next [`Self::set_mfa`] seals it.
    /// An encrypted value the process cannot open (no keyring, retired key) is
    /// an error, never the envelope handed back as if it were the seed.
    ///
    /// # Errors
    /// Propagates storage read failures and an envelope this process cannot
    /// decrypt.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn mfa_secret(&self, id: &str) -> Result<Option<String>> {
        let meta = crate::doctypes::user_meta();
        let ctx = crate::store_support::system_context(&meta).reveal_secrets();
        match self.engine.read_secret(&ctx, id, "mfa_secret").await {
            Ok(secret) => Ok(secret.map(ferris_core::secret::SecretValue::into_inner)),
            Err(Error::Document(DocumentError::NotFound { .. })) => Ok(None),
            // Not a v1 envelope (legacy plaintext) or no keyring at all: the
            // raw column decides which of the two this is.
            Err(
                error @ Error::Secret(SecretError::NotRecoverable { .. } | SecretError::KeyMissing),
            ) => match self.stored_mfa_secret(id).await? {
                Some(raw) if is_secret_envelope(&raw) => Err(error),
                Some(raw) => Ok(Some(raw)),
                None => Ok(None),
            },
            Err(error) => Err(error),
        }
    }

    /// The `mfa_secret` column exactly as stored, without the read mask.
    ///
    /// Only the legacy-plaintext branch of [`Self::mfa_secret`] uses this: the
    /// engine's document read replaces a `Secret` value with the mask, so the
    /// only way to tell an old clear-text seed from a sealed one is the column.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    async fn stored_mfa_secret(&self, id: &str) -> Result<Option<String>> {
        let table = self.user_table()?;
        let column = self.user_column("mfa_secret")?;
        let name = self.user_column("name")?;
        let row = self
            .db()
            .fetch_optional(&Stmt::new(
                format!("SELECT {column} FROM {table} WHERE {name} = ?"),
                vec![Value::from(id)],
            ))
            .await?;
        Ok(row
            .as_ref()
            .and_then(|row| row.opt_str("mfa_secret"))
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty()))
    }

    /// Spend TOTP time step `step` for `id`.
    ///
    /// Answers `true` only when this statement moved the account's
    /// `mfa_last_step` forward, which makes a code single-use **across
    /// processes and replicas**: tracking the matched step (not the current
    /// one) keeps a fresh code for the next step working, while the code just
    /// used is refused for as long as the TOTP window would accept it.
    ///
    /// One atomic conditional `UPDATE` with no read before it, so two
    /// concurrent logins carrying the same code cannot both win.
    ///
    /// # Errors
    ///
    /// Storage errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn claim_totp_step(&self, id: &str, step: u64) -> Result<bool> {
        let table = self.user_table()?;
        let column = self.user_column("mfa_last_step")?;
        let name = self.user_column("name")?;
        let step = i64::try_from(step).unwrap_or(i64::MAX);
        let sql = format!(
            "UPDATE {table} SET {column} = ? \
             WHERE {name} = ? AND ({column} IS NULL OR {column} < ?)"
        );
        let affected = self
            .db()
            .execute(&Stmt::new(
                sql,
                vec![Value::from(step), Value::from(id), Value::from(step)],
            ))
            .await?;
        Ok(affected.rows() > 0)
    }

    /// Whether the account has MFA enabled (source of truth for the login gate).
    ///
    /// # Errors
    /// Propagates storage read failures.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn mfa_enabled(&self, id: &str) -> Result<bool> {
        Ok(self
            .load(id)
            .await?
            .and_then(|d| d.get("mfa_enabled").and_then(Value::as_bool))
            .unwrap_or(false))
    }

    /// Persist the MFA state for a user in one update: the enabled flag, the
    /// secret and the single-use backup code **hashes**.
    ///
    /// `secret` is the **plaintext** Base32 seed: the engine owns sealing
    /// (`Secret` fieldtype), so a stored envelope would be rejected as
    /// double-wrapped. A deployment with MFA must therefore have a `[secrets]`
    /// keyring installed, or the write fails closed with
    /// [`SecretError::KeyMissing`].
    ///
    /// # Errors
    /// Propagates serialization, encryption (missing keyring) and storage write
    /// failures.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn set_mfa(
        &self,
        id: &str,
        enabled: bool,
        secret: Option<&str>,
        backup_code_hashes: &[String],
    ) -> Result<()> {
        let codes_json = serde_json::to_string(backup_code_hashes)
            .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
        let mut fields = Map::new();
        fields.insert("mfa_enabled".to_string(), Value::from(enabled));
        fields.insert(
            "mfa_secret".to_string(),
            secret.map_or(Value::Null, Value::from),
        );
        fields.insert(
            "mfa_backup_codes".to_string(),
            Value::from(codes_json.as_str()),
        );
        self.patch(id, fields).await
    }

    /// Flip the enabled flag (used after the first code verifies, and by the
    /// desk to force-disable an account).
    ///
    /// # Errors
    /// Propagates storage write failures.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn set_mfa_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut fields = Map::new();
        fields.insert("mfa_enabled".to_string(), Value::from(enabled));
        self.patch(id, fields).await
    }

    /// Burn one backup code. The stored list holds Argon2 **hashes**; the code
    /// is verified against each in turn and removed from the list on a match, so
    /// a code can be used exactly once and a leaked store leaks no usable codes.
    ///
    /// Verification (not equality) is the point: backup codes are only ~41 bits,
    /// so an unsalted fast digest was trivially brute-forced offline (AUTH8).
    /// Argon2 makes each guess cost the same as a password guess.
    ///
    /// # Errors
    /// Propagates serialization, hashing and storage failures.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn consume_mfa_backup_code(&self, id: &str, code: &str) -> Result<bool> {
        let Some(document) = self.load(id).await? else {
            return Ok(false);
        };
        let Some(codes_json) = document.get("mfa_backup_codes").and_then(Value::as_str) else {
            return Ok(false);
        };
        let mut hashes: Vec<String> = match serde_json::from_str(codes_json) {
            Ok(hashes) => hashes,
            Err(_) => return Ok(false),
        };
        let Some(pos) = hashes
            .iter()
            .position(|h| crate::verify_password(code, h).unwrap_or(false))
        else {
            return Ok(false);
        };
        hashes.remove(pos);
        let remaining = serde_json::to_string(&hashes)
            .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
        let mut fields = Map::new();
        fields.insert(
            "mfa_backup_codes".to_string(),
            Value::from(remaining.as_str()),
        );
        self.patch(id, fields).await?;
        Ok(true)
    }

    /// Argon2 hash of a backup code — the form stored on the user.
    ///
    /// Uses [`crate::password::hash_secret_default`], not `hash_password_*`:
    /// a backup code is a server-generated secret and must not be run through
    /// the human password policy (it would reject e.g. an 8-character
    /// alphanumeric code for lacking a symbol).
    ///
    /// # Errors
    ///
    /// A hashing failure from the crate's Argon2 configuration.
    pub fn hash_backup_code(code: &str) -> Result<String> {
        crate::password::hash_secret_default(code)
    }
}
