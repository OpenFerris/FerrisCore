//! The `User Invitation` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta, IndexMeta};

use super::common::{MODULE, system_manager};

/// A pending invitation for someone to join.
///
/// The token the invited person receives is never stored: `token_hash` holds
/// its SHA-256 hex digest, at `permlevel` 1 and hidden, and the row is marked
/// `accepted` before the `User` is created. `roles` is a JSON array of role
/// names as text — the invitation declares what the new account may do.
#[must_use]
pub fn user_invitation_meta() -> DoctypeMeta {
    DoctypeMeta::new("User Invitation", MODULE)
        .description("A pending invitation for someone to join.")
        .title_field("email")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::email("email")
                .label("Email")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("full_name").label("Full Name"))
        .field(
            FieldMeta::small_text("roles")
                .label("Roles")
                .description("JSON array of role names granted to the accepted account."),
        )
        .field(
            FieldMeta::datetime("expires_at")
                .label("Expires At")
                .description("RFC 3339 deadline; the token is refused after it."),
        )
        .field(
            FieldMeta::data("invited_by")
                .label("Invited By")
                .description("Account that issued the invitation."),
        )
        .field(
            FieldMeta::data("token_hash")
                .label("Token Hash")
                .permlevel(1)
                .hidden()
                .description(
                    "SHA-256 hex digest of the invitation token; cleared when the \
                     invitation is accepted.",
                ),
        )
        .field(
            FieldMeta::check("accepted")
                .label("Accepted")
                .default_value("0")
                .in_list_view(),
        )
        .field(
            FieldMeta::datetime("accepted_at")
                .label("Accepted At")
                .description(
                    "RFC 3339 timestamp of the acceptance; set in the same \
                     atomic claim that consumes the token.",
                ),
        )
        // Acceptance resolves the row by digest on the unauthenticated path.
        .index(IndexMeta::new("idx_user_invitation_token", ["token_hash"]))
        .permission(system_manager())
}
