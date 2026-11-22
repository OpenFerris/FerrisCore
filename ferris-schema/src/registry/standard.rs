//! The standard fields every registered DocType gets.
//!
//! "Standard" (Frappe's term) is distinct from the engine's `SYSTEM_FIELDS`:
//! a system field is engine-owned, while a standard field is an ordinary,
//! client-writable field the framework adds so every `DocType` has it.

use super::types::{DoctypeMeta, FieldMeta};

/// The fieldname of the `tags` standard field.
///
/// Every `DocType` carries it without having to declare it. It is a plain
/// `Data` field holding a comma-separated tag list, Frappe's `_user_tags`
/// shape: a document's tags are part of its own payload and round-trip with it,
/// even when the derived `Tag`/`Tag Link` tables (`ferris_api::tags`) could not
/// be written. The [`tags_field`] declaration below is what the Desk renders and
/// the engine validates.
pub const TAGS_FIELD: &str = "tags";

/// The `tags` standard field.
///
/// `Data` (a single-line, comma-separated list) rather than a `Table
/// MultiSelect`: the form renderer has no multi-select widget, and this field is
/// the **source of truth** — `ferris_api::tags` derives the `Tag` catalogue and
/// the `Tag Link` associations from it on every save, so the document stays
/// readable on its own. `Tag` remains its own `DocType` for the
/// catalogue/autocomplete view; a document field is a reference into it, not a
/// second declaration.
#[must_use]
pub fn tags_field() -> FieldMeta {
    FieldMeta {
        fieldname: TAGS_FIELD.to_string(),
        label: Some("Tags".to_string()),
        fieldtype: "Data".to_string(),
        in_standard_filter: true,
        description: Some("Comma-separated tags. Each tag is a `Tag` document.".to_string()),
        ..Default::default()
    }
}

/// The standard fields every registered `DocType` gets, in declaration order.
///
/// "Standard" (Frappe's term) is distinct from the engine's `SYSTEM_FIELDS`
/// (`owner`, `creation`, …): a system field is engine-owned (a client may not
/// set `owner`), while a standard field is an ordinary, client-writable field
/// the framework adds so every `DocType` has it. `tags` is the first of them.
#[must_use]
pub fn standard_fields() -> Vec<FieldMeta> {
    vec![tags_field()]
}

/// Append to `meta` every standard field it does not already declare.
///
/// Idempotent, and a declaration keeps the last word: a `DocType` that declares
/// its own `tags` field (a different label, `hidden`, `read_only`, …) is not
/// overwritten. [`SchemaRegistry::register`] calls this, which is what makes the
/// guarantee "every `DocType` has `tags`" hold for compiled, core and plugin
/// definitions alike — they all go through the same registration.
pub fn add_standard_fields(meta: &mut DoctypeMeta) {
    for field in standard_fields() {
        if !meta.fields.iter().any(|f| f.fieldname == field.fieldname) {
            meta.fields.push(field);
        }
    }
}
