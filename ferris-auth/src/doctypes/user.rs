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
            FieldMeta::check("email_verified")
                .label("Email Verified")
                .default_value("0"),
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
            "The single source of truth for authorization: a JSON array of role \
             names, written and read by the login and permission paths. No other \
             table grants a role.",
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
            FieldMeta::secret("mfa_secret")
                .label("MFA Secret")
                .permlevel(1)
                .hidden()
                .description(
                    "A TOTP seed, sealed at rest by the engine (AES-256-GCM, \
                     `ferris_core::secret`). Ordinary reads return the mask, never \
                     the seed; only `DocumentEngine::read_secret` with \
                     `reveal_secrets()` recovers it.",
                ),
        )
        .field(
            FieldMeta::small_text("mfa_backup_codes")
                .label("MFA Backup Codes")
                .permlevel(1)
                .hidden()
                .description("A JSON array of Argon2 hashes; never the plaintext codes."),
        )
        .field(
            FieldMeta::int("mfa_last_step")
                .label("MFA Last Step")
                .permlevel(1)
                .hidden()
                .description(
                    "The last TOTP time step spent by the account. Written by one \
                     atomic conditional `UPDATE`, so a code cannot be replayed — \
                     not even against another replica.",
                ),
        )
        // OAuth identity binding. The provider's stable `sub` is written once, on
        // the first OAuth login, and compared afterwards: a provider that proves
        // an email is not enough on its own, because an address can be
        // reassigned or changed upstream. `permlevel` 1 keeps it out of ordinary
        // reads.
        .field(
            FieldMeta::data("oauth_subject")
                .label("OAuth Subject")
                .permlevel(1)
                .hidden()
                .description(
                    "The provider's stable `sub`, bound to this account at its \
                     first OAuth login. Written once; a later login presenting a \
                     different subject is refused.",
                ),
        )
        // Recovery credentials. Both keys hold the **SHA-256 hex digest** of a
        // single-use token, never the token itself, so a leaked row cannot be
        // turned into a working link. `permlevel` 1 keeps them out of every
        // read but the store's `System Manager` context.
        .field(
            FieldMeta::data("reset_password_key")
                .label("Reset Password Key")
                .permlevel(1)
                .hidden()
                .description(
                    "SHA-256 hex digest of the pending password-reset token; \
                     cleared on a successful confirmation.",
                ),
        )
        .field(
            FieldMeta::datetime("reset_password_expires_at")
                .label("Reset Password Expires At")
                .permlevel(1)
                .hidden()
                .description("RFC 3339 deadline for the pending reset token."),
        )
        .field(
            FieldMeta::data("email_verification_key")
                .label("Email Verification Key")
                .permlevel(1)
                .hidden()
                .description(
                    "SHA-256 hex digest of the pending email-verification token; \
                     cleared once the address is confirmed.",
                ),
        )
        .field(
            FieldMeta::datetime("email_verification_expires_at")
                .label("Email Verification Expires At")
                .permlevel(1)
                .hidden()
                .description("RFC 3339 deadline for the pending verification token."),
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
        // request; these indexes keep those `=` filters in SQL instead of
        // scanning the small-but-hot user table. They are **unique**: the login
        // path takes the first match, so two accounts sharing an address would
        // make sign-in and password reset resolve one of them at random. The
        // names are new (`_unique`) because `IF NOT EXISTS` would skip an index
        // that already exists under the old, non-unique name.
        .index(IndexMeta::new("idx_user_email_unique", ["email"]).unique())
        // `username` is optional; every backend allows several `NULL`s in a
        // unique index, so accounts without one are unaffected.
        .index(IndexMeta::new("idx_user_username_unique", ["username"]).unique())
        // Confirming a reset/verification link resolves the account by digest,
        // so those two lookups must not scan the user table.
        .index(IndexMeta::new("idx_user_reset_key", ["reset_password_key"]))
        .index(IndexMeta::new(
            "idx_user_verification_key",
            ["email_verification_key"],
        ))
        .permission(system_manager())
}
