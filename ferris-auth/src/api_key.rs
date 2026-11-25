//! API keys.
//!
//! Mirrors Frappe's `API Key` / `API Secret`: a public `key` identifies the
//! credential and a secret is shown once and stored only as a hash. Clients
//! authenticate with `Authorization: token <key>:<secret>` (the same scheme as
//! Frappe).
//!
//! Frappe keeps these as the `api_key`/`api_secret` fields of `User`; Ferris
//! issues many keys per user, so they are persisted as
//! [`API Key`](crate::doctypes::api_key_meta) documents and the document engine
//! is the single source of truth. The store acts as `System Manager` (see
//! `store_support.rs`): it is a trusted framework caller running before
//! any user is authenticated, and it has to read the stored secret hash to
//! verify a presented secret.
//!
//! The secret is stored as `sha256:<64 lowercase hex>` over the presented
//! string, never in clear. A 32-byte random token needs no password KDF — the
//! entropy is already there, and Argon2 on every API request was measurable
//! latency (AUTH12). `secret_hash` is a plain `Data` field at `permlevel` 1:
//! the manager hashes and writes it itself, so the engine must **not** run it
//! through the process-wide password hasher. Rows written before this format
//! carry a `$argon2...` PHC hash; [`ApiKeyManager::verify`] still accepts them
//! and rewrites the row with the digest on the first successful use.
//!
//! Without a database the manager keeps records in memory, which is what the
//! unit tests and a pool-less boot use.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ferris_core::{AuthError, DocumentError, Error, Map, Result, ValidationError, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery};
use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

use crate::crypto::{constant_time_eq, sha256_hex};
use crate::store_support::system_context;

/// The DocType the manager persists (`API Key`).
const DOCTYPE: &str = "API Key";

/// Prefix of the stored digest: `sha256:<64 lowercase hex characters>`.
const DIGEST_PREFIX: &str = "sha256:";

/// Prefix of a legacy Argon2 PHC hash, written when `secret_hash` was a
/// `Password` field.
const LEGACY_ARGON2_PREFIX: &str = "$argon2";

/// An issued API key (the secret itself is never stored).
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub user: String,
    /// Public identifier sent by the client.
    pub key: String,
    #[serde(skip_serializing)]
    pub secret_hash: String,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Redacts the stored digest. `ApiKey` derives `Serialize` (with
/// `secret_hash` skipped) precisely so it can travel to a caller; letting the
/// derived `Debug` print the hash would put the same credential in every log
/// line and error message.
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKey")
            .field("id", &self.id)
            .field("user", &self.user)
            .field("key", &self.key)
            .field("secret_hash", &"<redacted>")
            .field("enabled", &self.enabled)
            .field("description", &self.description)
            .field("created_at", &self.created_at)
            .field("last_used_at", &self.last_used_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl ApiKey {
    /// Whether the key is enabled and not expired at `now`.
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        self.enabled && self.expires_at.map_or(true, |expires| expires > now)
    }
}

/// The stored form of `secret`.
fn hash_secret_for_storage(secret: &str) -> String {
    format!("{DIGEST_PREFIX}{}", sha256_hex(secret.as_bytes()))
}

/// The outcome of comparing a presented secret with the stored value.
enum SecretCheck {
    /// The secret matched. `upgrade` is set when the row still holds a legacy
    /// Argon2 hash and should be rewritten with the digest.
    Match { upgrade: bool },
    /// The secret did not match, or the stored value is malformed. Both fail
    /// closed; the caller must not distinguish them.
    NoMatch,
}

/// Compare `presented` with a stored `secret_hash`.
///
/// Fails closed: an unparseable or unknown stored value is a mismatch, never an
/// error and never a panic. The digest comparison is constant time.
fn check_secret(presented: &str, stored: &str) -> SecretCheck {
    if let Some(stored_hex) = stored.strip_prefix(DIGEST_PREFIX) {
        let well_formed =
            stored_hex.len() == 64 && stored_hex.bytes().all(|b| b.is_ascii_hexdigit());
        if !well_formed {
            return SecretCheck::NoMatch;
        }
        let computed = sha256_hex(presented.as_bytes());
        return if constant_time_eq(computed.as_bytes(), stored_hex.as_bytes()) {
            SecretCheck::Match { upgrade: false }
        } else {
            SecretCheck::NoMatch
        };
    }

    // Rows created before the SHA-256 format hold an Argon2 PHC hash. A parse
    // or hashing error here is a malformed stored value: refuse, do not error.
    if stored.starts_with(LEGACY_ARGON2_PREFIX) {
        return match crate::password::verify_password_default(presented, stored) {
            Ok(true) => SecretCheck::Match { upgrade: true },
            Ok(false) | Err(_) => SecretCheck::NoMatch,
        };
    }

    SecretCheck::NoMatch
}

fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(&buffer)
}

fn opt_datetime(value: Option<DateTime<Utc>>) -> Value {
    value.map_or(Value::Null, |dt| Value::from(dt.to_rfc3339()))
}

fn parse_datetime(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let text = value.and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// `ApiKey` → engine document.
///
/// `secret_hash` already holds the stored digest; the manager writes it and
/// the `Data` fieldtype leaves it untouched on the way to the column.
fn to_document(record: &ApiKey) -> Value {
    let mut map = Map::new();
    map.insert("name".to_string(), Value::from(record.id.as_str()));
    map.insert("user".to_string(), Value::from(record.user.as_str()));
    map.insert("key".to_string(), Value::from(record.key.as_str()));
    map.insert(
        "secret_hash".to_string(),
        Value::from(record.secret_hash.as_str()),
    );
    map.insert("enabled".to_string(), Value::from(record.enabled));
    map.insert(
        "description".to_string(),
        record.description.clone().map_or(Value::Null, Value::from),
    );
    map.insert(
        "created_at".to_string(),
        Value::from(record.created_at.to_rfc3339()),
    );
    map.insert(
        "last_used_at".to_string(),
        opt_datetime(record.last_used_at),
    );
    map.insert("expires_at".to_string(), opt_datetime(record.expires_at));
    Value::Object(map)
}

/// Engine document → `ApiKey`.
///
/// # Errors
///
/// [`AuthError::TokenInvalid`](ferris_core::AuthError::TokenInvalid) when a
/// stored timestamp column is not RFC 3339. A corrupted `created_at` must
/// surface, not be masked with "now" (AUTH12).
fn from_document(document: &Value) -> Result<ApiKey> {
    let text = |name: &str| {
        document
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let created_at = parse_datetime(document.get("created_at")).ok_or_else(|| {
        Error::Auth(AuthError::TokenInvalid(
            "API key has an invalid created_at".to_string(),
        ))
    })?;
    Ok(ApiKey {
        id: text("name"),
        user: text("user"),
        key: text("key"),
        secret_hash: text("secret_hash"),
        enabled: document
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        description: document
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        created_at,
        last_used_at: parse_datetime(document.get("last_used_at")),
        expires_at: parse_datetime(document.get("expires_at")),
    })
}

/// How stale `last_used_at` may get before a successful use rewrites it.
///
/// The column is an audit convenience, never an authentication input, and
/// `verify` sits on every API-key request: rewriting it each time turns a
/// read-mostly check into a write. Five minutes keeps "was this key used
/// recently?" meaningful while bounding the writes.
pub const LAST_USED_RESOLUTION_SECS: i64 = 5 * 60;

/// Manages API key issuance and verification.
pub struct ApiKeyManager {
    engine: Option<DocumentEngine>,
    memory: RwLock<Vec<ApiKey>>,
    /// Per-key failed-verification budget. An attacker who knows a public key
    /// can otherwise grind the secret indefinitely; the budget lives on the
    /// manager because the key id is only known after the lookup.
    failures: crate::middleware::RateLimiter,
}

impl ApiKeyManager {
    pub fn new() -> Self {
        Self::with_verification_limit(10, 15 * 60)
    }

    pub fn with_db(db: Db) -> Self {
        Self::with_db_and_limit(db, 10, 15 * 60)
    }

    /// Build an in-memory manager with an explicit failure budget.
    #[must_use]
    pub fn with_verification_limit(max_attempts: u32, window_seconds: u64) -> Self {
        Self {
            engine: None,
            memory: RwLock::new(Vec::new()),
            failures: crate::middleware::RateLimiter::new(max_attempts, window_seconds),
        }
    }

    /// Build a database-backed manager with an explicit failure budget.
    #[must_use]
    pub fn with_db_and_limit(db: Db, max_attempts: u32, window_seconds: u64) -> Self {
        Self {
            engine: Some(DocumentEngine::new(db)),
            memory: RwLock::new(Vec::new()),
            failures: crate::middleware::RateLimiter::new(max_attempts, window_seconds),
        }
    }

    /// Ensure the `API Key` table exists (no-op without a database).
    ///
    /// # Errors
    ///
    /// Database errors from creating the table or its indexes.
    pub async fn init(&self) -> Result<()> {
        let Some(engine) = &self.engine else {
            return Ok(());
        };
        // `secret_hash` is written by the manager as a plain `Data` value, so
        // no password hasher has to be installed for this DocType.
        ferris_schema::register_doctype(crate::doctypes::api_key_meta());
        engine.ensure_table(DOCTYPE).await.map(|_| ())
    }

    /// Issue a new key. The plaintext secret is returned **once**.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Invalid`] without a user, or a storage error.
    pub async fn create(
        &self,
        user: &str,
        description: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(ApiKey, String)> {
        if user.trim().is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "API key requires a user".to_string(),
            )));
        }
        let secret = random_token(32);
        let record = ApiKey {
            id: Uuid::new_v4().to_string(),
            user: user.to_string(),
            key: random_token(16),
            secret_hash: hash_secret_for_storage(&secret),
            enabled: true,
            description,
            created_at: Utc::now(),
            last_used_at: None,
            expires_at,
        };

        let Some(engine) = &self.engine else {
            self.memory.write().push(record.clone());
            return Ok((record, secret));
        };

        let meta = crate::doctypes::api_key_meta();
        let ctx = system_context(&meta);
        let created = engine.create(&ctx, to_document(&record)).await?;
        Ok((from_document(&created)?, secret))
    }

    /// Verify a `key`/`secret` pair.
    ///
    /// The stored digest is compared in constant time. A legacy `$argon2...`
    /// hash is accepted once and the row is rewritten with the digest, so the
    /// expensive path disappears as each key is used. A malformed or unknown
    /// stored value is a mismatch (fail closed), not an error.
    ///
    /// `last_used_at` is refreshed on success, but **at most once per**
    /// [`LAST_USED_RESOLUTION_SECS`]: an API key is checked on every request, and
    /// writing the audit column each time would turn a read-mostly credential
    /// check into a write on the hottest path there is. A use inside the window
    /// returns the stored timestamp unchanged, so the value never claims a
    /// precision the column does not have.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn verify(&self, key: &str, secret: &str) -> Result<Option<ApiKey>> {
        let Some(mut record) = self.find_by_key(key).await? else {
            return Ok(None);
        };
        if !record.is_valid_at(Utc::now()) {
            return Ok(None);
        }
        if self.failures.is_blocked(&record.id) {
            return Ok(None);
        }
        match check_secret(secret, &record.secret_hash) {
            SecretCheck::Match { upgrade: false } => {}
            SecretCheck::Match { upgrade: true } => {
                let digest = hash_secret_for_storage(secret);
                self.store_secret_hash(&record.id, &digest).await?;
                record.secret_hash = digest;
            }
            SecretCheck::NoMatch => {
                self.failures.record_failure(&record.id);
                return Ok(None);
            }
        }
        self.failures.record_success(&record.id);

        let now = Utc::now();
        let stale = record
            .last_used_at
            .is_none_or(|last| (now - last).num_seconds() >= LAST_USED_RESOLUTION_SECS);
        if !stale {
            // Still fresh: the stored value is what the caller sees, and no
            // write happens.
            return Ok(Some(record));
        }
        record.last_used_at = Some(now);
        if let Some(engine) = &self.engine {
            let meta = crate::doctypes::api_key_meta();
            let ctx = system_context(&meta);
            let mut body = Map::new();
            body.insert(
                "last_used_at".to_string(),
                opt_datetime(record.last_used_at),
            );
            engine.update(&ctx, &record.id, Value::Object(body)).await?;
        } else {
            let mut memory = self.memory.write();
            if let Some(existing) = memory.iter_mut().find(|k| k.id == record.id) {
                existing.last_used_at = record.last_used_at;
            }
        }
        Ok(Some(record))
    }

    /// Replace the stored digest of `id` (the legacy-upgrade write).
    async fn store_secret_hash(&self, id: &str, digest: &str) -> Result<()> {
        if let Some(engine) = &self.engine {
            let meta = crate::doctypes::api_key_meta();
            let ctx = system_context(&meta);
            let mut body = Map::new();
            body.insert("secret_hash".to_string(), Value::from(digest));
            engine.update(&ctx, id, Value::Object(body)).await?;
        } else if let Some(existing) = self.memory.write().iter_mut().find(|k| k.id == id) {
            existing.secret_hash = digest.to_string();
        }
        Ok(())
    }

    /// Verify a `key:secret` credential string (Frappe `token` scheme).
    ///
    /// # Errors
    ///
    /// Hashing or storage errors.
    pub async fn authenticate(&self, credentials: &str) -> Result<Option<ApiKey>> {
        let Some((key, secret)) = credentials.split_once(':') else {
            return Ok(None);
        };
        self.verify(key.trim(), secret.trim()).await
    }

    /// Disable a key.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::NotFound`] for an unknown key, or a storage error.
    pub async fn revoke(&self, id: &str) -> Result<()> {
        if let Some(engine) = &self.engine {
            let meta = crate::doctypes::api_key_meta();
            let ctx = system_context(&meta);
            if let Err(e) = engine.get(&ctx, id).await {
                return match e {
                    Error::Document(DocumentError::NotFound { .. }) => {
                        Err(Error::Auth(AuthError::NotFound(format!("API key {id}"))))
                    }
                    other => Err(other),
                };
            }
            let mut body = Map::new();
            body.insert("enabled".to_string(), Value::from(false));
            engine.update(&ctx, id, Value::Object(body)).await?;
        } else {
            let mut memory = self.memory.write();
            let record = memory
                .iter_mut()
                .find(|k| k.id == id)
                .ok_or_else(|| Error::Auth(AuthError::NotFound(format!("API key {id}"))))?;
            record.enabled = false;
        }
        Ok(())
    }

    /// List a user's keys.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn list(&self, user: &str) -> Result<Vec<ApiKey>> {
        let Some(engine) = &self.engine else {
            return Ok(self
                .memory
                .read()
                .iter()
                .filter(|k| k.user == user)
                .cloned()
                .collect());
        };
        let meta = crate::doctypes::api_key_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            filters: HashMap::from([("user".to_string(), Value::from(user))]),
            limit: Some(u32::MAX),
            ..ListQuery::default()
        };
        let (documents, _) = engine.list(&ctx, &query).await?;
        documents.iter().map(from_document).collect()
    }

    async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>> {
        let Some(engine) = &self.engine else {
            return Ok(self.memory.read().iter().find(|k| k.key == key).cloned());
        };
        let meta = crate::doctypes::api_key_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            filters: HashMap::from([("key".to_string(), Value::from(key))]),
            limit: Some(1),
            ..ListQuery::default()
        };
        let (documents, _) = engine.list(&ctx, &query).await?;
        documents.first().map(from_document).transpose()
    }
}

impl Default for ApiKeyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
