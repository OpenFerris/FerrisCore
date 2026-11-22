//! The transaction handle.
//!
//! Split out of `exec.rs` so that file holds one concern (the pooled
//! connection) and this one holds another (a transaction on it). `Tx` is
//! re-exported from [`crate::exec`] so every existing `crate::exec::Tx` path
//! still resolves.

use crate::dialect::Dialect;
use crate::error;
use crate::exec::{Affected, Stmt};
use crate::row::Row;
use ferris_core::{Error, Result};
use sqlx::Any;

/// An open transaction.
///
/// Dropping it without committing rolls back: `sqlx` queues the rollback on
/// the connection, so releasing the handle is enough and there is nothing
/// useful to do from `Drop`.
pub struct Tx {
    inner: Option<sqlx::Transaction<'static, Any>>,
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
    Error::Database(ferris_core::error::DatabaseError::Transaction(
        "Transaction already completed".into(),
    ))
}

impl Tx {
    /// Wrap an open driver transaction together with the dialect it speaks.
    pub(crate) fn new(inner: sqlx::Transaction<'static, Any>, dialect: Dialect) -> Self {
        Self {
            inner: Some(inner),
            dialect,
        }
    }

    /// The connection this transaction runs on.
    ///
    /// `sqlx::Transaction` implements `Deref` to its connection, and the
    /// `Executor` bound is on that connection rather than on the transaction,
    /// so every statement goes through here.
    fn inner_mut(&mut self) -> Result<&mut sqlx::AnyConnection> {
        let tx = self.inner.as_mut().ok_or_else(completed)?;
        Ok(&mut **tx)
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Whether this transaction has been committed or rolled back.
    pub fn is_finished(&self) -> bool {
        self.inner.is_none()
    }

    pub async fn execute(&mut self, stmt: &Stmt) -> Result<Affected> {
        let inner = self.inner_mut()?;
        stmt.query()
            .execute(&mut *inner)
            .await
            .map(|r| Affected(r.rows_affected()))
            .map_err(error::map)
    }

    /// Run a multi-statement script inside the transaction.
    pub async fn execute_script(&mut self, sql: &str) -> Result<()> {
        let inner = self.inner_mut()?;
        sqlx::raw_sql(sql)
            .execute(&mut *inner)
            .await
            .map(|_| ())
            .map_err(error::map)
    }

    pub async fn fetch_all(&mut self, stmt: &Stmt) -> Result<Vec<Row>> {
        let inner = self.inner_mut()?;
        let rows = stmt
            .query()
            .fetch_all(&mut *inner)
            .await
            .map_err(error::map)?;
        Ok(rows.into_iter().map(Row::new).collect())
    }

    pub async fn fetch_optional(&mut self, stmt: &Stmt) -> Result<Option<Row>> {
        let row = stmt.query().fetch_optional(self.inner_mut()?).await;
        crate::row::optional(row).map(|r| r.map(Row::new))
    }

    pub async fn fetch_one(&mut self, stmt: &Stmt) -> Result<Row> {
        let row = stmt
            .query()
            .fetch_one(self.inner_mut()?)
            .await
            .map_err(error::map)?;
        Ok(Row::new(row))
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
        let tx = self.inner.take().ok_or_else(completed)?;
        tx.commit().await.map_err(|e| transaction_error(&e))
    }

    /// Roll back. Errors if the transaction is already finished.
    pub async fn rollback(&mut self) -> Result<()> {
        let tx = self.inner.take().ok_or_else(completed)?;
        tx.rollback().await.map_err(|e| transaction_error(&e))
    }
}

fn transaction_error(e: &sqlx::Error) -> Error {
    Error::Database(ferris_core::error::DatabaseError::Transaction(
        e.to_string(),
    ))
}
