//! Server-side OAuth `state` storage.
//!
//! Two backends behind one enum:
//!
//! * [`MemoryStateStore`] — the historical behaviour: a process-local map. It
//!   binds the callback to the browser that started the flow but not to the
//!   replica that minted it, so it only works behind a sticky load balancer.
//! * [`DbStateStore`] — `tab_oauth_state`, shared by every replica:
//!
//!   | column          | type            | purpose                          |
//!   |-----------------|-----------------|----------------------------------|
//!   | `state`         | `VARCHAR(255)`  | primary key, the one-time token  |
//!   | `provider`      | `VARCHAR(255)`  | provider that minted it          |
//!   | `pkce_verifier` | `VARCHAR(1024)` | PKCE verifier, `NULL` when off   |
//!   | `expires_at`    | `BIGINT`        | expiry as a Unix timestamp (s)   |
//!
//! Single use is enforced by the store, not the caller: [`take`] selects the
//! row and then deletes it **with `expires_at > now` in the `WHERE` clause**,
//! requiring exactly one affected row. Two concurrent callbacks can both see
//! the row, but only one `DELETE` matches; the loser answers `None` and its
//! callback is refused. That is what makes a replayed `code`+`state` pair fail
//! even when the two requests land on different replicas.
//!
//! Expired rows are refused on read and swept by [`sweep`]; [`authorize`]
//! sweeps on every login so an abandoned flow cannot grow the table without
//! bound.
//!
//! [`take`]: StateStore::take
//! [`sweep`]: StateStore::sweep
//! [`authorize`]: super::OAuthManager::authorize

// `Error` is the crate-wide error type; boxing it would leak into every caller.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use ferris_core::{DatabaseError, Error, Result, Value};
use ferris_db::sql::{Column, Ident};
use ferris_db::{Db, Stmt};
use tokio::sync::OnceCell;

/// The table the DB-backed store owns.
pub(super) const OAUTH_STATE_TABLE: &str = "tab_oauth_state";

/// A pending OAuth authorization state.
///
/// `pub(super)` so the crate's own tests can seed an expired entry directly;
/// it is not part of the public API.
#[derive(Debug, Clone)]
pub(super) struct PendingState {
    pub(super) provider: String,
    pub(super) pkce_verifier: Option<String>,
    pub(super) expires_at: DateTime<Utc>,
}

/// Where pending states live.
#[derive(Debug, Clone)]
pub(super) enum StateStore {
    Memory(MemoryStateStore),
    Db(DbStateStore),
}

impl StateStore {
    /// A process-local store (the default).
    pub(super) fn memory() -> Self {
        Self::Memory(MemoryStateStore::default())
    }

    /// A store shared through `tab_oauth_state`.
    pub(super) fn from_db(db: Db) -> Self {
        Self::Db(DbStateStore::new(db))
    }

    /// Persist `pending` under `state`.
    ///
    /// # Errors
    ///
    /// Database errors from the insert or the table creation.
    pub(super) async fn put(&mut self, state: &str, pending: PendingState) -> Result<()> {
        match self {
            Self::Memory(store) => {
                store.entries.insert(state.to_string(), pending);
                Ok(())
            }
            Self::Db(store) => store.put(state, &pending).await,
        }
    }

    /// Consume `state` and answer what was stored under it.
    ///
    /// `None` means unknown, already consumed by another callback, or lost a
    /// concurrent race; the caller cannot tell those apart (and must not). An
    /// expired entry is still returned so the caller can answer
    /// [`AuthError::TokenExpired`](ferris_core::AuthError::TokenExpired) instead
    /// of the generic invalid-state error; it has been removed either way.
    ///
    /// # Errors
    ///
    /// Database errors from the lookup, the delete or the table creation.
    pub(super) async fn take(&mut self, state: &str) -> Result<Option<PendingState>> {
        match self {
            Self::Memory(store) => Ok(store.entries.remove(state)),
            Self::Db(store) => store.take(state).await,
        }
    }

    /// Drop every expired entry.
    ///
    /// # Errors
    ///
    /// Database errors from the sweep or the table creation.
    pub(super) async fn sweep(&mut self) -> Result<()> {
        match self {
            Self::Memory(store) => {
                let now = Utc::now();
                store.entries.retain(|_, pending| pending.expires_at > now);
                Ok(())
            }
            Self::Db(store) => store.sweep().await,
        }
    }

    /// Number of entries held in memory; `None` for the DB backend.
    ///
    /// Test-only: the DB store has no cheap synchronous count and production
    /// code has no reason to ask.
    #[cfg(test)]
    pub(super) fn memory_len(&self) -> Option<usize> {
        match self {
            Self::Memory(store) => Some(store.entries.len()),
            Self::Db(_) => None,
        }
    }

    /// Whether an in-memory entry exists; `false` for the DB backend.
    #[cfg(test)]
    pub(super) fn memory_contains(&self, state: &str) -> bool {
        match self {
            Self::Memory(store) => store.entries.contains_key(state),
            Self::Db(_) => false,
        }
    }
}

/// The process-local backend.
#[derive(Debug, Clone, Default)]
pub(super) struct MemoryStateStore {
    entries: HashMap<String, PendingState>,
}

/// The `tab_oauth_state` backend.
#[derive(Debug, Clone)]
pub(super) struct DbStateStore {
    db: Db,
    /// Set once per store value (and shared by its clones) after the table has
    /// been ensured. The DDL is idempotent, so a replica racing this one is
    /// harmless.
    ready: Arc<OnceCell<()>>,
}

impl DbStateStore {
    fn new(db: Db) -> Self {
        Self {
            db,
            ready: Arc::new(OnceCell::new()),
        }
    }

    /// Create the table and its expiry index if absent.
    async fn ensure_table(&self) -> Result<()> {
        self.ready
            .get_or_try_init(|| async {
                self.db.ensure_table(OAUTH_STATE_TABLE, &columns()).await?;
                // The sweep filters on `expires_at`; without the index it scans
                // the whole table. Idempotent, duplicate-name tolerant.
                self.db
                    .ensure_index(
                        "idx_oauth_state_expires",
                        OAUTH_STATE_TABLE,
                        &["expires_at"],
                        false,
                    )
                    .await
            })
            .await?;
        Ok(())
    }

    /// The dialect-quoted table name.
    fn table(&self) -> Result<String> {
        Ok(Ident::new(OAUTH_STATE_TABLE)?.quote(self.db.dialect()))
    }

    async fn put(&self, state: &str, pending: &PendingState) -> Result<()> {
        self.ensure_table().await?;
        let table = self.table()?;
        let sql = format!(
            "INSERT INTO {table} (state, provider, pkce_verifier, expires_at) \
             VALUES (?, ?, ?, ?)"
        );
        self.db
            .execute(&Stmt::new(
                sql,
                vec![
                    Value::from(state),
                    Value::from(pending.provider.as_str()),
                    pending
                        .pkce_verifier
                        .as_deref()
                        .map_or(Value::Null, Value::from),
                    Value::from(pending.expires_at.timestamp()),
                ],
            ))
            .await
            .map(|_| ())
    }

    async fn take(&self, state: &str) -> Result<Option<PendingState>> {
        self.ensure_table().await?;
        let table = self.table()?;
        let Some(row) = self
            .db
            .fetch_optional(&Stmt::new(
                format!(
                    "SELECT provider, pkce_verifier, expires_at \
                     FROM {table} WHERE state = ?"
                ),
                vec![Value::from(state)],
            ))
            .await?
        else {
            return Ok(None);
        };
        let provider = row.str("provider")?;
        let pkce_verifier = row
            .opt_str("pkce_verifier")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let expires_at = row.i64("expires_at")?;
        let expires_at = DateTime::from_timestamp(expires_at, 0).ok_or_else(|| {
            Error::Database(DatabaseError::Query(
                "oauth state row has an unreadable expires_at".to_string(),
            ))
        })?;
        let pending = PendingState {
            provider,
            pkce_verifier,
            expires_at,
        };

        let now = Utc::now();
        if expires_at <= now {
            // Dead either way; deleting it keeps the table clean. It is still
            // returned so the caller answers "expired", not "invalid".
            self.delete(state).await?;
            return Ok(Some(pending));
        }

        // The atomic half: only the statement that actually deletes the row
        // wins, so two concurrent callbacks cannot both redeem the state.
        let affected = self
            .db
            .execute(&Stmt::new(
                format!("DELETE FROM {table} WHERE state = ? AND expires_at > ?"),
                vec![Value::from(state), Value::from(now.timestamp())],
            ))
            .await?;
        if affected.rows() == 0 {
            return Ok(None);
        }
        Ok(Some(pending))
    }

    async fn delete(&self, state: &str) -> Result<()> {
        let table = self.table()?;
        self.db
            .execute(&Stmt::new(
                format!("DELETE FROM {table} WHERE state = ?"),
                vec![Value::from(state)],
            ))
            .await
            .map(|_| ())
    }

    async fn sweep(&self) -> Result<()> {
        self.ensure_table().await?;
        let table = self.table()?;
        self.db
            .execute(&Stmt::new(
                format!("DELETE FROM {table} WHERE expires_at <= ?"),
                vec![Value::from(Utc::now().timestamp())],
            ))
            .await
            .map(|_| ())
    }
}

/// The table shape, shared with `Db::ensure_table`.
fn columns() -> Vec<Column> {
    vec![
        Column::new("state", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("provider", "VARCHAR(255)").not_null(),
        Column::new("pkce_verifier", "VARCHAR(1024)"),
        Column::new("expires_at", "BIGINT").not_null(),
    ]
}
