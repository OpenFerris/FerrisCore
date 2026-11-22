//! Translating a [`ListQuery`] into a `WHERE`/`ORDER BY`/`LIMIT` statement.
//!
//! # Why this module exists
//!
//! `DocumentEngine::list` used to run `SELECT data FROM tab<DocType> ORDER BY
//! modified DESC` with no `WHERE` and no `LIMIT`, deserialize **every** row into
//! a `Value`, filter, sort and only then `skip(offset).take(limit)`. One list
//! request over a table of a million rows materialized a million documents in
//! RAM: a one-line denial of service, and it made every later fix more expensive
//! because §C2 added a permission evaluation per row on top of it.
//!
//! Frappe does not have this problem because its fields are columns
//! (`frappe/model/db_query.py::DatabaseQuery.execute` composes `add_user_permissions`
//! and `get_permission_query_conditions` into the SQL and applies
//! `limit_page_length` there). Ferris stores the document as one JSON blob
//! (`ferris-schema/src/migration.rs:41-50`), so most predicates cannot be
//! expressed in SQL at all.
//!
//! # What this module does about that
//!
//! It splits the work by what the storage layer can actually answer:
//!
//! - **Column fields** — `name`, `owner`, `docstatus`, `creation`, `modified`.
//!   These are real, indexed columns ([`document_indexes`]). Filters, ordering
//!   and pagination over them are pushed down to SQL, which is where an index
//!   can be used.
//! - **Indexed blob fields** — a document field a DocType declares through a
//!   single-field `meta.indexes` entry. Because the declaration is a promise
//!   that an index over `data.field` exists (materialized by
//!   [`ferris_schema::migration::classify_index`] and
//!   [`ferris_db::sql::ensure_json_index`]), an **exact** filter — `=` and
//!   `in` — over such a field is pushed down as `json_extract(data,
//!   '$.field') = ?` (or the dialect's spelling), so the database can serve it
//!   from that index instead of scanning. Ranges and `LIKE` over an indexed
//!   field stay in Rust: the Rust comparator is numeric-when-both-parse and
//!   case-insensitive otherwise, and SQL text comparison would return
//!   different rows, which this module refuses to do.
//! - **Everything else** — living inside the `data` JSON without an index.
//!   These stay in Rust, and [`Plan::residual`] says so.
//!
//! The split is a **performance** boundary and never an **authorization** one.
//! Permission filtering is computed by the caller (the engine's `can_read`) and
//! is *not* something this module can push down: a policy decision depends on
//! the engine's principals, the document's owner and any condition, and
//! re-implementing that as SQL would be a second, silently divergent copy of
//! the authorization rule — the duplication `AGENTS.md` §3 forbids. So
//! [`Plan::page_is_exhaustive`] is false whenever the caller must still filter
//! rows, and the engine keeps paginating in Rust in that case.
//!
//! When *every* predicate is column-backed and the caller needs no row-level
//! filtering, [`Plan::page_is_exhaustive`] is true and the engine can trust SQL
//! for `LIMIT`/`OFFSET` and `COUNT(*)`. That is the case this module exists to
//! make reachable; the fallback is documented rather than silent.
//!
//! [`document_indexes`]: ferris_schema::migration::document_indexes

mod plan;
mod render;
mod types;

// The planner's public surface. `Plan` is what `list()` consumes; the reason
// types are public so a caller can explain *why* a filter did not reach SQL.
pub use types::{Plan, Residual, ResidualReason, is_column_field, is_filterable_field};

// The OR-group count and the `LIKE` escaper, re-exported only for the test
// modules (`#[cfg(test)]`), which assert the planner agrees with the Rust
// evaluator (DOC5) without reaching into a private module.
#[cfg(test)]
pub(crate) use render::{escape_like, group_count};

// Internal to the module tree: the predicate/order AST and the free functions
// `plan` and `render` share. Not promoted to `pub` — nothing outside should name
// a predicate.
// `Plan::build` / `order_sql` / `select_all` are inherent methods defined in
// `plan.rs`; they are reached through the type, not re-exported here.

#[cfg(test)]
mod tests;
