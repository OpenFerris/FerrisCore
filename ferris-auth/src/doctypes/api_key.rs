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
/// `secret_hash` is a credential digest at `permlevel` 1: a plain `Data` value
/// (`sha256:<64 hex>`) written by `ApiKeyManager`, hidden from ordinary readers
/// and read-only in the UI. It is deliberately **not** a `Password` field — the
/// manager already stores a one-way digest and letting the engine run it
/// through Argon2 would hash a hash on every write. The table is infrastructure
/// and `hidden`, like `DocPerm` and `Session`.
#[must_use]
pub fn api_key_meta() -> DoctypeMeta {
    DoctypeMeta::new("API Key", MODULE)
        .description("An issued API key. The secret is stored only as a hash.")
        .sort("created_at", "desc")
        .allow_delete()
        .hidden()
        .field(FieldMeta::section_break("section_details").label("Details"))
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
            FieldMeta::data("secret_hash")
                .label("Secret Hash")
                .permlevel(1)
                .hidden()
                .read_only()
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
        // The public `key` is the lookup value of every token request *and* a
        // credential identifier: two live rows must never share it. The index
        // is named `..._unique` (rather than making the old `idx_api_key_key`
        // unique) so an existing table actually gains the constraint — a
        // renamed declaration would be skipped by `IF NOT EXISTS` against the
        // index of the old name.
        .index(IndexMeta::new("idx_api_key_key_unique", ["key"]).unique())
        .permission(system_manager())
}
