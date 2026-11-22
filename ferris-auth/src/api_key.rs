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
//! verify a presented secret. `secret_hash` is a `Password` field, so the engine
//! hashes the plaintext on write and only System Manager reads the hash back.
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
use uuid::Uuid;

use crate::store_support::system_context;

/// The DocType the manager persists.
const DOCTYPE: &str = "API Key";

/// An issued API key (the secret itself is never stored).
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl ApiKey {
    /// Whether the key is enabled and not expired at `now`.
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        self.enabled && self.expires_at.map_or(true, |expires| expires > now)
    }
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

/// `ApiKey` → engine document. `secret` is what lands in `secret_hash`: the
/// plaintext on create (the engine hashes it) or an already-stored hash.
fn to_document(record: &ApiKey, secret: &str) -> Value {
    let mut map = Map::new();
    map.insert("name".to_string(), Value::from(record.id.as_str()));
    map.insert("user".to_string(), Value::from(record.user.as_str()));
    map.insert("key".to_string(), Value::from(record.key.as_str()));
    map.insert("secret_hash".to_string(), Value::from(secret));
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
}

impl ApiKeyManager {
    pub fn new() -> Self {
        Self {
            engine: None,
            memory: RwLock::new(Vec::new()),
        }
    }

    pub fn with_db(db: Db) -> Self {
        Self {
            engine: Some(DocumentEngine::new(db)),
            memory: RwLock::new(Vec::new()),
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
        // `secret_hash` is a `Password` field: the engine hashes it through the
        // process-wide hook, which `ferris-auth` installs. Make sure it is in
        // place even when the caller has not run the full boot sequence.
        ferris_core::password::set_password_hasher(crate::password::hash_password_default);
        ferris_schema::register_doctype(crate::doctypes::api_key_meta());
        engine.ensure_table(DOCTYPE).await.map(|_| ())
    }

    /// Issue a new key. The plaintext secret is returned **once**.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Invalid`] without a user, a hashing error
    /// from the engine without a database, or a storage error with one.
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
            secret_hash: String::new(),
            enabled: true,
            description,
            created_at: Utc::now(),
            last_used_at: None,
            expires_at,
        };

        let Some(engine) = &self.engine else {
            let mut stored = record.clone();
            stored.secret_hash = crate::password::hash_password_default(&secret)?;
            self.memory.write().push(stored.clone());
            return Ok((stored, secret));
        };

        // The engine hashes the plaintext `Password` field on write; the record
        // it returns carries the stored hash.
        let meta = crate::doctypes::api_key_meta();
        let ctx = system_context(&meta);
        let created = engine.create(&ctx, to_document(&record, &secret)).await?;
        Ok((from_document(&created)?, secret))
    }

    /// Verify a `key`/`secret` pair.
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
    /// Hashing or storage errors.
    pub async fn verify(&self, key: &str, secret: &str) -> Result<Option<ApiKey>> {
        let Some(mut record) = self.find_by_key(key).await? else {
            return Ok(None);
        };
        if !record.is_valid_at(Utc::now()) {
            return Ok(None);
        }
        let valid = crate::password::verify_password_default(secret, &record.secret_hash)?;
        if !valid {
            return Ok(None);
        }

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
mod tests {
    use super::*;

    async fn db() -> Db {
        ferris_db::memory_db().await.unwrap()
    }

    #[tokio::test]
    async fn test_api_key_lifecycle() {
        let manager = ApiKeyManager::with_db(db().await);
        manager.init().await.unwrap();

        let (record, secret) = manager
            .create("user@example.com", Some("integration".to_string()), None)
            .await
            .unwrap();
        assert!(!secret.is_empty());
        assert!(!record.key.is_empty());

        // Correct credentials, both directly and via the Frappe `token` scheme.
        assert!(
            manager
                .verify(&record.key, &secret)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            manager
                .authenticate(&format!("{}:{}", record.key, secret))
                .await
                .unwrap()
                .is_some()
        );

        // Wrong secret / unknown key.
        assert!(manager.verify(&record.key, "nope").await.unwrap().is_none());
        assert!(manager.verify("unknown", &secret).await.unwrap().is_none());

        // Listing hides nothing but the secret hash is skipped on serialize.
        let listed = manager.list("user@example.com").await.unwrap();
        assert_eq!(listed.len(), 1);
        let json = serde_json::to_string(&listed[0]).unwrap();
        assert!(!json.contains(&secret));

        // Revoked keys no longer verify.
        manager.revoke(&record.id).await.unwrap();
        assert!(
            manager
                .verify(&record.key, &secret)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// A use inside the window must not write: the hot path stays a read, and
    /// the timestamp returned is the stored one (not a fresher value the row
    /// does not have).
    #[tokio::test]
    async fn last_used_at_is_not_rewritten_on_every_request() {
        let manager = ApiKeyManager::new();
        let (record, secret) = manager
            .create("user@example.com", None, None)
            .await
            .unwrap();

        let first = manager.verify(&record.key, &secret).await.unwrap().unwrap();
        let first_used = first.last_used_at.expect("the first use stamps the key");
        let second = manager.verify(&record.key, &secret).await.unwrap().unwrap();
        assert_eq!(
            second.last_used_at,
            Some(first_used),
            "a second use inside the resolution window must not move the stamp"
        );
    }

    /// Once the stamp is stale the next successful use refreshes it.
    #[tokio::test]
    async fn a_stale_last_used_at_is_refreshed() {
        let manager = ApiKeyManager::new();
        let (record, secret) = manager
            .create("user@example.com", None, None)
            .await
            .unwrap();
        let stale = Utc::now() - chrono::Duration::seconds(LAST_USED_RESOLUTION_SECS + 1);
        manager
            .memory
            .write()
            .iter_mut()
            .find(|key| key.id == record.id)
            .expect("the created key")
            .last_used_at = Some(stale);

        let verified = manager.verify(&record.key, &secret).await.unwrap().unwrap();
        let refreshed = verified.last_used_at.expect("refreshed");
        assert!(
            refreshed > stale,
            "a stale stamp must be refreshed: {stale} -> {refreshed}"
        );
        let stored = manager
            .list("user@example.com")
            .await
            .unwrap()
            .into_iter()
            .find(|key| key.id == record.id)
            .expect("stored key");
        assert_eq!(stored.last_used_at, Some(refreshed));
    }

    #[tokio::test]
    async fn test_expired_key_is_invalid() {
        let manager = ApiKeyManager::new();
        let (record, secret) = manager
            .create(
                "user@example.com",
                None,
                Some(Utc::now() - chrono::Duration::seconds(1)),
            )
            .await
            .unwrap();
        assert!(
            manager
                .verify(&record.key, &secret)
                .await
                .unwrap()
                .is_none()
        );
    }
}
