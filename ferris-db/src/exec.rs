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
//! the concrete row [`Row::new`] accepts. Those three signatures name
//! [`DbPool`] and [`crate::row::DbRow`], and that is the whole set.

mod tx;

#[cfg(test)]
mod tests;

pub use tx::Tx;

use crate::dialect::Dialect;
use crate::driver::{self, DbPool, PinnedConnection};
use crate::row::{DbRow, Row};
use crate::sql::Upsert;
use ferris_core::{Error, Result, Value};
use sqlx::{Decode, MySql, Postgres, Sqlite, Type};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};

/// A statement bound for execution.
#[derive(Clone)]
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

    /// A statement whose `?` holes are spelled for `dialect`.
    ///
    /// Postgres takes `$1`, `$2`, …; the rendering allocates only for it. A
    /// statement with no parameters is borrowed untouched, because a `?` there
    /// is a literal rather than a hole.
    pub(crate) fn for_dialect(&self, dialect: Dialect) -> Cow<'_, Stmt> {
        if self.params.is_empty() {
            return Cow::Borrowed(self);
        }
        match dialect.render_placeholders(&self.sql) {
            Cow::Borrowed(_) => Cow::Borrowed(self),
            Cow::Owned(sql) => Cow::Owned(Self {
                sql,
                params: self.params.clone(),
            }),
        }
    }
}

impl From<Upsert> for Stmt {
    fn from(upsert: Upsert) -> Self {
        let (sql, params) = upsert.into_parts();
        Self { sql, params }
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
    pool: DbPool,
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
    pub fn new(pool: DbPool, config: &ferris_core::DatabaseConfig) -> Self {
        let dialect = Dialect::from_database_type(config.r#type);
        Self {
            pool,
            dialect,
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Wrap a pool with an explicitly known dialect.
    pub fn with_dialect(pool: DbPool, dialect: Dialect) -> Self {
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
    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    /// Take the underlying pool, for code that has to pass it to a driver
    /// ferris-db does not wrap.
    pub fn into_pool(self) -> DbPool {
        self.pool
    }

    /// Borrow one connection from the pool.
    ///
    /// The migration runner needs this: a session-scoped advisory lock is only
    /// meaningful if the lock, the migrations and the release all run on the
    /// *same* session, and every method above takes whatever connection the
    /// pool hands out. Holding a [`PinnedConnection`] pins one session for as
    /// long as the guard lives.
    pub(crate) async fn acquire(&self) -> Result<PinnedConnection> {
        self.pool.acquire().await
    }

    /// Run a statement, returning how many rows it changed.
    pub async fn execute(&self, stmt: &Stmt) -> Result<Affected> {
        let stmt = stmt.for_dialect(self.dialect);
        match &self.pool {
            DbPool::Sqlite(pool) => driver::execute::<Sqlite, _>(pool, stmt.as_ref()).await,
            DbPool::Postgres(pool) => driver::execute::<Postgres, _>(pool, stmt.as_ref()).await,
            DbPool::MySql(pool) => driver::execute::<MySql, _>(pool, stmt.as_ref()).await,
        }
    }

    /// Run a DDL script, tolerating an index/column that already exists.
    ///
    /// MySQL has no `CREATE INDEX IF NOT EXISTS`; the lazy table path re-runs
    /// the generated DDL against an existing table, so a duplicate index is the
    /// normal idempotent case, not an error. Migrations use the strict
    /// [`Self::execute_script`] instead.
    pub async fn execute_script_tolerant(&self, sql: &str) -> Result<()> {
        match self.execute_script(sql).await {
            Ok(()) => Ok(()),
            Err(error) if is_duplicate_index(&error) || is_duplicate_column(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Run a multi-statement script.
    ///
    /// Dialect-aware DDL needs this: a table plus its indexes is more than one
    /// statement.
    pub async fn execute_script(&self, sql: &str) -> Result<()> {
        match &self.pool {
            DbPool::Sqlite(pool) => driver::script::<Sqlite, _>(pool, sql).await,
            DbPool::Postgres(pool) => driver::script::<Postgres, _>(pool, sql).await,
            DbPool::MySql(pool) => driver::script::<MySql, _>(pool, sql).await,
        }
    }

    /// Fetch every row.
    pub async fn fetch_all(&self, stmt: &Stmt) -> Result<Vec<Row>> {
        let stmt = stmt.for_dialect(self.dialect);
        match &self.pool {
            DbPool::Sqlite(pool) => driver::fetch_all::<Sqlite, _>(pool, stmt.as_ref()).await,
            DbPool::Postgres(pool) => driver::fetch_all::<Postgres, _>(pool, stmt.as_ref()).await,
            DbPool::MySql(pool) => driver::fetch_all::<MySql, _>(pool, stmt.as_ref()).await,
        }
    }

    /// Fetch the first row, or `None` when there is none.
    pub async fn fetch_optional(&self, stmt: &Stmt) -> Result<Option<Row>> {
        let stmt = stmt.for_dialect(self.dialect);
        match &self.pool {
            DbPool::Sqlite(pool) => driver::fetch_optional::<Sqlite, _>(pool, stmt.as_ref()).await,
            DbPool::Postgres(pool) => {
                driver::fetch_optional::<Postgres, _>(pool, stmt.as_ref()).await
            }
            DbPool::MySql(pool) => driver::fetch_optional::<MySql, _>(pool, stmt.as_ref()).await,
        }
    }

    /// Fetch exactly one row, erroring when there is none.
    pub async fn fetch_one(&self, stmt: &Stmt) -> Result<Row> {
        let stmt = stmt.for_dialect(self.dialect);
        match &self.pool {
            DbPool::Sqlite(pool) => driver::fetch_one::<Sqlite, _>(pool, stmt.as_ref()).await,
            DbPool::Postgres(pool) => driver::fetch_one::<Postgres, _>(pool, stmt.as_ref()).await,
            DbPool::MySql(pool) => driver::fetch_one::<MySql, _>(pool, stmt.as_ref()).await,
        }
    }

    /// Fetch a single scalar from the first column of the first row.
    pub async fn fetch_scalar<T>(&self, stmt: &Stmt) -> Result<Option<T>>
    where
        for<'r> T: Decode<'r, Sqlite>
            + Type<Sqlite>
            + Decode<'r, Postgres>
            + Type<Postgres>
            + Decode<'r, MySql>
            + Type<MySql>,
    {
        let stmt = stmt.for_dialect(self.dialect);
        match &self.pool {
            DbPool::Sqlite(pool) => driver::fetch_scalar::<Sqlite, _, T>(pool, stmt.as_ref()).await,
            DbPool::Postgres(pool) => {
                driver::fetch_scalar::<Postgres, _, T>(pool, stmt.as_ref()).await
            }
            DbPool::MySql(pool) => driver::fetch_scalar::<MySql, _, T>(pool, stmt.as_ref()).await,
        }
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
        Tx::begin(&self.pool).await
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
pub(crate) async fn execute_script_on(conn: &mut PinnedConnection, sql: &str) -> Result<()> {
    conn.script(sql).await
}

/// Run a bound statement on a specific pooled connection.
///
/// The pinned-session counterpart of [`Db::execute`], used by the migration
/// runner so its bookkeeping `INSERT` lands on the same connection as the DDL
/// it records.
pub(crate) async fn execute_stmt_on(
    conn: &mut PinnedConnection,
    stmt: &Stmt,
    dialect: Dialect,
) -> Result<Affected> {
    let stmt = stmt.for_dialect(dialect);
    conn.execute(stmt.as_ref()).await
}

/// Fetch every row of a bound statement on a specific pooled connection.
///
/// The pinned-session counterpart of [`Db::fetch_all`], so the migration
/// runner reads the applied set on the same connection it holds the lock on.
pub(crate) async fn fetch_all_on(
    conn: &mut PinnedConnection,
    stmt: &Stmt,
    dialect: Dialect,
) -> Result<Vec<Row>> {
    let stmt = stmt.for_dialect(dialect);
    conn.fetch_all(stmt.as_ref()).await
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
pub(crate) async fn open_pool(config: &ferris_core::DatabaseConfig) -> Result<DbPool> {
    driver::open_pool(config).await
}

/// Column names of a row, in order.
pub fn column_names(row: &DbRow) -> Vec<&str> {
    row.columns()
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
