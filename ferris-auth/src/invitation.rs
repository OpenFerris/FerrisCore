//! User invitations.
//!
//! An invitation is a single-use token that lets somebody create a `User`
//! account without an administrator typing a password for them. It is stored as
//! a [`User Invitation`](crate::doctypes::user_invitation_meta) document, so the
//! document engine remains the single source of truth and the desk can list
//! pending invitations.
//!
//! # Token handling
//!
//! The plaintext token is 32 random bytes, URL-safe and unpadded, returned by
//! [`InvitationStore::create`] exactly once. Only its SHA-256 hex digest is
//! persisted (`token_hash`, `permlevel` 1, hidden) and it is compared with
//! [`crate::crypto::constant_time_eq`] on acceptance. A leaked table therefore
//! leaks no usable link.
//!
//! # Single use and ordering
//!
//! Acceptance first **claims** the row with one conditional
//! `UPDATE ... SET accepted = ?, token_hash = NULL WHERE name = ? AND token_hash = ?`.
//! Only one caller can win that statement, so two concurrent acceptances cannot
//! both create an account. The user is created **after** the claim; if the
//! engine rejects the password (policy) the claim is released, so the invited
//! person can retry instead of needing a new invitation.
//!
//! Sending the invitation email is not this crate's job: the caller renders the
//! message and sends it through [`ferris_core::ports::MailerPort`]. This module
//! never depends on `ferris-email`.
//!
//! Every timestamp is an RFC 3339 column, parsed with
//! [`chrono::DateTime::parse_from_rfc3339`].

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use ferris_core::{AuthError, DocumentError, Error, Map, Result, ValidationError, Value};
use ferris_db::{Db, Dialect, Stmt};
use ferris_document::{DocumentEngine, ListQuery};
use rand::RngCore;
use std::collections::HashMap;
use uuid::Uuid;

use crate::UserType;
use crate::crypto::constant_time_eq;
use crate::store_support::system_context;
use crate::user_store::{StoredUser, UserStore};

/// The name of the `DocType` the store persists.
const DOCTYPE: &str = "User Invitation";

/// Bytes of entropy in an invitation token.
const TOKEN_BYTES: usize = 32;

/// An invitation as the desk sees it (never the token or its digest).
#[derive(Debug, Clone)]
pub struct Invitation {
    /// Document name (the id `revoke` takes).
    pub id: String,
    /// Address the invitation was sent to.
    pub email: String,
    /// Name the inviter pre-filled, if any.
    pub full_name: Option<String>,
    /// Roles the accepted account receives.
    pub roles: Vec<String>,
    /// Account that issued the invitation.
    pub invited_by: Option<String>,
    /// When the token stops being accepted.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether the invitation was already used.
    pub accepted: bool,
    /// When it was accepted.
    pub accepted_at: Option<DateTime<Utc>>,
}

/// Engine-backed invitation store.
#[derive(Debug, Clone)]
pub struct InvitationStore {
    engine: DocumentEngine,
    users: UserStore,
    db: Db,
}

impl InvitationStore {
    /// Build a store over `db`. Call [`Self::init`] once before the first write
    /// (the boot path and the tests do).
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self {
            engine: DocumentEngine::new(db.clone()),
            users: UserStore::new(db.clone()),
            db,
        }
    }

    /// The user store acceptance creates accounts through.
    #[must_use]
    pub const fn users(&self) -> &UserStore {
        &self.users
    }

    /// Register the declaration and create (or extend) the `tab_user_invitation`
    /// table.
    ///
    /// `refresh_table`, not `ensure_table`: the engine caches the tables it has
    /// ensured per database, so a table created by an earlier release would keep
    /// its old shape and never gain `token_hash`/`expires_at`/`accepted_at`.
    ///
    /// # Errors
    ///
    /// Database errors from creating the table or its indexes.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn init(&self) -> Result<()> {
        ferris_schema::register_doctype(crate::doctypes::user_invitation_meta());
        self.engine.refresh_table(DOCTYPE).await.map(|_| ())
    }

    /// Issue an invitation and return it with the **plaintext** token.
    ///
    /// `ttl` is how long the link stays valid. `invited_by` is the account that
    /// issued it; it is recorded for the desk and is never an authorization
    /// input here.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Invalid`] for an empty email, or a storage error.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn create(
        &self,
        email: &str,
        full_name: Option<&str>,
        roles: &[String],
        ttl: Duration,
        invited_by: Option<&str>,
    ) -> Result<(Invitation, String)> {
        let email = email.trim();
        if email.is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "an invitation requires an email".to_string(),
            )));
        }
        let roles_json = serde_json::to_string(roles)
            .map_err(|error| Error::Serialization(ferris_core::SerializationError::Json(error)))?;
        let token = random_token();
        let expires_at = Utc::now() + ttl;
        let mut map = Map::new();
        map.insert("name".to_string(), Value::from(Uuid::new_v4().to_string()));
        map.insert("email".to_string(), Value::from(email));
        map.insert(
            "full_name".to_string(),
            full_name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map_or(Value::Null, Value::from),
        );
        map.insert("roles".to_string(), Value::from(roles_json));
        map.insert(
            "expires_at".to_string(),
            Value::from(expires_at.to_rfc3339()),
        );
        map.insert(
            "invited_by".to_string(),
            invited_by
                .map(str::trim)
                .filter(|account| !account.is_empty())
                .map_or(Value::Null, Value::from),
        );
        map.insert("token_hash".to_string(), Value::from(digest_of(&token)));
        map.insert("accepted".to_string(), Value::from(false));

        let meta = crate::doctypes::user_invitation_meta();
        let ctx = system_context(&meta);
        let created = self.engine.create(&ctx, Value::Object(map)).await?;
        Ok((from_document(&created), token))
    }

    /// Accept an invitation: create the `User` and consume the token.
    ///
    /// `full_name` overrides the invited name when non-empty; `password` is the
    /// plaintext the engine hashes on write.
    ///
    /// Answers `Ok(None)` for an unknown, expired, already-accepted or
    /// revoked token, and for an email that already has an account (the
    /// invitation is then left untouched). It never reveals *which* of those it
    /// was, so the public endpoint cannot probe.
    ///
    /// # Errors
    ///
    /// Storage errors, and the engine's validation error when the password
    /// violates the policy (the claim is then released).
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn accept(
        &self,
        token: &str,
        full_name: &str,
        password: &str,
    ) -> Result<Option<StoredUser>> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }
        let digest = digest_of(token);
        let meta = crate::doctypes::user_invitation_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            filters: HashMap::from([("token_hash".to_string(), Value::from(digest.as_str()))]),
            limit: Some(1),
            ..ListQuery::default()
        };
        let (documents, _) = self.engine.list(&ctx, &query).await?;
        let Some(document) = documents.first() else {
            return Ok(None);
        };
        if !stored_digest_matches(document.get("token_hash"), &digest) {
            return Ok(None);
        }
        let invitation = from_document(document);
        if invitation.accepted || invitation.email.is_empty() || !is_live(&invitation) {
            return Ok(None);
        }

        // A duplicate address is refused **before** the claim, so the pending
        // invitation survives and the caller can simply sign in instead.
        if self
            .users
            .first_by("email", &invitation.email)
            .await?
            .is_some()
        {
            return Ok(None);
        }

        if !self.claim(&invitation.id, &digest).await? {
            // Another acceptance consumed the token first.
            return Ok(None);
        }

        let name = full_name.trim();
        let full_name = if name.is_empty() {
            invitation
                .full_name
                .clone()
                .unwrap_or_else(|| invitation.email.clone())
        } else {
            name.to_string()
        };
        match self
            .users
            .create(
                &invitation.email,
                &full_name,
                None,
                UserType::SystemUser,
                password,
                &invitation.roles,
            )
            .await
        {
            Ok(user) => Ok(Some(user)),
            Err(error) => {
                // Release the claim so a rejected password does not burn the
                // invitation. A failed release is only logged: the original
                // error is the one the caller needs.
                if let Err(release_error) = self.release(&invitation.id, &digest).await {
                    tracing::warn!(
                        %release_error,
                        invitation = %invitation.id,
                        "could not release an unclaimed invitation"
                    );
                }
                Err(error)
            }
        }
    }

    /// Every invitation, newest first by the engine's declared sort.
    ///
    /// # Errors
    ///
    /// Storage errors.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn list(&self) -> Result<Vec<Invitation>> {
        let meta = crate::doctypes::user_invitation_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            limit: Some(u32::MAX),
            ..ListQuery::default()
        };
        let (documents, _) = self.engine.list(&ctx, &query).await?;
        Ok(documents.iter().map(from_document).collect())
    }

    /// Revoke an invitation by document id.
    ///
    /// A cancelled draft is deleted hard, so the token stops resolving at once.
    ///
    /// # Errors
    ///
    /// [`AuthError::NotFound`] for an unknown id, or a storage error.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn revoke(&self, id: &str) -> Result<()> {
        let meta = crate::doctypes::user_invitation_meta();
        let ctx = system_context(&meta);
        if let Err(error) = self.engine.get(&ctx, id).await {
            return match error {
                Error::Document(DocumentError::NotFound { .. }) => {
                    Err(Error::Auth(AuthError::NotFound(format!("invitation {id}"))))
                }
                other => Err(other),
            };
        }
        self.engine.delete(&ctx, id).await
    }

    /// Claim `id` for acceptance and consume its token in one statement.
    ///
    /// `true` only when this call moved the row from unaccepted to accepted;
    /// the token column is cleared in the same `UPDATE`, so the digest cannot
    /// match again.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    async fn claim(&self, id: &str, digest: &str) -> Result<bool> {
        let table = self.table()?;
        let name = self.column("name")?;
        let accepted = self.column("accepted")?;
        let accepted_at = self.column("accepted_at")?;
        let token = self.column("token_hash")?;
        let placeholder = datetime_placeholder(self.db.dialect());
        let sql = format!(
            "UPDATE {table} SET {accepted} = ?, {accepted_at} = {placeholder}, {token} = NULL \
             WHERE {name} = ? AND {token} = ?"
        );
        let affected = self
            .db
            .execute(&Stmt::new(
                sql,
                vec![
                    Value::from(true),
                    Value::from(Utc::now().to_rfc3339()),
                    Value::from(id),
                    Value::from(digest),
                ],
            ))
            .await?;
        Ok(affected.rows() > 0)
    }

    /// Undo [`Self::claim`] after a failed user creation.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    async fn release(&self, id: &str, digest: &str) -> Result<()> {
        let table = self.table()?;
        let name = self.column("name")?;
        let accepted = self.column("accepted")?;
        let accepted_at = self.column("accepted_at")?;
        let token = self.column("token_hash")?;
        self.db
            .execute(&Stmt::new(
                format!(
                    "UPDATE {table} SET {accepted} = ?, {accepted_at} = NULL, {token} = ? \
                     WHERE {name} = ? AND {accepted} = ?"
                ),
                vec![
                    Value::from(false),
                    Value::from(digest),
                    Value::from(id),
                    Value::from(true),
                ],
            ))
            .await?;
        Ok(())
    }

    /// The dialect-quoted `tab_user_invitation` table.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    fn table(&self) -> Result<String> {
        let table = ferris_document::table_name(DOCTYPE)?;
        Ok(ferris_db::sql::Ident::new(&table)?.quote(self.db.dialect()))
    }

    /// One dialect-quoted invitation column.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    fn column(&self, name: &str) -> Result<String> {
        Ok(ferris_db::sql::Ident::new(name)?.quote(self.db.dialect()))
    }
}

/// A new URL-safe, unpadded 32-byte token.
fn random_token() -> String {
    let mut buffer = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

/// The stored form of `token`.
fn digest_of(token: &str) -> String {
    crate::crypto::sha256_hex(token.as_bytes())
}

/// Whether the stored digest column holds exactly the digest of the token.
fn stored_digest_matches(stored: Option<&Value>, computed: &str) -> bool {
    stored.and_then(Value::as_str).is_some_and(|stored| {
        stored.len() == computed.len()
            && stored.bytes().all(|byte| byte.is_ascii_hexdigit())
            && constant_time_eq(stored.as_bytes(), computed.as_bytes())
    })
}

/// Whether the invitation's RFC 3339 deadline is still in the future.
///
/// A missing or unparseable deadline counts as expired: fail closed.
fn is_live(invitation: &Invitation) -> bool {
    invitation
        .expires_at
        .is_some_and(|expires_at| expires_at > Utc::now())
}

/// Engine document → [`Invitation`].
fn from_document(document: &Value) -> Invitation {
    let text = |name: &str| {
        document
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Invitation {
        id: text("name"),
        email: text("email"),
        full_name: optional_text(document, "full_name"),
        roles: serde_json::from_str(&text("roles")).unwrap_or_default(),
        invited_by: optional_text(document, "invited_by"),
        expires_at: parse_datetime(document.get("expires_at")),
        accepted: document
            .get("accepted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        accepted_at: parse_datetime(document.get("accepted_at")),
    }
}

/// A text column as `Some` only when it carries something.
fn optional_text(document: &Value, name: &str) -> Option<String> {
    document
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Parse an RFC 3339 timestamp column.
fn parse_datetime(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc))
}

/// The placeholder a `Datetime` column needs on each backend.
const fn datetime_placeholder(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres => "CAST(? AS TIMESTAMP)",
        Dialect::MySQL => "CAST(? AS DATETIME)",
        Dialect::SQLite => "?",
    }
}

#[cfg(test)]
mod tests;
