//! Engine helpers, split by concern.
//!
//! The module used to be one 655-line file, which the repository's size rule
//! (`AGENTS.md` §4) does not allow. It is a folder now: [`sink`] owns the
//! connection-vs-transaction seam, [`perm`] the authorization decisions,
//! [`fields`] the document-shape helpers, [`hooks`] the lifecycle dispatch, and
//! [`virtual_fields`] the Virtual DocField/DocType registries.
//!
//! `mod.rs` only declares and re-exports. Every path that resolved as
//! `engine::support::X` before still resolves: the glob re-exports below
//! reproduce the old flat namespace, and the two items with wider visibility
//! (`SYSTEM_FIELDS`, `is_engine_field`) are re-exported explicitly so a caller
//! outside `engine` keeps compiling.

mod fields;
mod hooks;
mod perm;
mod sink;
mod virtual_fields;

// Wider than `pub(crate)`: `engine/mod.rs` re-exports both, and `is_engine_field`
// is consumed by `crate::naming` / `crate::runtime` through the engine.
pub(crate) use fields::is_engine_field;
pub use perm::SYSTEM_FIELDS;
pub(in crate::engine) use perm::redact_read;
pub(crate) use sink::{PreparedInsert, WriteSink};
// The public Virtual DocField/DocType surface is re-exported from the crate root
// (`ferris_document::register_virtual_field` &c.).
pub use virtual_fields::{
    VirtualDoctypeProvider, VirtualFieldProvider, register_virtual_doctype, register_virtual_field,
};

// Everything else was `pub(super)` in the single-file module, which is exactly
// `pub(in crate::engine)` now that the items live one level deeper.
pub(in crate::engine) use fields::*;
pub(in crate::engine) use hooks::*;
pub(in crate::engine) use perm::*;
pub(in crate::engine) use sink::{
    canonical_blob, insert_row, read_row_via, run_after_insert_hooks, write_prepared,
};
pub(in crate::engine) use virtual_fields::{
    finish_virtual_write, is_virtual, reject_virtual_field_writes, virtual_doctype_provider,
    virtual_read,
};
