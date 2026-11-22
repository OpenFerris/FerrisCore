//! The `API Key` DocType (a Ferris extension over Frappe).

use ferris_schema::{DoctypeMeta, FieldMeta, IndexMeta};

use super::common::{MODULE, system_manager};

/// An issued API key (a Ferris extension over Frappe).
///
/// Frappe keeps API credentials as the `api_key`/`api_secret` *fields* of
/// `User` and ships no `API Key` DocType to mirror. Ferris's `ApiKeyManager`
/// issues many keys per user, each with its own public `key`, description,
/// expiry and revoke state — something a single pair of `User` fields cannot
/// hold — so the row is declared here and the manager migrates onto it.
/// `secret_hash` is a credential: a `Password` field at `permlevel` 1, so the
/// engine hashes the plaintext on write and masks the hash on read. The table is
/// infrastructure and `hidden`, like `DocPerm` and `Session`.
#[must_use]
pub fn api_key_meta() -> DoctypeMeta {
    DoctypeMeta::new("API Key", MODULE)
        .description("An issued API key. The secret is stored only as a hash.")
        .sort("created_at", "desc")
        .allow_delete()
        .hidden()
        .field(
            FieldMeta::link("user", "User")
                .label("User")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("key")
                .label("Key")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::password("secret_hash")
                .label("Secret Hash")
                .permlevel(1)
                .hidden()
                .required(),
        )
        .field(
            FieldMeta::check("enabled")
                .label("Enabled")
                .default_value("1"),
        )
        .field(FieldMeta::data("description").label("Description"))
        .field(
            FieldMeta::datetime("created_at")
                .label("Created At")
                .required(),
        )
        .field(FieldMeta::datetime("last_used_at").label("Last Used At"))
        .field(FieldMeta::datetime("expires_at").label("Expires At"))
        // Every token request looks a key up by this value.
        .index(IndexMeta::new("idx_api_key_key", ["key"]))
        .permission(system_manager())
}
