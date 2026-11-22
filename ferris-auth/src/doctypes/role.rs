//! The `Role` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A named set of rights granted to users.
#[must_use]
pub fn role_meta() -> DoctypeMeta {
    DoctypeMeta::new("Role", MODULE)
        .description("A named set of rights granted to users.")
        .title_field("role_name")
        .sort("role_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("role_name")
                .label("Role Name")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::check("desk_access")
                .label("Desk Access")
                .default_value("1")
                .in_list_view(),
        )
        .field(
            FieldMeta::check("disabled")
                .label("Disabled")
                .default_value("0"),
        )
        .field(FieldMeta::small_text("description").label("Description"))
        .permission(system_manager())
}
