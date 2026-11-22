//! The `Module Profile` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A reusable bundle of modules a user may reach.
#[must_use]
pub fn module_profile_meta() -> DoctypeMeta {
    DoctypeMeta::new("Module Profile", MODULE)
        .description("A reusable bundle of modules a user may reach.")
        .title_field("profile_name")
        .sort("profile_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("profile_name")
                .label("Profile Name")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::small_text("modules").label("Modules"))
        .field(FieldMeta::small_text("description").label("Description"))
        .permission(system_manager())
}
