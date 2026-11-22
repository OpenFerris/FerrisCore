//! Attribute argument types for the four macros.
//!
//! Every `#[...]` attribute is parsed into one of these plain structs, so the
//! expansion modules only ever read typed fields and never re-parse tokens.
//!
//! One file per macro keeps the grammar next to the expansion it feeds:
//! [`doctype`] also owns `IndexDef`/`IndexType` (referenced by `indexes`), and
//! [`permission`] owns `PermissionRule`, which `#[doctype(permissions = …)]`
//! reuses. The shared `in_*` flag list lives in [`in_flags`].

// The parser locals are named after the value they hold (`rule`, `state`,
// `index`); clippy reads the module path `args::…` as a repetition of the type
// name, which is not what the lint is for.
#![allow(clippy::module_name_repetitions)]

mod doctype;
mod in_flags;
mod job;
mod permission;
mod workflow;

pub(crate) use doctype::DoctypeArgs;
pub(crate) use in_flags::InFlags;
pub(crate) use job::JobArgs;
pub(crate) use permission::{PermissionArgs, PermissionRule};
pub(crate) use workflow::WorkflowArgs;
