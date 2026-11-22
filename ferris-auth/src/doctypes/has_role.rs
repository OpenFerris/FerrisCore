//! The `Has Role` child table.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::MODULE;

/// The `Has Role` child table: one role grant on a parent document.
///
/// Frappe declares this under `core/doctype/has_role` (`istable: 1`), but the
/// identity surface is owned here, so it is declared with the Users DocTypes and
/// referenced by name (`Table` → `Has Role`) from any parent that needs it —
/// `Workspace.roles` is the first. A child table carries no permissions of its
/// own.
#[must_use]
pub fn has_role_meta() -> DoctypeMeta {
    DoctypeMeta::new("Has Role", MODULE)
        .description("One role grant, embedded in a parent document.")
        .child_table()
        .field(FieldMeta::link("role", "Role").label("Role").in_list_view())
}
