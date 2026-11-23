//! The `DocPerm` DocType.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::common::{MODULE, system_manager};

/// The rights one role holds on one DocType (Frappe's `DocPerm`).
///
/// Frappe declares this as a child table of `DocType`; Ferris keys a row on
/// `(doc_type, role, permlevel)` and persists it on its own, so it is declared
/// as a standalone — but hidden — DocType. `DocPermStore` and
/// `ferris-api::authz::install_policies_with_role_permissions` read and write
/// exactly these rows, and the engine translation (`doc_type_policy`) is shared,
/// so a persisted row and a declared rule produce the same statements.
#[must_use]
pub fn docperm_meta() -> DoctypeMeta {
    DoctypeMeta::new("DocPerm", MODULE)
        .description("The rights one role holds on one DocType.")
        .sort("modified", "desc")
        .allow_delete()
        .hidden()
        .field(FieldMeta::tab_break("tab_target").label("Target"))
        .field(FieldMeta::section_break("section_target").label("Target"))
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("role", "Role")
                .label("Role")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::int("permlevel")
                .label("Permission Level")
                .default_value("0"),
        )
        .field(FieldMeta::tab_break("tab_rights").label("Rights"))
        .field(FieldMeta::section_break("section_document_rights").label("Document Rights"))
        .field(FieldMeta::check("read").label("Read").default_value("0"))
        .field(FieldMeta::check("write").label("Write").default_value("0"))
        .field(
            FieldMeta::check("create")
                .label("Create")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("delete")
                .label("Delete")
                .default_value("0"),
        )
        .field(FieldMeta::section_break("section_lifecycle").label("Lifecycle"))
        .field(
            FieldMeta::check("submit")
                .label("Submit")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("cancel")
                .label("Cancel")
                .default_value("0"),
        )
        .field(FieldMeta::check("amend").label("Amend").default_value("0"))
        .field(FieldMeta::section_break("section_sharing").label("Sharing and Reporting"))
        .field(FieldMeta::check("print").label("Print").default_value("0"))
        .field(FieldMeta::check("email").label("Email").default_value("0"))
        .field(FieldMeta::check("share").label("Share").default_value("0"))
        .field(
            FieldMeta::check("report")
                .label("Report")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("import")
                .label("Import")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("export")
                .label("Export")
                .default_value("0"),
        )
        .field(FieldMeta::tab_break("tab_advanced").label("Advanced"))
        .field(FieldMeta::section_break("section_overrides").label("Overrides"))
        .field(
            FieldMeta::check("if_owner")
                .label("If Owner")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("apply_to_all_doctypes")
                .label("Apply To All DocTypes")
                .default_value("0"),
        )
        .permission(system_manager())
}
