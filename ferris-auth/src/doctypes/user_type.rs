//! The `User Type` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A classification of users (system, website, guest).
#[must_use]
pub fn user_type_meta() -> DoctypeMeta {
    DoctypeMeta::new("User Type", MODULE)
        .description("A classification of users.")
        .title_field("user_type_name")
        .sort("user_type_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("user_type_name")
                .label("User Type Name")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::check("is_standard")
                .label("Is Standard")
                .default_value("0"),
        )
        .field(FieldMeta::small_text("description").label("Description"))
        .permission(system_manager())
}
