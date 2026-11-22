//! The `Role Permission Manager` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// The permission grid of one role on one DocType.
#[must_use]
pub fn role_permission_manager_meta() -> DoctypeMeta {
    DoctypeMeta::new("Role Permission Manager", MODULE)
        .description("Edits the permission rules a role holds on a DocType.")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::link("role", "Role").label("Role").in_list_view())
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(
            FieldMeta::int("permlevel")
                .label("Permission Level")
                .default_value("0"),
        )
        .permission(system_manager())
}
