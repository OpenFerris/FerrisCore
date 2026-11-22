//! Fluent builders for declarative DocType definitions.
//!
//! The registry's [`DoctypeMeta`]/[`FieldMeta`]/[`IndexMeta`] are plain structs
//! with a long tail of `in_*` toggles and a `Default` that is the right zero:
//! everything off, nothing granted. That is correct but verbose at a call site,
//! which is why native crates used to hand-write
//! `DoctypeMeta { ..Default::default() }` literals.
//!
//! These builders keep the literals out of the catalog. A crate declares its
//! DocTypes in its own `doctypes.rs` with a readable chain:
//!
//! ```ignore
//! DoctypeMeta::new("Print Settings", "Printing")
//!     .single()
//!     .field(FieldMeta::data("name").label("Name"))
//!     .permission(RolePermission::new("System Manager").read().write())
//! ```
//!
//! The `#[derive(DocType)]` macro stays the path for the typed engine route;
//! builders are for declarative, struct-less definitions (and for the tests of
//! DocTypes that never need a Rust type).
//!
//! Every method consumes and returns `Self` so the chain reads as one expression;
//! each is `#[must_use]` because dropping a builder silently drops the
//! definition. `DoctypeMeta::icon()` from the design sketch is deliberately
//! absent: the registry stores `icon_field` (the field holding an icon), not an
//! icon literal.
//!
//! Lint note: DocType and fieldtype identifiers (`DocType`, `Data`, `Link`)
//! appear in prose throughout, and backticking every mention is noise. The
//! non-allocating builders stay non-`const` so a chain has one uniform method
//! signature beside the allocating ones (`label`, `field`, …); `const` buys
//! nothing at the boot-time declaration sites these serve.
#![allow(clippy::doc_markdown, clippy::missing_const_for_fn)]

mod doctype_meta;
mod field_meta;
mod grants;
mod index_meta;
mod role_permission;

pub use grants::{system_manager, system_manager_single};
pub use role_permission::RolePermission;
