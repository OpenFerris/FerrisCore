//! The transaction handle.
//!
//! Split out of `exec.rs` so that file holds one concern (the pooled
//! connection) and this one holds another (a transaction on it). `Tx` is
//! re-exported from [`crate::exec`] so every existing `crate::exec::Tx` path
//! still resolves.
//!
//! The open transaction is kept in a private per-driver enum. The public shape
//! is still the plain `Tx` handle: `commit` and `rollback` take `&mut self` and
//! flip the handle to finished, which is only sound if the finished state
//! cannot be constructed from outside — hence the private enum behind the
//! `Option`, rather than `Option<Transaction<…>>` in a public variant.

use crate::dialect::Dialect;
use crate::driver::{self, DbPool};
use crate::exec::{Affected, Stmt};
use crate::row::Row;
use ferris_core::{DatabaseError, Error, Result};
use sqlx::{MySql, Postgres, Sqlite, Transaction};

/// The open driver transaction, per backend.
enum TxInner {
    Sqlite(Transaction<'static, Sqlite>),
    Postgres(Transaction<'static, Postgres>),
    MySql(Transaction<'static, MySql>),
}

/// An open transaction.
///
/// Dropping it without committing rolls back: `sqlx` queues the rollback on
/// the connection, so releasing the handle is enough and there is nothing
/// useful to do from `Drop`.
pub struct Tx {
    inner: Option<TxInner>,
    dialect: Dialect,
}

// `inner` is deliberately absent: a `sqlx::Transaction` holds a connection
// whose debug output is a pointer, which tells a reader nothing.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for Tx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tx")
            .field("dialect", &self.dialect)
            .field("finished", &self.is_finished())
            .finish()
    }
}

fn completed() -> Error {
    Error::Database(DatabaseError::Transaction(
        "Transaction already completed".into(),
    ))
}

impl Tx {
    /// Begin a transaction on `pool`, carrying the dialect of its backend.
    pub(crate) async fn begin(pool: &DbPool) -> Result<Self> {
        match pool {
            DbPool::Sqlite(pool) => Ok(Self::sqlite(
                pool.begin()
                    .await
                    .map_err(|error| transaction_error(&error))?,
            )),
            DbPool::Postgres(pool) => Ok(Self::postgres(
                pool.begin()
                    .await
                    .map_err(|error| transaction_error(&error))?,
            )),
            DbPool::MySql(pool) => Ok(Self::mysql(
                pool.begin()
                    .await
                    .map_err(|error| transaction_error(&error))?,
            )),
        }
    }

    fn sqlite(inner: Transaction<'static, Sqlite>) -> Self {
        Self {
            inner: Some(TxInner::Sqlite(inner)),
            dialect: Dialect::SQLite,
        }
    }

    fn postgres(inner: Transaction<'static, Postgres>) -> Self {
        Self {
            inner: Some(TxInner::Postgres(inner)),
            dialect: Dialect::Postgres,
        }
    }

    fn mysql(inner: Transaction<'static, MySql>) -> Self {
        Self {
            inner: Some(TxInner::MySql(inner)),
            dialect: Dialect::MySQL,
        }
    }

    /// The open transaction, or the "already completed" error.
    ///
    /// `sqlx::Transaction` implements `Deref` to its connection, and the
    /// `Executor` bound is on that connection rather than on the transaction,
    /// so every statement goes through the dereference at the call site.
    fn inner_mut(&mut self) -> Result<&mut TxInner> {
        self.inner.as_mut().ok_or_else(completed)
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Whether this transaction has been committed or rolled back.
    pub fn is_finished(&self) -> bool {
        self.inner.is_none()
    }

    pub async fn execute(&mut self, stmt: &Stmt) -> Result<Affected> {
        let stmt = stmt.for_dialect(self.dialect);
        match self.inner_mut()? {
            TxInner::Sqlite(tx) => driver::execute::<Sqlite, _>(&mut **tx, stmt.as_ref()).await,
            TxInner::Postgres(tx) => driver::execute::<Postgres, _>(&mut **tx, stmt.as_ref()).await,
            TxInner::MySql(tx) => driver::execute::<MySql, _>(&mut **tx, stmt.as_ref()).await,
        }
    }

    /// Run a multi-statement script inside the transaction.
    pub async fn execute_script(&mut self, sql: &str) -> Result<()> {
        match self.inner_mut()? {
            TxInner::Sqlite(tx) => driver::script::<Sqlite, _>(&mut **tx, sql).await,
            TxInner::Postgres(tx) => driver::script::<Postgres, _>(&mut **tx, sql).await,
            TxInner::MySql(tx) => driver::script::<MySql, _>(&mut **tx, sql).await,
        }
    }

    pub async fn fetch_all(&mut self, stmt: &Stmt) -> Result<Vec<Row>> {
        let stmt = stmt.for_dialect(self.dialect);
        match self.inner_mut()? {
            TxInner::Sqlite(tx) => driver::fetch_all::<Sqlite, _>(&mut **tx, stmt.as_ref()).await,
            TxInner::Postgres(tx) => {
                driver::fetch_all::<Postgres, _>(&mut **tx, stmt.as_ref()).await
            }
            TxInner::MySql(tx) => driver::fetch_all::<MySql, _>(&mut **tx, stmt.as_ref()).await,
        }
    }

    pub async fn fetch_optional(&mut self, stmt: &Stmt) -> Result<Option<Row>> {
        let stmt = stmt.for_dialect(self.dialect);
        match self.inner_mut()? {
            TxInner::Sqlite(tx) => {
                driver::fetch_optional::<Sqlite, _>(&mut **tx, stmt.as_ref()).await
            }
            TxInner::Postgres(tx) => {
                driver::fetch_optional::<Postgres, _>(&mut **tx, stmt.as_ref()).await
            }
            TxInner::MySql(tx) => {
                driver::fetch_optional::<MySql, _>(&mut **tx, stmt.as_ref()).await
            }
        }
    }

    pub async fn fetch_one(&mut self, stmt: &Stmt) -> Result<Row> {
        let stmt = stmt.for_dialect(self.dialect);
        match self.inner_mut()? {
            TxInner::Sqlite(tx) => driver::fetch_one::<Sqlite, _>(&mut **tx, stmt.as_ref()).await,
            TxInner::Postgres(tx) => {
                driver::fetch_one::<Postgres, _>(&mut **tx, stmt.as_ref()).await
            }
            TxInner::MySql(tx) => driver::fetch_one::<MySql, _>(&mut **tx, stmt.as_ref()).await,
        }
    }

    pub async fn count(&mut self, stmt: &Stmt) -> Result<u64> {
        let counting = crate::exec::count_stmt(stmt);
        let row = self.fetch_one(&counting).await?;
        Ok(row.count("count"))
    }

    /// Run a portable upsert inside the transaction.
    pub async fn upsert(&mut self, upsert: crate::sql::Upsert) -> Result<Affected> {
        self.execute(&upsert.into()).await
    }

    /// Commit. Errors if the transaction is already finished.
    pub async fn commit(&mut self) -> Result<()> {
        let inner = self.inner.take().ok_or_else(completed)?;
        match inner {
            TxInner::Sqlite(tx) => tx.commit().await.map_err(|error| transaction_error(&error)),
            TxInner::Postgres(tx) => tx.commit().await.map_err(|error| transaction_error(&error)),
            TxInner::MySql(tx) => tx.commit().await.map_err(|error| transaction_error(&error)),
        }
    }

    /// Roll back. Errors if the transaction is already finished.
    pub async fn rollback(&mut self) -> Result<()> {
        let inner = self.inner.take().ok_or_else(completed)?;
        match inner {
            TxInner::Sqlite(tx) => tx
                .rollback()
                .await
                .map_err(|error| transaction_error(&error)),
            TxInner::Postgres(tx) => tx
                .rollback()
                .await
                .map_err(|error| transaction_error(&error)),
            TxInner::MySql(tx) => tx
                .rollback()
                .await
                .map_err(|error| transaction_error(&error)),
        }
    }
}

fn transaction_error(e: &sqlx::Error) -> Error {
    Error::Database(DatabaseError::Transaction(e.to_string()))
}
