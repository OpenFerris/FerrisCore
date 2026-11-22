//! The `Role Profile` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A reusable bundle of roles applied to a user.
#[must_use]
pub fn role_profile_meta() -> DoctypeMeta {
    DoctypeMeta::new("Role Profile", MODULE)
        .description("A reusable bundle of roles applied to a user.")
        .title_field("profile_name")
        .sort("profile_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("profile_name")
                .label("Profile Name")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::small_text("roles").label("Roles"))
        .field(FieldMeta::small_text("description").label("Description"))
        .permission(system_manager())
}
