//! A `tower-sessions` store backed by [`ferris_db::Db`].
//!
//! The table is `tab_session_store`:
//!
//! | column    | type           | purpose                                   |
//! |-----------|----------------|-------------------------------------------|
//! | `id`      | `VARCHAR(255)` | the `tower_sessions::session::Id`, Base64 |
//! | `user_id` | `VARCHAR(255)` | the account behind the session, or `NULL` |
//! | `data`    | `TEXT NOT NULL`| the session map, serialized JSON          |
//! | `expiry`  | `BIGINT NOT NULL`| expiry as a Unix timestamp (seconds)     |
//!
//! `user_id` is denormalised out of the JSON map (key
//! [`USER_ID_KEY`](super::keys::USER_ID_KEY)) so "sign out everywhere" is one
//! indexed `DELETE` instead of a full scan and deserialisation of every row.
//! It is also what lets the realtime handshake tell a live socket's owner
//! apart from a forged cookie id.
//!
//! `id` is a bounded `VARCHAR` rather than `TEXT` because MySQL refuses a
//! `TEXT`/`BLOB` primary key without a key length, and the store must run on
//! all three dialects. The encoded id is a fixed 22 characters, so nothing is
//! lost.
//!
//! The values are JSON, and only the *data map* is stored — the id and expiry
//! are columns. Nothing in this module logs session contents: errors carry the
//! driver message (table/column names, never bound values) and the sweep only
//! counts rows.

use std::sync::Arc;

use async_trait::async_trait;
use ferris_core::Value;
use ferris_db::sql::{Column, Ident};
use ferris_db::{Db, Stmt, UpsertBuilder, UpsertSet};
use time::OffsetDateTime;
use tokio::sync::OnceCell;
use tower_sessions::{
    ExpiredDeletion, SessionStore,
    session::{Id, Record},
    session_store,
};

/// The table this store owns.
pub const SESSION_TABLE: &str = "tab_session_store";

/// The identity a live session carries, as the realtime handshake reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub user_id: String,
    pub email: String,
    pub roles: Vec<String>,
    pub session_id: String,
}

/// The largest `id` the `tower-sessions` base64 encoding produces (22 bytes).
/// `VARCHAR(255)` matches the rest of the framework's key columns.
fn columns() -> Vec<Column> {
    vec![
        Column::new("id", "VARCHAR(255)").not_null().primary_key(),
        Column::new("user_id", "VARCHAR(255)"),
        Column::new("data", "TEXT").not_null(),
        Column::new("expiry", "BIGINT").not_null(),
    ]
}

/// The account id stored in `record`, when the session was stamped by a login.
fn record_user_id(record: &Record) -> Option<&str> {
    record
        .data
        .get(super::keys::USER_ID_KEY)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The current Unix timestamp, in seconds.
fn now_unix() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

/// A [`SessionStore`] that persists sessions in `tab_session_store`.
#[derive(Debug, Clone)]
pub struct DbSessionStore {
    db: Db,
    /// Set once per store value (and shared by its clones) after the table has
    /// been ensured. The DDL itself is idempotent, so a second process racing
    /// this one is harmless.
    ready: Arc<OnceCell<()>>,
}

impl DbSessionStore {
    /// Build a store over `db`; the table is created on first use.
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self {
            db,
            ready: Arc::new(OnceCell::new()),
        }
    }

    /// Create the table and its expiry index if absent.
    ///
    /// Called lazily by every operation; a boot sequence may call it eagerly so
    /// a permissions problem surfaces at startup instead of on the first
    /// request.
    ///
    /// # Errors
    ///
    /// Database errors from the DDL.
    pub async fn ensure_table(&self) -> session_store::Result<()> {
        self.ready
            .get_or_try_init(|| async {
                self.db
                    .ensure_table(SESSION_TABLE, &columns())
                    .await
                    .map_err(|error| backend(&error))?;
                // A deployment created before the user index has the table
                // without `user_id`; add it idempotently so "sign out
                // everywhere" works after an upgrade.
                self.db
                    .ensure_column(SESSION_TABLE, &Column::new("user_id", "VARCHAR(255)"))
                    .await
                    .map_err(|error| backend(&error))?;
                // The sweeper filters on `expiry`; without the index it scans
                // the whole table. Idempotent, and tolerant of the duplicate
                // index error on MySQL.
                self.db
                    .ensure_index(
                        "idx_session_store_expiry",
                        SESSION_TABLE,
                        &["expiry"],
                        false,
                    )
                    .await
                    .map_err(|error| backend(&error))?;
                // "Sign out everywhere" deletes by user; the realtime check
                // also reads it.
                self.db
                    .ensure_index("idx_session_store_user", SESSION_TABLE, &["user_id"], false)
                    .await
                    .map_err(|error| backend(&error))
            })
            .await?;
        Ok(())
    }

    /// Every session of `user_id` is deleted, answering how many rows went.
    ///
    /// This is the storage half of "sign out everywhere". The caller fires the
    /// revocation hook so realtime sockets close; this method only removes the
    /// records, and the indexed `DELETE` is why the table carries `user_id`.
    ///
    /// # Errors
    ///
    /// Database errors, or a table that cannot be created.
    pub async fn delete_user_sessions(&self, user_id: &str) -> session_store::Result<u64> {
        self.ensure_table().await?;
        let stmt = Stmt::new(
            format!("DELETE FROM {} WHERE user_id = ?", self.table()?),
            vec![Value::from(user_id)],
        );
        self.db
            .execute(&stmt)
            .await
            .map(|affected| affected.rows())
            .map_err(|error| backend(&error))
    }

    /// The account stored on session `id`, when the record exists and is live.
    ///
    /// # Errors
    ///
    /// Database errors, or a table that cannot be created.
    pub async fn session_user_id(&self, id: &str) -> session_store::Result<Option<String>> {
        let id = id
            .parse::<Id>()
            .map_err(|error| session_store::Error::Decode(error.to_string()))?;
        Ok(self
            .load(&id)
            .await?
            .as_ref()
            .and_then(record_user_id)
            .map(str::to_string))
    }

    /// The full identity a live session carries, for the realtime handshake.
    ///
    /// The socket upgrade is handled before the HTTP auth layers, so it reads
    /// the record directly rather than an `AuthSession`. The values were
    /// stamped at login; the maintenance task revalidates them against the
    /// account row, so a role change or a disabled account still closes the
    /// socket.
    ///
    /// # Errors
    ///
    /// Database errors, or a table that cannot be created.
    pub async fn session_identity(
        &self,
        id: &str,
    ) -> session_store::Result<Option<SessionIdentity>> {
        let id = id
            .parse::<Id>()
            .map_err(|error| session_store::Error::Decode(error.to_string()))?;
        let Some(record) = self.load(&id).await? else {
            return Ok(None);
        };
        let Some(user_id) = record_user_id(&record) else {
            return Ok(None);
        };
        let email = record
            .data
            .get(super::keys::EMAIL_KEY)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let roles = record
            .data
            .get(super::keys::ROLES_KEY)
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            .unwrap_or_default();
        Ok(Some(SessionIdentity {
            user_id: user_id.to_string(),
            email,
            roles,
            session_id: id.to_string(),
        }))
    }

    /// The CSRF token stored on session `id`, when the record is live.
    ///
    /// Used by surfaces that hold only a cookie header (the plugin app
    /// dispatcher); the REST gate reads the same value from the `Session`
    /// extension.
    ///
    /// # Errors
    ///
    /// Database errors, or a table that cannot be created.
    pub async fn session_csrf(&self, id: &str) -> session_store::Result<Option<String>> {
        let id = id
            .parse::<Id>()
            .map_err(|error| session_store::Error::Decode(error.to_string()))?;
        Ok(self.load(&id).await?.and_then(|record| {
            record
                .data
                .get(super::keys::CSRF_TOKEN_KEY)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        }))
    }

    /// Number of live sessions `user_id` currently holds.
    ///
    /// # Errors
    ///
    /// Database errors, or a table that cannot be created.
    pub async fn count_user_sessions(&self, user_id: &str) -> session_store::Result<u64> {
        self.ensure_table().await?;
        let stmt = Stmt::new(
            format!(
                "SELECT COUNT(*) AS sessions FROM {} WHERE user_id = ? AND expiry > ?",
                self.table()?
            ),
            vec![Value::from(user_id), Value::from(now_unix())],
        );
        let row = self
            .db
            .fetch_optional(&stmt)
            .await
            .map_err(|error| backend(&error))?;
        Ok(row
            .and_then(|row| row.opt_i64("sessions"))
            .unwrap_or(0)
            .max(0)
            .unsigned_abs())
    }

    /// Whether session `id` exists, is unexpired and belongs to `user_id`.
    ///
    /// Used by the realtime engine's revalidation: the socket recorded whose
    /// session opened it at the handshake, and a deleted or reassigned session
    /// must close the socket even though the browser cookie is still present.
    #[must_use]
    pub async fn is_active(&self, id: &str, user_id: &str) -> bool {
        matches!(self.session_user_id(id).await, Ok(Some(owner)) if owner == user_id)
    }

    /// The quoted table name for this connection's dialect.
    fn table(&self) -> session_store::Result<String> {
        Ident::new(SESSION_TABLE)
            .map(|table| table.quote(self.db.dialect()))
            .map_err(|error| backend(&error))
    }

    /// Whether `id` already has a row (expired or not).
    async fn exists(&self, id: &Id) -> session_store::Result<bool> {
        let stmt = Stmt::new(
            format!("SELECT 1 AS present FROM {} WHERE id = ?", self.table()?),
            vec![Value::from(id.to_string())],
        );
        self.db
            .fetch_optional(&stmt)
            .await
            .map(|row| row.is_some())
            .map_err(|error| backend(&error))
    }

    /// Delete every expired row now.
    ///
    /// The [`ExpiredDeletion`] trait method; exposed as an inherent alias so a
    /// caller can sweep without importing the trait.
    ///
    /// # Errors
    ///
    /// Database errors, or a table that cannot be created.
    pub async fn cleanup_expired(&self) -> session_store::Result<()> {
        self.delete_expired().await
    }

    /// Sweep expired rows every `interval`, on the current Tokio runtime.
    ///
    /// Returns `false` when there is no runtime to attach the task to. The
    /// first tick fires immediately, so the sweep also runs at boot. A failure
    /// is logged and the loop continues: a transient database error must not
    /// stop expiry cleanup for the life of the process.
    #[must_use]
    pub fn spawn_cleanup_task(&self, interval: std::time::Duration) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let store = self.clone();
        handle.spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(error) = store.delete_expired().await {
                    tracing::warn!(%error, "session store cleanup failed");
                }
            }
        });
        true
    }
}

#[async_trait]
impl SessionStore for DbSessionStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        self.ensure_table().await?;
        // A 128-bit random id colliding is astronomically unlikely; the trait
        // still asks the store to resolve it rather than silently overwrite an
        // existing session. Four attempts is generous.
        for _ in 0..4 {
            if !self.exists(&record.id).await? {
                return self.save(record).await;
            }
            record.id = Id::default();
        }
        Err(session_store::Error::Backend(
            "could not allocate a unique session id".to_string(),
        ))
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        self.ensure_table().await?;
        let data = serde_json::to_string(&record.data)
            .map_err(|error| session_store::Error::Encode(error.to_string()))?;
        let user_id = record_user_id(record).map_or(Value::Null, Value::from);
        let upsert = UpsertBuilder::new(self.db.dialect(), SESSION_TABLE)
            .values([
                ("id", Value::from(record.id.to_string())),
                ("user_id", user_id),
                ("data", Value::from(data)),
                ("expiry", Value::from(record.expiry_date.unix_timestamp())),
            ])
            .conflict_on(["id"])
            .on_update("user_id", UpsertSet::FromExcluded("user_id".to_string()))
            .on_update("data", UpsertSet::FromExcluded("data".to_string()))
            .on_update("expiry", UpsertSet::FromExcluded("expiry".to_string()))
            .build()
            .map_err(|error| backend(&error))?;
        self.db
            .upsert(upsert)
            .await
            .map(|_| ())
            .map_err(|error| backend(&error))
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        self.ensure_table().await?;
        let stmt = Stmt::new(
            format!("SELECT data, expiry FROM {} WHERE id = ?", self.table()?),
            vec![Value::from(session_id.to_string())],
        );
        let Some(row) = self
            .db
            .fetch_optional(&stmt)
            .await
            .map_err(|error| backend(&error))?
        else {
            return Ok(None);
        };
        let expiry = row.opt_i64("expiry").ok_or_else(|| {
            session_store::Error::Decode("session row has no integer expiry".to_string())
        })?;
        if expiry <= now_unix() {
            return Ok(None);
        }
        let data = row
            .str("data")
            .map_err(|error| session_store::Error::Decode(error.to_string()))?;
        let data = serde_json::from_str(&data)
            .map_err(|error| session_store::Error::Decode(error.to_string()))?;
        let expiry_date = OffsetDateTime::from_unix_timestamp(expiry)
            .map_err(|error| session_store::Error::Decode(error.to_string()))?;
        Ok(Some(Record {
            id: *session_id,
            data,
            expiry_date,
        }))
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        self.ensure_table().await?;
        let stmt = Stmt::new(
            format!("DELETE FROM {} WHERE id = ?", self.table()?),
            vec![Value::from(session_id.to_string())],
        );
        self.db
            .execute(&stmt)
            .await
            .map(|_| ())
            .map_err(|error| backend(&error))
    }
}

#[async_trait]
impl ExpiredDeletion for DbSessionStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        self.ensure_table().await?;
        let stmt = Stmt::new(
            format!("DELETE FROM {} WHERE expiry <= ?", self.table()?),
            vec![Value::from(now_unix())],
        );
        self.db
            .execute(&stmt)
            .await
            .map(|_| ())
            .map_err(|error| backend(&error))
    }
}

/// Map a database failure onto the store error type.
///
/// The message is the driver's, which names the statement shape but not the
/// bound values, so no session content can leak through it.
fn backend(error: &ferris_core::Error) -> session_store::Error {
    session_store::Error::Backend(error.to_string())
}
