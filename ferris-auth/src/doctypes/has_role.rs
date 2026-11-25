//! The `Has Role` child table.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::MODULE;

/// The `Has Role` child table: one role grant on a parent document.
///
/// Frappe declares this under `core/doctype/has_role` (`istable: 1`), but the
/// identity surface is owned here, so it is declared with the Users DocTypes.
///
/// **Not** an authorization source. `User.roles` (a JSON array) is the single
/// source of truth the login and permission paths read and write. The only
/// parent that references this table is `Workspace.roles`, whose sync writes an
/// empty array and whose readers never consult it for access decisions, so the
/// declaration is kept for that child-table target and for the reserved-name
/// contract, not because a role is ever resolved through it. A child table
/// carries no permissions of its own.
#[must_use]
pub fn has_role_meta() -> DoctypeMeta {
    DoctypeMeta::new("Has Role", MODULE)
        .description("One role grant, embedded in a parent document.")
        .child_table()
        .field(FieldMeta::link("role", "Role").label("Role").in_list_view())
}
