//! The `User Invitation` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// A pending invitation for someone to join.
#[must_use]
pub fn user_invitation_meta() -> DoctypeMeta {
    DoctypeMeta::new("User Invitation", MODULE)
        .description("A pending invitation for someone to join.")
        .title_field("email")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("email")
                .label("Email")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("full_name").label("Full Name"))
        .field(FieldMeta::small_text("roles").label("Roles"))
        .field(FieldMeta::date("expires_on").label("Expires On"))
        .field(
            FieldMeta::check("accepted")
                .label("Accepted")
                .default_value("0")
                .in_list_view(),
        )
        .permission(system_manager())
}
