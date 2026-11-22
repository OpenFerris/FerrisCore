//! The execution boundary.
//!
//! [`Db`] is the only type the rest of the framework needs in order to talk to
//! a database. It owns the connection pool **and** the dialect, so a caller
//! cannot run Postgres-only SQL against MySQL by forgetting a flag — which is
//! what happened when [`crate::TransactionManager`] and
//! [`crate::MigrationRunner`] each defaulted to `Dialect::Postgres`.
//!
//! The convenience methods here return framework types only — no `sqlx` type
//! appears in their signatures, which is what lets the other crates stop
//! depending on `sqlx`. The escape hatches are the exception, and are
//! deliberate: a caller that has to hand the pool to a driver `ferris-db` does
//! not wrap needs [`Db::pool`] / [`Db::into_pool`], and the row decoder needs
//! the raw row [`Row::new`] accepts. Those three signatures do name `AnyPool`
//! and `AnyRow`, and that is the whole set.

mod tx;

#[cfg(test)]
mod tests;

pub use tx::Tx;

use crate::dialect::Dialect;
use crate::error;
use crate::row::Row;
use crate::sql::Upsert;
use ferris_core::{Error, Result, Value};
use sqlx::{
    Any, AnyPool, Column, Row as SqlxRow,
    any::{AnyArguments, AnyPoolOptions, AnyRow},
    query::Query,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// A statement bound for execution.
pub struct Stmt {
    sql: String,
    params: Vec<Value>,
}

impl Stmt {
    /// A statement with no parameters.
    ///
    /// The caller is responsible for the SQL: use [`crate::sql`] for anything
    /// containing an identifier.
    pub fn raw(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
        }
    }

    /// A statement with bound parameters.
    pub fn new(sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }

    /// Append one parameter.
    pub fn bind(mut self, value: impl Into<Value>) -> Self {
        self.params.push(value.into());
        self
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// The bound parameters, in order.
    ///
    /// Mirrors [`crate::sql::Upsert::params`]: a caller that must re-render a
    /// statement under another framing — such as `EXPLAIN QUERY PLAN`, which
    /// plans the query shape and binds the same holes — needs them to re-run
    /// the same `WHERE` without reconstructing it by hand.
    #[must_use]
    pub fn params(&self) -> &[Value] {
        &self.params
    }

    fn query(&self) -> Query<'_, Any, AnyArguments<'_>> {
        let mut query = sqlx::query(&self.sql);
        for value in &self.params {
            query = bind_value(query, value);
        }
        query
    }
}

impl From<Upsert> for Stmt {
    fn from(upsert: Upsert) -> Self {
        let (sql, params) = upsert.into_parts();
        Self { sql, params }
    }
}

/// Bind a framework [`Value`] as a query parameter.
///
/// `Any` is type-erased, so a value is bound at the closest concrete type the
/// JSON shape allows; anything richer is sent as JSON text, which is how the
/// document blob is stored.
fn bind_value<'q>(
    query: Query<'q, Any, AnyArguments<'q>>,
    value: &Value,
) -> Query<'q, Any, AnyArguments<'q>> {
    match value {
        Value::Null => query.bind(Option::<String>::None),
        Value::Bool(b) => query.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                query.bind(i)
            } else if let Some(f) = n.as_f64() {
                query.bind(f)
            } else {
                query.bind(n.to_string())
            }
        }
        Value::String(s) => query.bind(s.clone()),
        // `Any` cannot carry a JSON value, so it is stored as text.
        Value::Array(_) | Value::Object(_) => query.bind(value.to_json().to_string()),
    }
}

/// Rows affected by a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affected(pub u64);

impl Affected {
    pub fn rows(&self) -> u64 {
        self.0
    }

    /// Whether the statement matched nothing.
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

/// A connection pool plus the dialect it speaks.
#[derive(Clone)]
pub struct Db {
    pool: AnyPool,
    dialect: Dialect,
    /// Process-unique identity, shared by every clone of this handle.
    ///
    /// Caches that must be initialised once per *database* (not once per
    /// `Db` value, which is rebuilt per request) key on this value. It is an
    /// opaque counter, not a connection string: the pool no longer exposes the
    /// URL, and a pointer would be invalid after the last handle drops.
    id: u64,
}

/// Source of [`Db::id`] values.
static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("dialect", &self.dialect)
            .field("pool", &self.pool)
            .finish()
    }
}

impl Db {
    /// Wrap an existing pool, taking the dialect from `config`.
    ///
    /// The dialect cannot be read back off a built pool, so it comes from the
    /// configuration that produced it. Guessing from a URL the pool no longer
    /// exposes would silently emit SQL for the wrong backend.
    pub fn new(pool: AnyPool, config: &ferris_core::DatabaseConfig) -> Self {
        let dialect = Dialect::from_database_type(config.r#type);
        Self {
            pool,
            dialect,
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Wrap a pool with an explicitly known dialect.
    pub fn with_dialect(pool: AnyPool, dialect: Dialect) -> Self {
        Self {
            pool,
            dialect,
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Process-unique identity for this database handle.
    ///
    /// Every clone returns the same value; two separately opened pools — even
    /// against the same file — get different values, which is what a per-DB
    /// initialisation cache needs.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Open a pool from configuration and make sure it can be used.
    ///
    /// This is the single entry point for creating a connection; it applies the
    /// SSL mode, creates the SQLite file if missing and installs the
    /// migration bookkeeping table.
    pub async fn connect(config: &ferris_core::DatabaseConfig) -> Result<Self> {
        crate::pool::validate(config)?;
        Ok(Self::new(open_pool(config).await?, config))
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The underlying pool, for the few callers that must hand it on.
    ///
    /// Prefer the methods on this type: reaching past it is how SQL escaped
    /// the abstraction in the first place.
    pub fn pool(&self) -> &AnyPool {
        &self.pool
    }

    /// Take the underlying pool, for code that has to pass it to a driver
    /// ferris-db does not wrap.
    pub fn into_pool(self) -> AnyPool {
        self.pool
    }

    /// Borrow one connection from the pool.
    ///
    /// The migration runner needs this: a session-scoped advisory lock is only
    /// meaningful if the lock, the migrations and the release all run on the
    /// *same* session, and every method above takes whatever connection the
    /// pool hands out. Holding a `PoolConnection` pins one session for as long
    /// as the guard lives.
    pub(crate) async fn acquire(&self) -> Result<sqlx::pool::PoolConnection<Any>> {
        self.pool.acquire().await.map_err(error::map)
    }

    /// Run a statement, returning how many rows it changed.
    pub async fn execute(&self, stmt: &Stmt) -> Result<Affected> {
        stmt.query()
            .execute(&self.pool)
            .await
            .map(|r| Affected(r.rows_affected()))
            .map_err(error::map)
    }

    /// Run a multi-statement script.
    ///
    /// Dialect-aware DDL needs this: a table plus its indexes is more than one
    /// statement.
    pub async fn execute_script(&self, sql: &str) -> Result<()> {
        sqlx::raw_sql(sql)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(error::map)
    }

    /// Fetch every row.
    pub async fn fetch_all(&self, stmt: &Stmt) -> Result<Vec<Row>> {
        let rows = stmt
            .query()
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
        Ok(rows.into_iter().map(Row::new).collect())
    }

    /// Fetch the first row, or `None` when there is none.
    pub async fn fetch_optional(&self, stmt: &Stmt) -> Result<Option<Row>> {
        let row = stmt.query().fetch_optional(&self.pool).await;
        crate::row::optional(row).map(|r| r.map(Row::new))
    }

    /// Fetch exactly one row, erroring when there is none.
    pub async fn fetch_one(&self, stmt: &Stmt) -> Result<Row> {
        let row = stmt
            .query()
            .fetch_one(&self.pool)
            .await
            .map_err(error::map)?;
        Ok(Row::new(row))
    }

    /// Fetch a single scalar from the first column of the first row.
    pub async fn fetch_scalar<T>(&self, stmt: &Stmt) -> Result<Option<T>>
    where
        for<'r> T: sqlx::Decode<'r, Any> + sqlx::Type<Any>,
    {
        crate::row::optional(stmt.query().fetch_optional(&self.pool).await)
            .map(|row| row.and_then(|r| r.try_get::<T, _>(0).ok()))
    }

    /// How many rows `stmt` matches.
    ///
    /// `stmt` is wrapped in a sub-select so `GROUP BY` still counts groups
    /// rather than collapsing to one number.
    ///
    /// The statement must be **unpaged**. A `LIMIT` inside it stays inside the
    /// sub-select, so this returns the page size while looking like a total —
    /// pass the same `SELECT` without its `LIMIT`/`OFFSET`.
    pub async fn count(&self, stmt: &Stmt) -> Result<u64> {
        let counting = count_stmt(stmt);
        let row = self.fetch_one(&counting).await?;
        Ok(row.count("count"))
    }

    /// Whether a ping succeeds.
    pub async fn health_check(&self) -> Result<()> {
        self.execute(&Stmt::raw("SELECT 1")).await.map(|_| ())
    }

    /// Close the pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    // --- schema helpers -----------------------------------------------------
    // These exist so no caller writes `CREATE TABLE` by hand: the dialect
    // differences (MySQL's `TEXT DEFAULT`, its missing `CREATE INDEX IF NOT
    // EXISTS`) are handled once, here.

    /// Create `table` if absent.
    pub async fn ensure_table(&self, table: &str, columns: &[crate::sql::Column]) -> Result<()> {
        let sql = crate::sql::create_table(self.dialect, table, columns)?;
        self.execute(&Stmt::raw(sql)).await.map(|_| ())
    }

    /// Whether `table` exists.
    ///
    /// The related-table cascade (`DocumentEngine::delete`, ) must
    /// delete from side tables that are created lazily — a deployment that
    /// never shared or commented has no `tab_share` to clean. Each backend
    /// keeps its catalogue in a different place, so the dialect decides which
    /// query answers it.
    pub async fn table_exists(&self, table: &str) -> Result<bool> {
        let sql = match self.dialect {
            Dialect::SQLite => "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
            Dialect::MySQL => {
                "SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_name = ?"
            }
            Dialect::Postgres => {
                "SELECT 1 FROM pg_catalog.pg_tables \
                 WHERE schemaname = current_schema() AND tablename = ?"
            }
        };
        let row = self
            .fetch_optional(&Stmt::new(sql, vec![Value::from(table)]))
            .await?;
        Ok(row.is_some())
    }

    /// Create index `name` if absent.
    ///
    /// Tolerates a duplicate-name error, which is what makes it idempotent on
    /// MySQL where the statement cannot express "if not exists".
    pub async fn ensure_index(
        &self,
        name: &str,
        table: &str,
        columns: &[&str],
        unique: bool,
    ) -> Result<()> {
        let sql = crate::sql::ensure_index(self.dialect, name, table, columns, unique)?;
        match self.execute(&Stmt::raw(sql)).await {
            Ok(_) => Ok(()),
            Err(e) if is_duplicate_index(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Index the exact value of the document field `field`, if absent.
    ///
    /// The statement(s) come from [`crate::sql::ensure_json_index`]: one
    /// idempotent `CREATE INDEX` on SQLite/Postgres, and on MySQL an
    /// `ADD COLUMN` (the generated mirror column) plus a `CREATE INDEX`. Both
    /// tolerate their duplicate errors individually, which is what makes
    /// re-running this across boots and processes a no-op.
    pub async fn ensure_json_index(
        &self,
        name: &str,
        table: &str,
        field: &str,
        unique: bool,
    ) -> Result<()> {
        for sql in crate::sql::ensure_json_index(self.dialect, name, table, field, unique)? {
            match self.execute(&Stmt::raw(sql)).await {
                Ok(_) => {}
                Err(e) if is_duplicate_index(&e) || is_duplicate_column(&e) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Add a column to an existing table if absent.
    ///
    /// `ensure_table` only creates; loading an operator document gives a table
    /// its schema once, but a framework column added later (e.g. the MFA
    /// columns on `users`) still needs a migration that existing databases can
    /// run. There is no portable `ADD COLUMN IF NOT EXISTS`, so this tolerates
    /// the duplicate-column error, which is what makes it idempotent.
    ///
    /// # Errors
    /// Propagates the query error unless it is the duplicate-column case, which
    /// is consumed and reported as success.
    #[allow(clippy::result_large_err)] // the crate-wide `Error` type is intentionally unchanged
    pub async fn ensure_column(&self, table: &str, column: &crate::sql::Column) -> Result<()> {
        let sql = crate::sql::add_column(self.dialect, table, column)?;
        match self.execute(&Stmt::raw(sql)).await {
            Ok(_) => Ok(()),
            Err(e) if is_duplicate_column(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Run a portable upsert.
    pub async fn upsert(&self, upsert: Upsert) -> Result<Affected> {
        self.execute(&upsert.into()).await
    }

    // --- transactions -------------------------------------------------------

    /// Begin a transaction.
    pub async fn begin(&self) -> Result<Tx> {
        let inner = self.pool.begin().await.map_err(|e| {
            Error::Database(ferris_core::error::DatabaseError::Transaction(
                e.to_string(),
            ))
        })?;
        Ok(Tx::new(inner, self.dialect))
    }
}

/// The statement that counts the rows `stmt` matches.
///
/// `stmt` is wrapped in a sub-select so `GROUP BY` still counts groups rather
/// than collapsing to one number. Shared by `Db::count` and `Tx::count` so the
/// two cannot drift.
pub(crate) fn count_stmt(stmt: &Stmt) -> Stmt {
    Stmt::new(
        format!(
            "SELECT COUNT(*) AS count FROM ({}) AS ferris_count",
            stmt.sql()
        ),
        stmt.params.clone(),
    )
}

/// Run a multi-statement script on a specific pooled connection.
///
/// [`Db::execute_script`] takes whatever connection the pool hands out; the
/// migration runner needs one it has pinned, so the advisory lock it acquires
/// and releases on the same session is meaningful. This is the pinned-session
/// counterpart.
pub(crate) async fn execute_script_on(conn: &mut sqlx::AnyConnection, sql: &str) -> Result<()> {
    sqlx::raw_sql(sql)
        .execute(&mut *conn)
        .await
        .map(|_| ())
        .map_err(error::map)
}

/// Run a bound statement on a specific pooled connection.
///
/// The pinned-session counterpart of [`Db::execute`], used by the migration
/// runner so its bookkeeping `INSERT` lands on the same connection as the DDL
/// it records.
pub(crate) async fn execute_stmt_on(
    conn: &mut sqlx::AnyConnection,
    stmt: &Stmt,
) -> Result<Affected> {
    stmt.query()
        .execute(&mut *conn)
        .await
        .map(|r| Affected(r.rows_affected()))
        .map_err(error::map)
}

/// Fetch every row of a bound statement on a specific pooled connection.
///
/// The pinned-session counterpart of [`Db::fetch_all`], so the migration
/// runner reads the applied set on the same connection it holds the lock on.
pub(crate) async fn fetch_all_on(conn: &mut sqlx::AnyConnection, stmt: &Stmt) -> Result<Vec<Row>> {
    let rows = stmt
        .query()
        .fetch_all(&mut *conn)
        .await
        .map_err(error::map)?;
    Ok(rows.into_iter().map(Row::new).collect())
}

/// Whether the error is "that index already exists".
fn is_duplicate_index(error: &Error) -> bool {
    let Error::Database(db) = error else {
        return false;
    };
    let message = db.to_string().to_ascii_lowercase();
    message.contains("already exists")
        || message.contains("duplicate key name")
        || message.contains("duplicate index")
}

/// Whether the error is "that column already exists".
///
/// The spellings differ per backend: SQLite `duplicate column name`,
/// MySQL `duplicate column name`, Postgres `column "x" ... already exists`.
fn is_duplicate_column(error: &Error) -> bool {
    let Error::Database(db) = error else {
        return false;
    };
    let message = db.to_string().to_ascii_lowercase();
    message.contains("duplicate column")
        || message.contains("already exists")
        || message.contains("duplicate column name")
        || message.contains("has a column named")
}

/// Build a pool without touching migrations, for callers that only need a
/// connection.
pub(crate) async fn open_pool(config: &ferris_core::DatabaseConfig) -> Result<AnyPool> {
    sqlx::any::install_default_drivers();
    let url = crate::pool::database_url(config);
    let pool = AnyPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(std::time::Duration::from_secs(config.connect_timeout_secs))
        .idle_timeout(std::time::Duration::from_secs(config.idle_timeout_secs))
        .max_lifetime(std::time::Duration::from_secs(config.max_lifetime_secs))
        .connect(&url)
        .await
        .map_err(|e| {
            Error::Database(ferris_core::error::DatabaseError::Connection(e.to_string()))
        })?;
    Ok(pool)
}

/// Column names of a row, in order.
pub fn column_names(row: &AnyRow) -> Vec<&str> {
    row.columns().iter().map(Column::name).collect()
}

/// SHA-256 of `input`, lowercase hexadecimal.
///
/// The hash-chain primitive the document layer needs for `tab_version`; it
/// lives here because `sha2` is already a dependency of this crate and adding it
/// to `ferris-document` for one call would widen that crate's surface. The
/// migration checksum ([`crate::migration::calculate_checksum`]) is the same
/// digest over a different input.
#[must_use]
pub fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}
