//! Concrete per-driver plumbing.
//!
//! `sqlx::Any` erased the backend, and with it the native column types: a
//! `DATE`, `DECIMAL`, `BOOLEAN` or `UUID` column could be written by the engine
//! but never decoded through the facade, because `Any` only speaks a handful of
//! lowered types. The enums here keep the pool, the row and the pinned
//! connection at their concrete type, so binding and decoding reach the driver
//! the database actually speaks.
//!
//! Everything above this module still works in terms of [`Db`](crate::Db),
//! [`Stmt`], [`Row`] and [`Tx`](crate::Tx); the concrete types stay private to
//! the connection, the row wrapper and this module.

// Every function here is crate-internal and the crate-wide `Error` type is
// intentionally unchanged, so the two derived lints are noise, not signal.
#![allow(clippy::redundant_pub_crate, clippy::result_large_err)]

use crate::error;
use crate::exec::{Affected, Stmt};
use crate::row::{DbRow, Row};
use ferris_core::{DatabaseConfig, DatabaseType, Error, Result, Value};
use sqlx::pool::PoolConnection;
use sqlx::query::Query;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{Database, Executor, MySql, Postgres, Sqlite};
use std::str::FromStr;

/// A connection pool for one concrete backend.
///
/// The variant is chosen from [`DatabaseType`] when the pool is opened; the
/// dialect used to spell SQL still lives on [`crate::Db`], next to it.
#[derive(Clone, Debug)]
pub enum DbPool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
}

impl DbPool {
    /// Close the pool, waiting for in-flight connections.
    pub(crate) async fn close(&self) {
        match self {
            Self::Sqlite(pool) => pool.close().await,
            Self::Postgres(pool) => pool.close().await,
            Self::MySql(pool) => pool.close().await,
        }
    }

    /// Borrow one connection, pinning its session.
    ///
    /// The migration runner needs a stable session: an advisory lock taken on
    /// one pooled connection is released on that same connection, so the guard
    /// has to be held for the whole batch.
    pub(crate) async fn acquire(&self) -> Result<PinnedConnection> {
        match self {
            Self::Sqlite(pool) => Ok(PinnedConnection::Sqlite(
                pool.acquire().await.map_err(error::map)?,
            )),
            Self::Postgres(pool) => Ok(PinnedConnection::Postgres(
                pool.acquire().await.map_err(error::map)?,
            )),
            Self::MySql(pool) => Ok(PinnedConnection::MySql(
                pool.acquire().await.map_err(error::map)?,
            )),
        }
    }
}

/// A pooled connection held for a session-scoped operation.
pub(crate) enum PinnedConnection {
    Sqlite(PoolConnection<Sqlite>),
    Postgres(PoolConnection<Postgres>),
    MySql(PoolConnection<MySql>),
}

impl PinnedConnection {
    /// Run a bound statement on this session.
    pub(crate) async fn execute(&mut self, stmt: &Stmt) -> Result<Affected> {
        match self {
            Self::Sqlite(conn) => execute::<Sqlite, _>(&mut **conn, stmt).await,
            Self::Postgres(conn) => execute::<Postgres, _>(&mut **conn, stmt).await,
            Self::MySql(conn) => execute::<MySql, _>(&mut **conn, stmt).await,
        }
    }

    /// Run a multi-statement script on this session.
    pub(crate) async fn script(&mut self, sql: &str) -> Result<()> {
        match self {
            Self::Sqlite(conn) => script::<Sqlite, _>(&mut **conn, sql).await,
            Self::Postgres(conn) => script::<Postgres, _>(&mut **conn, sql).await,
            Self::MySql(conn) => script::<MySql, _>(&mut **conn, sql).await,
        }
    }

    /// Fetch every row of a bound statement on this session.
    pub(crate) async fn fetch_all(&mut self, stmt: &Stmt) -> Result<Vec<Row>> {
        match self {
            Self::Sqlite(conn) => fetch_all::<Sqlite, _>(&mut **conn, stmt).await,
            Self::Postgres(conn) => fetch_all::<Postgres, _>(&mut **conn, stmt).await,
            Self::MySql(conn) => fetch_all::<MySql, _>(&mut **conn, stmt).await,
        }
    }
}

/// Per-backend operations the framework needs from a row.
///
/// A `sqlx::Database` already knows its row type and its argument buffer; these
/// methods are the only backend-specific knowledge left: how to bind a
/// framework [`Value`], how many rows a statement changed, how to read a
/// scalar, and how to wrap a decoded row.
pub(crate) trait Driver: Database {
    /// Bind one framework value as a query parameter.
    fn bind<'q>(
        query: Query<'q, Self, Self::Arguments<'q>>,
        value: &Value,
    ) -> Query<'q, Self, Self::Arguments<'q>>;

    /// Rows changed by a statement.
    ///
    /// `rows_affected` is an inherent method on each concrete query result, not
    /// on a shared trait, so the backend has to spell it out.
    fn rows_affected(result: Self::QueryResult) -> u64;

    /// Decode the column at `index` from a driver row, answering `None` for
    /// SQL `NULL`.
    fn scalar<T>(row: &Self::Row, index: usize) -> std::result::Result<Option<T>, sqlx::Error>
    where
        for<'r> T: sqlx::Decode<'r, Self> + sqlx::Type<Self>;

    /// Wrap a driver row in the framework's row enum.
    fn wrap_row(row: Self::Row) -> DbRow;
}

/// Bind one value at the closest concrete type the JSON shape allows.
///
/// `Null` is bound as a typed `NULL`; the engine only writes non-null values to
/// typed columns, so the text-shaped null is enough for the untyped ones.
macro_rules! bind_value {
    ($query:expr, $value:expr) => {{
        let query = $query;
        match $value {
            Value::Null => query.bind(Option::<String>::None),
            Value::Bool(value) => query.bind(*value),
            Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    query.bind(value)
                } else if let Some(value) = number.as_f64() {
                    query.bind(value)
                } else {
                    query.bind(number.to_string())
                }
            }
            Value::String(value) => query.bind(value.clone()),
            // The JSON column is text on every backend this framework targets.
            Value::Array(_) | Value::Object(_) => query.bind($value.to_json().to_string()),
        }
    }};
}

macro_rules! impl_driver {
    ($database:ty, $variant:ident) => {
        impl Driver for $database {
            fn bind<'q>(
                query: Query<'q, Self, Self::Arguments<'q>>,
                value: &Value,
            ) -> Query<'q, Self, Self::Arguments<'q>> {
                bind_value!(query, value)
            }

            fn rows_affected(result: Self::QueryResult) -> u64 {
                result.rows_affected()
            }

            fn scalar<T>(
                row: &Self::Row,
                index: usize,
            ) -> std::result::Result<Option<T>, sqlx::Error>
            where
                for<'r> T: sqlx::Decode<'r, Self> + sqlx::Type<Self>,
            {
                use sqlx::{Row as _, ValueRef as _};

                if row.try_get_raw(index)?.is_null() {
                    return Ok(None);
                }
                row.try_get(index).map(Some)
            }

            fn wrap_row(row: Self::Row) -> DbRow {
                DbRow::$variant(row)
            }
        }
    };
}

impl_driver!(Sqlite, Sqlite);
impl_driver!(Postgres, Postgres);
impl_driver!(MySql, MySql);

/// Build the driver query for `stmt`, binding every parameter.
fn prepared<'q, DB: Driver>(stmt: &'q Stmt) -> Query<'q, DB, DB::Arguments<'q>>
where
    DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let mut query = sqlx::query::<DB>(stmt.sql());
    for value in stmt.params() {
        query = DB::bind(query, value);
    }
    query
}

/// Run a bound statement.
pub(crate) async fn execute<'e, DB, E>(executor: E, stmt: &Stmt) -> Result<Affected>
where
    DB: Driver,
    E: Executor<'e, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    prepared::<DB>(stmt)
        .execute(executor)
        .await
        .map(|result| Affected(DB::rows_affected(result)))
        .map_err(error::map)
}

/// Run a multi-statement script.
pub(crate) async fn script<'e, DB, E>(executor: E, sql: &'e str) -> Result<()>
where
    DB: Database,
    E: Executor<'e, Database = DB>,
{
    sqlx::raw_sql(sql)
        .execute(executor)
        .await
        .map(|_| ())
        .map_err(error::map)
}

/// Fetch every row.
pub(crate) async fn fetch_all<'e, DB, E>(executor: E, stmt: &Stmt) -> Result<Vec<Row>>
where
    DB: Driver,
    E: Executor<'e, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let rows = prepared::<DB>(stmt)
        .fetch_all(executor)
        .await
        .map_err(error::map)?;
    Ok(rows
        .into_iter()
        .map(|row| Row::new(DB::wrap_row(row)))
        .collect())
}

/// Fetch the first row, or `None` when there is none.
pub(crate) async fn fetch_optional<'e, DB, E>(executor: E, stmt: &Stmt) -> Result<Option<Row>>
where
    DB: Driver,
    E: Executor<'e, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let row = prepared::<DB>(stmt)
        .fetch_optional(executor)
        .await
        .map_err(error::map)?;
    Ok(row.map(|row| Row::new(DB::wrap_row(row))))
}

/// Fetch exactly one row, erroring when there is none.
pub(crate) async fn fetch_one<'e, DB, E>(executor: E, stmt: &Stmt) -> Result<Row>
where
    DB: Driver,
    E: Executor<'e, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let row = prepared::<DB>(stmt)
        .fetch_one(executor)
        .await
        .map_err(error::map)?;
    Ok(Row::new(DB::wrap_row(row)))
}

/// Fetch a single scalar from the first column of the first row.
pub(crate) async fn fetch_scalar<'e, DB, E, T>(executor: E, stmt: &Stmt) -> Result<Option<T>>
where
    DB: Driver,
    E: Executor<'e, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    for<'r> T: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
{
    let row = prepared::<DB>(stmt)
        .fetch_optional(executor)
        .await
        .map_err(error::map)?;
    Ok(row.and_then(|row| DB::scalar::<T>(&row, 0).ok().flatten()))
}

/// Open the concrete pool for `config`.
///
/// The SSL mode, the SQLite file creation and the connection settings are the
/// same for every backend; only the driver and therefore the pool type differ.
pub(crate) async fn open_pool(config: &DatabaseConfig) -> Result<DbPool> {
    let url = crate::pool::database_url(config);
    match config.r#type {
        DatabaseType::SQLite => {
            // WAL lets readers and the writer coexist; `NORMAL` removes the
            // per-commit fsync without risking corruption under WAL. The cache
            // and mmap sizes are per connection and SQLite otherwise keeps a
            // small 2 MiB page cache, which shows up on every list query.
            let connect_options = SqliteConnectOptions::from_str(&url)
                .map_err(connection_error)?
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(std::time::Duration::from_secs(5))
                .foreign_keys(true)
                .pragma("cache_size", "-16000")
                .pragma("mmap_size", "268435456")
                .pragma("temp_store", "MEMORY")
                .pragma("query_only", "OFF");
            // SQLite has a single writer; more than a handful of connections
            // only multiplies the per-connection page cache. The server's
            // default is 20, which at 16 MiB each would be 320 MiB of cache
            // alone. Eight is enough for concurrent readers under WAL.
            let max_connections = config.max_connections.min(8);
            let min_connections = config.min_connections.min(max_connections);
            Ok(DbPool::Sqlite(
                options::<Sqlite>(config)
                    .max_connections(max_connections)
                    .min_connections(min_connections)
                    .connect_with(connect_options)
                    .await
                    .map_err(connection_error)?,
            ))
        }
        DatabaseType::Postgres => Ok(DbPool::Postgres(
            options::<Postgres>(config)
                .connect(&url)
                .await
                .map_err(connection_error)?,
        )),
        DatabaseType::MySQL => Ok(DbPool::MySql(
            options::<MySql>(config)
                .connect(&url)
                .await
                .map_err(connection_error)?,
        )),
    }
}

/// The pool settings shared by the three drivers.
fn options<DB: Database>(config: &DatabaseConfig) -> sqlx::pool::PoolOptions<DB> {
    sqlx::pool::PoolOptions::<DB>::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(std::time::Duration::from_secs(config.connect_timeout_secs))
        .idle_timeout(std::time::Duration::from_secs(config.idle_timeout_secs))
        .max_lifetime(std::time::Duration::from_secs(config.max_lifetime_secs))
}

/// A failed initial connection is a connection error, not a query error.
#[allow(clippy::needless_pass_by_value)] // `map_err` hands the error over and it is only stringified
fn connection_error(error: sqlx::Error) -> Error {
    Error::Database(ferris_core::error::DatabaseError::Connection(
        error.to_string(),
    ))
}
