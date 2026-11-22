//! DocType table migration engine.
//!
//! This module owns the **one** physical definition of a `tab<DocType>` table.
//! [`ferris_document`] creates the same tables lazily on first write, and the two
//! used to disagree: the engine's `CREATE TABLE IF NOT EXISTS` had a `data`
//! blob and integer timestamps, while the generated migration declared
//! `modified_by NOT NULL` plus per-field columns and no `data` at all. Whichever
//! ran first won and the other's queries then failed against it — and because
//! the generated names were quoted while the engine's were not, `tabToDo` and
//! `"tabToDo"` were two different tables on Postgres.
//!
//! [`schema::document_table`] and [`schema::document_columns`] are that shared
//! definition; both callers use them.
//!
//! The module was split in two when it crossed the 600-line budget: `schema.rs`
//! holds the table/column/index definition every caller shares, `runner.rs` the
//! [`MigrationRunner`] that turns those definitions into numbered migrations.
//! The public paths (`ferris_schema::migration::…`) are unchanged — the `pub use`
//! re-exports below keep them resolving.

mod runner;
mod schema;

#[cfg(test)]
mod tests;

pub use runner::{MigrationRunner, PendingMigration};
pub use schema::{
    DeclaredIndex, classify_index, document_columns, document_indexes, document_table,
    is_indexable_column, schema_version,
};
