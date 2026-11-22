//! Write paths of the [`DocumentEngine`].
//!
//! One concern per file: naming, insert, update, delete, `apply`, `docstatus`
//! and the workflow gate. The module is a folder precisely so a change to one
//! path cannot push a single 900-line file past the size limit the repository
//! enforces (`AGENTS.md` §4); `mod.rs` only declares and re-exports.
//!
//! Cross-file helpers are `pub(super)` and reach each other through `super::`
//! (or `use super::naming::mint_name`), never by duplicating a rule.

mod amend;
mod apply;
mod delete;
mod docstatus;
mod insert;
mod naming;
mod rename;
mod update;
mod workflow;

// `mint_name` and `enforce_workflow` are free functions used by another file in
// this folder; re-export them at the folder root so `use super::naming::mint_name`
// stays a single, greppable import.
pub(super) use naming::mint_name;
pub(super) use workflow::enforce_workflow;
