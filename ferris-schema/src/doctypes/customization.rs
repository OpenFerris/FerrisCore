//! Customization and form-designer DocTypes (Desk section **Build**).

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;

use super::permissions::{system_manager, system_manager_single};

/// Single: the working row of the form designer.
#[must_use]
pub fn customize_form_meta() -> DoctypeMeta {
    DoctypeMeta::new("Customize Form", BUILD_MODULE)
        .single()
        .description("Form designer settings for one DocType.")
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(FieldMeta::link("doc_type", "DocType").label("Document Type"))
        .field(FieldMeta::data("image_field").label("Image Field"))
        .field(FieldMeta::int("max_attachments").label("Max Attachments"))
        .field(FieldMeta::check("allow_copy").label("Allow Copy"))
        .field(FieldMeta::check("quick_entry").label("Quick Entry"))
        .field(FieldMeta::check("track_changes").label("Track Changes"))
        .permission(system_manager_single())
}

/// A field added to a DocType without changing its Rust definition.
#[must_use]
pub fn custom_field_meta() -> DoctypeMeta {
    DoctypeMeta::new("Custom Field", BUILD_MODULE)
        .description("A custom field added to an existing DocType.")
        .title_field("label")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("dt", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
        .field(FieldMeta::data("fieldname").label("Fieldname"))
        .field(FieldMeta::data("fieldtype").label("Field Type"))
        .field(FieldMeta::text("options").label("Options"))
        .field(FieldMeta::text("default").label("Default"))
        .field(FieldMeta::check("in_list_view").label("In List View"))
        .field(FieldMeta::check("in_standard_filter").label("In Standard Filter"))
        .field(
            FieldMeta::check("is_custom_field")
                .label("Is Custom Field")
                .default_value("1"),
        )
        .permission(system_manager())
}

/// An override of a standard property of a DocType or one of its fields.
#[must_use]
pub fn property_setter_meta() -> DoctypeMeta {
    DoctypeMeta::new("Property Setter", BUILD_MODULE)
        .description("An override of a standard metadata property.")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("doc_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(FieldMeta::data("doc_name").label("Document Name"))
        .field(
            FieldMeta::data("field_name")
                .label("Field Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("property").label("Property").in_list_view())
        .field(FieldMeta::text("value").label("Value"))
        .permission(system_manager())
}

/// A translation of a source string into one language.
#[must_use]
pub fn custom_translation_meta() -> DoctypeMeta {
    DoctypeMeta::new("Custom Translation", BUILD_MODULE)
        .description("An operator-supplied translation for a source string.")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::link("language", "Language")
                .label("Language")
                .in_list_view(),
        )
        .field(FieldMeta::text("source_text").label("Source Text"))
        .field(FieldMeta::text("translated_text").label("Translated Text"))
        .permission(system_manager())
}

/// Single: the announcement and help link shown in the navbar.
#[must_use]
pub fn navbar_settings_meta() -> DoctypeMeta {
    DoctypeMeta::new("Navbar Settings", BUILD_MODULE)
        .single()
        .description("Announcement and help link shown in the navbar.")
        .field(FieldMeta::data("announcement").label("Announcement"))
        .field(FieldMeta::data("logo").label("Logo"))
        .field(FieldMeta::data("help_link").label("Help Link"))
        .permission(system_manager_single())
}

/// A site-specific permission override for one `(DocType, role, permlevel)`.
///
/// This is the "Custom DocPerm" half of Frappe's Customize page
/// (<https://docs.frappe.io/framework/user/en/basics/doctypes/customize>): the
/// standard `DocPerm` rows ship with the app, while a tenant's overrides live
/// here and are merged over them by the API's effective metadata. `select` marks
/// an override that replaces the standard rows for the same pair entirely.
///
/// `custom_permissions` is a JSON array of [`Permission Type`](https://docs.frappe.io/framework/permission-types)
/// names (`["approve", "download"]`). It is a text column because the runtime
/// rule type holds `Vec<String>` and a child table would add a doctype for a
/// list that is always read as a whole.
#[must_use]
pub fn custom_doc_perm_meta() -> DoctypeMeta {
    DoctypeMeta::new("Custom DocPerm", BUILD_MODULE)
        .description("Site-specific permission override for one DocType and role.")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::tab_break("tab_target").label("Target"))
        .field(FieldMeta::section_break("section_target").label("Target"))
        .field(
            FieldMeta::link("parent", "DocType")
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
                .label("Perm Level")
                .default_value("0"),
        )
        .field(FieldMeta::tab_break("tab_rights").label("Rights"))
        .field(FieldMeta::section_break("section_document_rights").label("Document Rights"))
        .field(right("read", "Read"))
        .field(right("write", "Write"))
        .field(right("create", "Create"))
        .field(right("delete", "Delete"))
        .field(FieldMeta::section_break("section_lifecycle").label("Lifecycle"))
        .field(right("submit", "Submit"))
        .field(right("cancel", "Cancel"))
        .field(right("amend", "Amend"))
        .field(FieldMeta::section_break("section_sharing").label("Sharing and Reporting"))
        .field(right("print", "Print"))
        .field(right("email", "Email"))
        .field(right("share", "Share"))
        .field(right("report", "Report"))
        .field(right("import", "Import"))
        .field(right("export", "Export"))
        .field(FieldMeta::tab_break("tab_advanced").label("Advanced"))
        .field(FieldMeta::section_break("section_overrides").label("Overrides"))
        .field(right("if_owner", "If Owner"))
        .field(
            FieldMeta::check("select")
                .label("Select")
                .default_value("0"),
        )
        .field(
            FieldMeta::small_text("custom_permissions")
                .label("Custom Permissions")
                .description("JSON array of Permission Type names."),
        )
        .permission(system_manager())
}

/// One `Check` right of [`custom_doc_perm_meta`], defaulting to off.
fn right(fieldname: &str, label: &str) -> FieldMeta {
    FieldMeta::check(fieldname).label(label).default_value("0")
}
