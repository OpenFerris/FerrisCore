//! MFA surface of the [`UserStore`](super::UserStore).
//!
//! The store is the source of truth for the login gate (`mfa_enabled`), the
//! TOTP secret awaiting activation, and the single-use backup code **hashes**.
//! Kept apart from the login/identity surface in `super` so neither fragment
//! grows past the size limit.

use ferris_core::{Error, Map, Result, SerializationError, Value};

use super::UserStore;

impl UserStore {
    /// The user's TOTP secret, whether the account is already enabled or the
    /// secret is pending activation (the enable flow persists it first and only
    /// flips the flag once the first code verifies).
    ///
    /// # Errors
    /// Propagates storage read failures.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn mfa_secret(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .load(id)
            .await?
            .and_then(|d| {
                d.get("mfa_secret")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty()))
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
    /// # Errors
    /// Propagates serialization and storage write failures.
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
