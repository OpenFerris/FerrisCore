//! Ferris DB — the framework's database boundary.
//!
//! Everything that touches a database goes through this crate. The rules it
//! enforces:
//!
//! - **One pool construction.** [`create_pool`] is the only way a pool is
//!   built, so the SSL mode, SQLite file creation and migration bookkeeping
//!   cannot be skipped by a caller that rolls its own.
//! - **One dialect, carried by the connection.** [`Db`] holds the [`Dialect`],
//!   so SQL is spelled for the backend actually being talked to. There is no
//!   way to forget to set it.
//! - **One error mapping.** [`error::map`] is the only path from a driver
//!   error to a `ferris_core::Error`.
//! - **Values are always bound.** Identifiers are validated and quoted by the
//!   builders in [`sql`] — [`sql::create_table`], [`UpsertBuilder::build`] and
//!   friends — and values travel as query parameters in [`Stmt`].
//!
//! ```no_run
//! # use ferris_db::{Db, Stmt, sql::Column};
//! # async fn example(db: &Db) -> ferris_core::Result<()> {
//! db.ensure_table(
//!     "sessions",
//!     &[Column::new("id", "VARCHAR(255)").not_null().primary_key()],
//! )
//! .await?;
//!
//! let row = db.fetch_one(&Stmt::raw("SELECT 1 AS one")).await?;
//! assert_eq!(row.i64("one")?, 1);
//! # Ok(())
//! # }
//! ```

pub mod dialect;
pub mod error;
pub mod exec;
pub mod migration;
pub mod pool;
pub mod row;
pub mod sql;

#[cfg(test)]
mod test_support;

pub use dialect::Dialect;
pub use error::map as map_error;
pub use exec::{Affected, Db, Stmt, Tx};
pub use migration::{
    ChecksumMismatch, Migration, MigrationResult, MigrationRunner, calculate_checksum,
};
pub use pool::{create_pool, database_url, memory_db, open_unmanaged, validate as validate_pool};
pub use row::Row;
pub use sql::{Column, Ident, Upsert, UpsertBuilder, UpsertSet};
