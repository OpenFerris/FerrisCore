//! The `User Group` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A named collection of users.
#[must_use]
pub fn user_group_meta() -> DoctypeMeta {
    DoctypeMeta::new("User Group", MODULE)
        .description("A named collection of users.")
        .title_field("group_name")
        .sort("group_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("group_name")
                .label("Group Name")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::small_text("description").label("Description"))
        .permission(system_manager())
}
