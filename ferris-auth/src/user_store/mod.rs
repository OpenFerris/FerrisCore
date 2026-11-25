//! Engine-backed user store used by the session login flow.
//!
//! Accounts are [`User`](crate::doctypes::user_meta) documents: the document
//! engine is the single source of truth for the `tabUser` table and the store
//! is a thin typed facade over it, exposing the minimal surface the Desk needs
//! (login, session identity, dev bootstrap). The store acts as `System Manager`
//! (`store_support.rs`) because login runs before any user is authenticated and
//! because the `password` hash and the `permlevel` 1 secrets it has to read are
//! administrator-only elsewhere.
//!
//! Passwords are Argon2-hashed by the engine: `create` hands it the plaintext
//! and never touches the hash, and `password` reads back as a mask to every
//! caller but the store.
//!
//! Split into fragments: this `mod.rs` owns the row type, the login/identity
//! surface and the document↔row mapping; [`mfa`] owns the MFA and backup-code
//! surface; [`login_support`] owns the credential material the `axum-login`
//! backend reads; [`tests`] holds the suite.

mod login_support;
mod mfa;

pub use login_support::LoginRecord;

#[cfg(test)]
mod tests;

use ferris_core::{DocumentError, Error, Map, Result, SerializationError, ValidationError, Value};
use ferris_db::{Db, Stmt};
use ferris_document::{DocumentEngine, ListQuery};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::middleware::UserValidator;
use crate::store_support::system_context;
use crate::{User, UserType};

/// The DocType the store persists.
const DOCTYPE: &str = "User";

/// A user as persisted in the `User` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUser {
    pub id: String,
    pub email: String,
    pub username: Option<String>,
    pub full_name: String,
    pub user_type: String,
    pub enabled: bool,
    /// Whether the account's email address has been proven (`email_verified`).
    pub email_verified: bool,
    pub mfa_enabled: bool,
    pub roles: Vec<String>,
    pub last_login: Option<String>,
    pub login_count: u32,
    /// The account's chosen locale; `None` follows `Accept-Language`.
    pub language: Option<String>,
}

impl StoredUser {
    /// Portable identity the Desk renders.
    pub fn public_json(&self) -> Value {
        Value::from(serde_json::json!({
            "id": self.id,
            "email": self.email,
            "username": self.username,
            "full_name": self.full_name,
            "roles": self.roles,
            "user_type": self.user_type,
            "language": self.language,
        }))
    }

    /// Convert into the auth [`User`] used across the framework.
    pub fn to_user(&self) -> User {
        let user_type = match self.user_type.as_str() {
            "WebsiteUser" => UserType::WebsiteUser,
            "Guest" => UserType::Guest,
            _ => UserType::SystemUser,
        };
        User {
            id: self.id.clone(),
            email: self.email.clone(),
            username: self.username.clone(),
            full_name: self.full_name.clone(),
            user_type,
            enabled: self.enabled,
            roles: self.roles.clone(),
            language: self.language.clone(),
            ..User::guest() // guest() seeds defaults for the remaining fields
        }
    }
}

/// Source-of-truth validator used by the auth middleware: when configured on
/// [`AuthState`](crate::AuthState), the resolved user (roles included) is taken
/// from the users table instead of trusting claims/session payloads, so the
/// metadata engine authorises requests with real roles.
#[async_trait::async_trait]
impl UserValidator for UserStore {
    async fn validate(&self, user_id: &str) -> Option<User> {
        self.find_by_id(user_id)
            .await
            .ok()
            .flatten()
            .map(|u| u.to_user())
    }
}

/// Document → typed row.
fn document_to_stored(document: &Value) -> Result<StoredUser> {
    let text = |name: &str| {
        document
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let roles: Vec<String> = serde_json::from_str(&text("roles")).unwrap_or_default();
    Ok(StoredUser {
        id: text("name"),
        email: text("email"),
        username: document
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string),
        full_name: text("full_name"),
        user_type: text("user_type"),
        enabled: document
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        email_verified: document
            .get("email_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        mfa_enabled: document
            .get("mfa_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        roles,
        last_login: document
            .get("last_login")
            .and_then(Value::as_str)
            .map(str::to_string),
        login_count: document
            .get("login_count")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .clamp(0, i64::from(u32::MAX)) as u32,
        language: document
            .get("language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

/// The fixed AES-256 key the `Secret`-field tests seal with.
///
/// `User.mfa_secret` is a `Secret` field, so a test that writes one needs the
/// AEAD keyring installed. `install_keyring` treats a re-install of the same
/// material as a no-op and `OnceLock` cannot be reset, so this one key serves
/// the whole test binary.
#[cfg(test)]
pub(crate) fn install_test_keyring() {
    const TEST_SECRET_KEY: &str =
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let key = ferris_core::secret::SecretKey::parse(TEST_SECRET_KEY)
        .expect("the test key is 32 bytes of hex");
    let _ = ferris_core::secret::install_keyring(ferris_core::secret::SecretKeyring::new(
        key,
        Vec::new(),
    ));
}

/// Engine-backed user repository.
#[derive(Debug, Clone)]
pub struct UserStore {
    db: Db,
    engine: DocumentEngine,
}

impl UserStore {
    pub fn new(db: Db) -> Self {
        Self {
            engine: DocumentEngine::new(db.clone()),
            db,
        }
    }

    /// The backing database, for the desk notification helpers.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Create the `User` table (no-op when it already exists) and make sure the
    /// `Password` hasher is installed.
    ///
    /// `refresh_table`, not `ensure_table`: the engine caches the tables it has
    /// ensured per database, so a table created by an earlier release would keep
    /// its old shape and never gain a column added to the declaration since
    /// (`mfa_last_step`). Refreshing drops the cache entry and re-runs the
    /// idempotent `ALTER TABLE ... ADD COLUMN` path.
    ///
    /// # Errors
    ///
    /// Database errors from creating the table or its indexes.
    pub async fn init(&self) -> Result<()> {
        ferris_core::password::set_password_hasher(crate::password::hash_password_default);
        // Register the meta first so `ensure_table` materializes the declared
        // `email`/`username` indexes, exactly as the boot-time registry does.
        ferris_schema::register_doctype(crate::doctypes::user_meta());
        self.engine.refresh_table(DOCTYPE).await.map(|_| ())
    }

    /// The dialect-quoted `tabUser` table name.
    ///
    /// The store's hand-written statements (the atomic TOTP step claim and the
    /// lockout counter) cannot go through the engine, so they derive the table
    /// the same way the engine does and quote it for the active backend.
    pub(crate) fn user_table(&self) -> Result<String> {
        let table = ferris_document::table_name(DOCTYPE)?;
        Ok(ferris_db::sql::Ident::new(&table)?.quote(self.db.dialect()))
    }

    /// One dialect-quoted `User` column name.
    pub(crate) fn user_column(&self, name: &str) -> Result<String> {
        Ok(ferris_db::sql::Ident::new(name)?.quote(self.db.dialect()))
    }

    /// The engine document for a login (email or username), or `None`.
    async fn document_by_login(&self, login: &str) -> Result<Option<Value>> {
        let login = login.trim();
        for field in ["email", "username"] {
            if let Some(document) = self.first_by(field, login).await? {
                return Ok(Some(document));
            }
        }
        Ok(None)
    }

    /// The first document whose `field` equals `value`.
    ///
    /// `pub(crate)` for the recovery flows, which resolve a single-use token by
    /// its digest column.
    pub(crate) async fn first_by(&self, field: &str, value: &str) -> Result<Option<Value>> {
        let meta = crate::doctypes::user_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            filters: HashMap::from([(field.to_string(), Value::from(value))]),
            limit: Some(1),
            ..ListQuery::default()
        };
        let (documents, _) = self.engine.list(&ctx, &query).await?;
        Ok(documents.into_iter().next())
    }

    /// Resolve a user by email or username.
    ///
    /// # Errors
    ///
    /// Storage or document-decoding errors.
    pub async fn find_by_login(&self, login: &str) -> Result<Option<StoredUser>> {
        self.document_by_login(login)
            .await?
            .as_ref()
            .map(document_to_stored)
            .transpose()
    }

    /// Look up a user by id (session bootstrap).
    ///
    /// # Errors
    ///
    /// Storage or document-decoding errors.
    pub async fn find_by_id(&self, id: &str) -> Result<Option<StoredUser>> {
        self.document_by_id(id)
            .await?
            .as_ref()
            .map(document_to_stored)
            .transpose()
    }

    async fn document_by_id(&self, id: &str) -> Result<Option<Value>> {
        let meta = crate::doctypes::user_meta();
        let ctx = system_context(&meta);
        match self.engine.get(&ctx, id).await {
            Ok(document) => Ok(Some(document)),
            Err(Error::Document(DocumentError::NotFound { .. })) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Fetch the stored password hash for a login.
    async fn password_hash(&self, login: &str) -> Result<Option<(String, String)>> {
        let Some(document) = self.document_by_login(login).await? else {
            return Ok(None);
        };
        // A disabled user cannot authenticate.
        if !document
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            return Ok(None);
        }
        // A user with no hash cannot authenticate; treat it as no credential
        // rather than as an error, so the caller answers `None`.
        let Some(hash) = document
            .get("password")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        let id = document
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(Some((id, hash.to_string())))
    }

    /// Persist a user. The engine hashes the `password` field.
    ///
    /// # Errors
    ///
    /// Validation, permission or storage errors from the engine.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        email: &str,
        full_name: &str,
        username: Option<&str>,
        user_type: UserType,
        password: &str,
        roles: &[String],
    ) -> Result<StoredUser> {
        let id = uuid::Uuid::new_v4().to_string();
        let user_type = match user_type {
            UserType::WebsiteUser => "WebsiteUser",
            UserType::Guest => "Guest",
            UserType::SystemUser => "SystemUser",
        };
        let roles_json = serde_json::to_string(roles)
            .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
        let mut map = Map::new();
        map.insert("name".to_string(), Value::from(id.as_str()));
        map.insert("email".to_string(), Value::from(email));
        map.insert("username".to_string(), Value::from(username));
        map.insert("full_name".to_string(), Value::from(full_name));
        map.insert("user_type".to_string(), Value::from(user_type));
        map.insert("enabled".to_string(), Value::from(true));
        map.insert("mfa_enabled".to_string(), Value::from(false));
        map.insert("roles".to_string(), Value::from(roles_json.as_str()));
        map.insert("login_count".to_string(), Value::from(0_i64));
        // Plaintext: the engine hashes `Password` fields on write.
        map.insert("password".to_string(), Value::from(password));

        let meta = crate::doctypes::user_meta();
        let ctx = system_context(&meta);
        let created = self.engine.create(&ctx, Value::Object(map)).await?;
        document_to_stored(&created)
    }

    /// Seed a default operator. No-op once the table already has a user.
    ///
    /// # Errors
    /// Returns [`ValidationError::Invalid`] when the table already holds users,
    /// and propagates database/hashing failures from [`UserStore::create`].
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn ensure_admin(
        &self,
        email: &str,
        password: &str,
        roles: &[String],
    ) -> Result<StoredUser> {
        self.init().await?;
        if let Some(existing) = self.find_by_login(email).await? {
            return Ok(existing);
        }
        let meta = crate::doctypes::user_meta();
        let ctx = system_context(&meta);
        let (_, total) = self
            .engine
            .list(
                &ctx,
                &ListQuery {
                    limit: Some(1),
                    ..ListQuery::default()
                },
            )
            .await?;
        if total > 0 {
            return Err(Error::Validation(ValidationError::Invalid(
                "users already exist; refusing to auto-seed a default operator".to_string(),
            )));
        }
        self.create(
            email,
            "Administrator",
            Some("admin"),
            UserType::SystemUser,
            password,
            roles,
        )
        .await
    }

    /// Read a user document, or `None` when there is no such user.
    ///
    /// `pub(crate)` so the recovery flows can inspect and clear the `permlevel`
    /// 1 token columns, which [`StoredUser`] deliberately does not carry.
    pub(crate) async fn load(&self, id: &str) -> Result<Option<Value>> {
        self.document_by_id(id).await
    }

    /// Merge `fields` into a user document. A no-op for an unknown user, so the
    /// audit writers never fail a request over a deleted account.
    ///
    /// `pub(crate)` for the recovery flows, which write and clear `permlevel` 1
    /// token columns through the same engine path as every other update.
    pub(crate) async fn patch(&self, id: &str, fields: Map) -> Result<()> {
        if self.load(id).await?.is_none() {
            return Ok(());
        }
        let meta = crate::doctypes::user_meta();
        let ctx = system_context(&meta);
        self.engine.update(&ctx, id, Value::Object(fields)).await?;
        Ok(())
    }

    /// Persist the account's locale. `None` clears it, which makes the account
    /// follow `Accept-Language` again.
    ///
    /// # Errors
    ///
    /// Storage errors from the engine.
    pub async fn set_language(&self, id: &str, language: Option<&str>) -> Result<()> {
        let mut fields = Map::new();
        fields.insert(
            "language".to_string(),
            language.map_or(Value::Null, Value::from),
        );
        self.patch(id, fields).await
    }

    /// The OAuth provider's stable subject bound to `id`, if any.
    ///
    /// `oauth_subject` is `permlevel` 1, so this goes through the store's
    /// `System Manager` context; the empty string reads back as `None`.
    ///
    /// # Errors
    ///
    /// Storage or document-decoding errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn oauth_subject(&self, id: &str) -> Result<Option<String>> {
        Ok(self.load(id).await?.and_then(|document| {
            document
                .get("oauth_subject")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|subject| !subject.is_empty())
                .map(str::to_string)
        }))
    }

    /// Bind `subject` to `id`, refusing to replace a different one.
    ///
    /// Answers `true` when the account already carries this subject or the
    /// claim just succeeded, and `false` when another subject is bound. The
    /// write is one conditional `UPDATE` (`... WHERE oauth_subject IS NULL OR
    /// oauth_subject = ''`), so two concurrent first logins for the same
    /// account cannot both bind: only one statement matches, and the loser
    /// re-reads and compares. Nothing ever overwrites a non-empty subject.
    ///
    /// # Errors
    ///
    /// Storage errors from the claim or the follow-up read.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn bind_oauth_subject(&self, id: &str, subject: &str) -> Result<bool> {
        let subject = subject.trim();
        if subject.is_empty() {
            return Ok(false);
        }
        let table = self.user_table()?;
        let column = self.user_column("oauth_subject")?;
        let name = self.user_column("name")?;
        let affected = self
            .db
            .execute(&Stmt::new(
                format!(
                    "UPDATE {table} SET {column} = ? \
                     WHERE {name} = ? AND ({column} IS NULL OR {column} = '')"
                ),
                vec![Value::from(subject), Value::from(id)],
            ))
            .await?;
        if affected.rows() > 0 {
            return Ok(true);
        }
        // Either the account already carries a subject or it does not exist;
        // comparing the stored value answers both without a write.
        Ok(self.oauth_subject(id).await?.as_deref() == Some(subject))
    }

    /// Replace (or clear, with `None`) the stored OAuth subject.
    ///
    /// The administrative primitive; the login path must use
    /// [`Self::bind_oauth_subject`] so an existing binding is never overwritten.
    ///
    /// # Errors
    ///
    /// Storage errors from the engine.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn set_oauth_subject(&self, id: &str, subject: Option<&str>) -> Result<()> {
        let mut fields = Map::new();
        fields.insert(
            "oauth_subject".to_string(),
            subject.map_or(Value::Null, Value::from),
        );
        self.patch(id, fields).await
    }

    /// Update the last-login timestamp after a successful login.
    ///
    /// `login_count` is computed here and written through the engine, which is a
    /// read-modify-write: two concurrent logins for the same account can lose
    /// one increment (AUTH14). That is accepted deliberately — the counter is a
    /// display statistic, not a security or correctness invariant, and the row
    /// is owned by the engine as the single source of truth for the table, so an
    /// atomic `SET login_count = login_count + 1` would mean a hand-written
    /// write that bypasses it. The `last_login` stamp it writes is correct
    /// regardless of which concurrent writer wins.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn record_login(&self, id: &str) -> Result<()> {
        let Some(document) = self.load(id).await? else {
            return Ok(());
        };
        let logins = document
            .get("login_count")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + 1;
        let mut fields = Map::new();
        fields.insert(
            "last_login".to_string(),
            Value::from(chrono::Utc::now().to_rfc3339()),
        );
        fields.insert("login_count".to_string(), Value::from(logins));
        self.patch(id, fields).await
    }

    /// Verify credentials against the stored Argon2 hash.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn verify_login(&self, login: &str, password: &str) -> Result<Option<StoredUser>> {
        let Some((id, hash)) = self.password_hash(login).await? else {
            return Ok(None);
        };
        // Argon2id is deliberately expensive. Verifying it on an async worker
        // blocked that worker for the whole hash (7 s in a debug build) and was
        // one of the reasons a second parallel login made the desk crawl. Move
        // the CPU-bound half to the blocking pool; the runtime stays responsive
        // while the password is checked.
        let password = password.to_string();
        let verified = tokio::task::spawn_blocking(move || {
            crate::verify_password(&password, &hash).unwrap_or(false)
        })
        .await;
        let ok = match verified {
            Ok(ok) => ok,
            Err(error) => {
                tracing::warn!(%error, "password verification task failed");
                false
            }
        };
        if !ok {
            return Ok(None);
        }
        self.find_by_id(&id).await
    }
}
