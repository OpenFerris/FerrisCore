//! The `Session` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta, IndexMeta};

use super::common::{MODULE, system_manager};

/// A signed-in session.
///
/// Frappe keeps sessions in the `tabSessions` virtual table with no DocType;
/// declaring one here lets `SessionManager` migrate onto the engine, so a
/// session gets the same lifecycle (validation, hooks, field permissions) as
/// any other document. `csrf_token` is a credential: administrator-only and
/// hidden. The table itself is infrastructure — like `DocPerm` it is declared
/// `hidden`, so it stays out of the Users workspace and is reached through the
/// session store rather than a sidebar link.
#[must_use]
pub fn session_meta() -> DoctypeMeta {
    DoctypeMeta::new("Session", MODULE)
        .description("A signed-in session.")
        .sort("created_at", "desc")
        .allow_delete()
        .hidden()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("user_id", "User")
                .label("User")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("ip_address").label("IP Address"))
        .field(FieldMeta::small_text("user_agent").label("User Agent"))
        .field(FieldMeta::small_text("data").label("Data").required())
        .field(
            FieldMeta::datetime("created_at")
                .label("Created At")
                .required(),
        )
        .field(
            FieldMeta::datetime("last_activity")
                .label("Last Activity")
                .required(),
        )
        .field(
            FieldMeta::datetime("expires_at")
                .label("Expires At")
                .required(),
        )
        .field(
            FieldMeta::data("csrf_token")
                .label("CSRF Token")
                .permlevel(1)
                .hidden()
                .read_only()
                .required(),
        )
        // Listing and capping a user's sessions filters on this field.
        .index(IndexMeta::new("idx_session_user_id", ["user_id"]))
        .permission(system_manager())
}
