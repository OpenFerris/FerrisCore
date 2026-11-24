//! The `User` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta, IndexMeta};

use super::common::{MODULE, system_manager};

/// An account that can sign in to the Desk.
#[must_use]
pub fn user_meta() -> DoctypeMeta {
    DoctypeMeta::new("User", MODULE)
        .description("An account that can sign in to the Desk.")
        .title_field("full_name")
        .sort("full_name", "asc")
        .allow_delete()
        .field(FieldMeta::tab_break("tab_profile").label("Profile"))
        .field(FieldMeta::section_break("section_identity").label("Identity"))
        .field(
            FieldMeta::email("email")
                .label("Email")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("full_name")
                .label("Full Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("username").label("Username"))
        .field(
            FieldMeta::select("user_type", "SystemUser\nWebsiteUser\nGuest")
                .label("User Type")
                .default_value("SystemUser")
                .in_list_view(),
        )
        .field(
            FieldMeta::check("enabled")
                .label("Enabled")
                .default_value("1")
                .in_list_view(),
        )
        .field(FieldMeta::link("role_profile", "Role Profile").label("Role Profile"))
        .field(FieldMeta::small_text("roles").label("Roles").description(
            "Embedded role list, stored as a JSON array. The `Has Role` child table \
                     is declared, but the login path still reads roles from this field.",
        ))
        // Credentials and secrets. A `Password` field is hashed on write and
        // masked on read by the document engine (`ferris_core::password`); a
        // `permlevel` of 1 makes the field administrator-only, so it is stripped
        // from any read a non-administrator performs.
        .field(FieldMeta::data("language").label("Language"))
        .field(FieldMeta::data("timezone").label("Timezone"))
        .field(FieldMeta::tab_break("tab_security").label("Security"))
        .field(FieldMeta::section_break("section_credentials").label("Credentials"))
        .field(
            FieldMeta::password("password")
                .label("Password")
                .hidden()
                .description("Stored as an Argon2 hash; reads back masked."),
        )
        .field(
            FieldMeta::data("api_key")
                .label("API Key")
                .permlevel(1)
                .hidden(),
        )
        .field(
            FieldMeta::password("api_secret")
                .label("API Secret")
                .permlevel(1)
                .hidden(),
        )
        .field(
            FieldMeta::small_text("mfa_secret")
                .label("MFA Secret")
                .permlevel(1)
                .hidden()
                .description(
                    "A TOTP seed. Frappe encrypts it (a reversible `Password`); \
                     Ferris's `Password` fieldtype is one-way, so the seed is a \
                     hidden administrator-only field instead — the engine refuses \
                     to hand it to anyone below `permlevel` 1.",
                ),
        )
        .field(
            FieldMeta::small_text("mfa_backup_codes")
                .label("MFA Backup Codes")
                .permlevel(1)
                .hidden()
                .description("A JSON array of SHA-256 hashes; never the plaintext codes."),
        )
        .field(
            FieldMeta::data("reset_password_key")
                .label("Reset Password Key")
                .permlevel(1)
                .hidden(),
        )
        // Login audit. Read-only: the engine's `set_user_and_timestamp` and the
        // login path own these, not the caller's body.
        .field(
            FieldMeta::check("mfa_enabled")
                .label("MFA Enabled")
                .default_value("0"),
        )
        .field(FieldMeta::tab_break("tab_activity").label("Activity"))
        .field(FieldMeta::section_break("section_login_audit").label("Login Audit"))
        .field(
            FieldMeta::datetime("last_login")
                .label("Last Login")
                .read_only(),
        )
        .field(
            FieldMeta::int("login_count")
                .label("Login Count")
                .default_value("0")
                .read_only(),
        )
        .field(
            FieldMeta::int("failed_login_attempts")
                .label("Failed Login Attempts")
                .default_value("0")
                .read_only(),
        )
        .field(
            FieldMeta::datetime("locked_until")
                .label("Locked Until")
                .permlevel(1)
                .hidden(),
        )
        // Login resolves by email or username on every unauthenticated
        // request; the JSON expression indexes keep those `=` filters pushed
        // into SQL instead of scanning the small-but-hot user table.
        .index(IndexMeta::new("idx_user_email", ["email"]))
        .index(IndexMeta::new("idx_user_username", ["username"]))
        .permission(system_manager())
}
