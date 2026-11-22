//! The framework's own schema DocTypes: `DocType` (and its child `DocField`).
//!
//! Every other crate declares the DocTypes of its *domain* (`ferris-auth` the
//! identity ones, `ferris-web` the CMS ones, and so on). This module declares
//! the DocTypes that describe DocTypes, so the schema registry itself becomes
//! data — the same move Frappe makes with `DocType`/`DocField`/`DocPerm`.
//!
//! `DocType` is registered like any other DocType; its rows are the registry's
//! metadata, mirrored by [`store::DoctypeStore`]. That is what lets the Desk (or
//! an operator) list, inspect and, in time, edit DocTypes through the ordinary
//! document engine instead of a bespoke introspection endpoint.
//!
//! Permissions stay in `ferris-auth`'s `tabDocPerm` (`ferris_auth::docperm`):
//! that table already exists with its own schema, so `DocPerm` is deliberately
//! **not** redeclared here — a DocType declaration for it would create a second
//! `tabDocPerm` with the generic columns and collide with the real one.
#![allow(clippy::doc_markdown)]

use ferris_schema::builders::system_manager;
use ferris_schema::registry::{DoctypeMeta, FieldMeta};

mod store;

pub use store::{DoctypeStore, SyncReport};

/// Desk module of the metadata DocTypes: the same **Build** section that holds
/// the rest of the framework's schema tooling (`ferris-schema/src/doctypes.rs`).
pub const BUILD_MODULE: &str = "Build";

/// The DocType that stores DocTypes.
pub const DOCTYPE: &str = "DocType";

/// The child DocType describing one field of a DocType.
pub const DOCFIELD: &str = "DocField";

/// The `DocType` DocType: one document per registered DocType.
///
/// The individual columns exist for the Desk list (module, flags, counts); the
/// whole metadata is also carried losslessly in `meta_json` so
/// [`DoctypeStore::load_into_registry`] can rebuild a [`DoctypeMeta`] exactly,
/// including the parts (indexes, workflow, permissions) a small column set
/// cannot represent.
#[must_use]
pub fn doc_type_meta() -> DoctypeMeta {
    DoctypeMeta::new(DOCTYPE, BUILD_MODULE)
        .description("A document type — the stored form of the schema registry.")
        .sort("modified", "desc")
        .title_field("module")
        .allow_delete()
        .field(FieldMeta::data("module").label("Module").in_list_view())
        .field(FieldMeta::small_text("description").label("Description"))
        .field(FieldMeta::check("is_single").label("Is Single"))
        .field(FieldMeta::check("is_submittable").label("Is Submittable"))
        .field(FieldMeta::check("is_child_table").label("Is Child Table"))
        .field(FieldMeta::check("is_tree").label("Is Tree"))
        .field(FieldMeta::check("is_virtual").label("Is Virtual"))
        .field(FieldMeta::check("custom").label("Custom"))
        .field(FieldMeta::data("naming_series").label("Naming Series"))
        .field(FieldMeta::data("title_field").label("Title Field"))
        .field(
            FieldMeta::int("field_count")
                .label("Field Count")
                .read_only(),
        )
        .field(FieldMeta::table("fields", DOCFIELD).label("Fields"))
        .field(FieldMeta::code("meta_json").label("Metadata").hidden())
        .permission(system_manager())
}

/// The child `DocField` DocType: one row per field of a parent `DocType`.
#[must_use]
pub fn doc_field_meta() -> DoctypeMeta {
    DoctypeMeta::new(DOCFIELD, BUILD_MODULE)
        .child_table()
        .description("One field of a DocType.")
        .field(FieldMeta::link("parent", DOCTYPE).label("Document Type"))
        .field(
            FieldMeta::data("fieldname")
                .label("Fieldname")
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
        .field(
            FieldMeta::data("fieldtype")
                .label("Field Type")
                .in_list_view(),
        )
        .field(FieldMeta::small_text("options").label("Options"))
        .field(FieldMeta::text("default").label("Default"))
        .field(FieldMeta::check("reqd").label("Required"))
        .field(FieldMeta::check("in_list_view").label("In List View"))
        .permission(system_manager())
}

/// Every DocType this crate declares, in registration order.
#[must_use]
pub fn doctype_metas() -> Vec<DoctypeMeta> {
    vec![doc_type_meta(), doc_field_meta()]
}

/// Register this crate's DocTypes in the global schema registry.
///
/// Idempotent, like every other crate's `register_doctypes`. The catalog in the
/// host (`ferris-server/src/catalog.rs`) calls this alongside the domain crates.
pub fn register_doctypes() {
    for meta in doctype_metas() {
        ferris_schema::register_doctype(meta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_is_a_first_class_doctype_in_the_build_module() {
        let meta = doc_type_meta();
        assert_eq!(meta.name, DOCTYPE);
        assert_eq!(meta.module, BUILD_MODULE);
        assert!(!meta.is_single);
        assert!(!meta.is_child_table);
        assert!(!meta.fields.is_empty());
    }

    #[test]
    fn doc_field_is_a_child_table_of_doctype() {
        let meta = doc_field_meta();
        assert_eq!(meta.name, DOCFIELD);
        assert!(meta.is_child_table);
        assert_eq!(
            meta.fields
                .iter()
                .find(|f| f.fieldname == "parent")
                .unwrap()
                .fieldtype,
            "Link"
        );
    }
}
