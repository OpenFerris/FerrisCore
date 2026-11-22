//! The `Role Permission for Page and Report` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A page-or-report permission granted to a role.
#[must_use]
pub fn role_permission_for_page_and_report_meta() -> DoctypeMeta {
    DoctypeMeta::new("Role Permission for Page and Report", MODULE)
        .description("A page-or-report permission granted to a role.")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::link("role", "Role")
                .label("Role")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("page").label("Page"))
        .field(FieldMeta::data("report").label("Report"))
        .permission(system_manager())
}
