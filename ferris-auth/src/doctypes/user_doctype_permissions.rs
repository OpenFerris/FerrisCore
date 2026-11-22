//! The `User Doctype Permissions` report DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// Report: the DocType-level permissions granted to a user.
#[must_use]
pub fn user_doctype_permissions_meta() -> DoctypeMeta {
    DoctypeMeta::new("User Doctype Permissions", MODULE)
        .description("Report: the DocType-level permissions granted to a user.")
        .sort("modified", "desc")
        .field(FieldMeta::link("user", "User").label("User").in_list_view())
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
