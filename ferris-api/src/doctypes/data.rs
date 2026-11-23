//! The Data section DocTypes: import/export runs and their logs, document
//! naming rules and settings, and background-prepared reports.
//!
//! They are visible Desk items ([`super::constants::DATA_MODULE`]) unlike the
//! framework feeds, because an operator drives them from the workspace.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::constants::DATA_MODULE;
use super::support::{system_manager, system_manager_single};

/// A data import run against one DocType.
#[must_use]
pub fn data_import_meta() -> DoctypeMeta {
    DoctypeMeta::new("Data Import", DATA_MODULE)
        .description("One run of importing rows into a DocType.")
        .title_field("reference_doctype")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("reference_doctype", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::select("import_type", "Insert New Records\nUpdate Existing Records")
                .label("Import Type")
                .default_value("Insert New Records")
                .in_list_view(),
        )
        .field(FieldMeta::attach("import_file").label("Import File"))
        .field(
            FieldMeta::select("status", "Pending\nSuccess\nPartial Success\nError")
                .label("Status")
                .default_value("Pending")
                .in_list_view(),
        )
        .field(
            FieldMeta::check("mute_emails")
                .label("Mute Emails")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("submit_after_import")
                .label("Submit After Import")
                .default_value("0"),
        )
        .permission(system_manager())
}

/// A data export run for one DocType.
#[must_use]
pub fn data_export_meta() -> DoctypeMeta {
    DoctypeMeta::new("Data Export", DATA_MODULE)
        .description("One run of exporting rows of a DocType.")
        .title_field("reference_doctype")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("reference_doctype", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::select("file_format", "Excel\nCSV")
                .label("File Format")
                .default_value("Excel")
                .in_list_view(),
        )
        .field(FieldMeta::code("fields").label("Fields"))
        .field(FieldMeta::code("filters").label("Filters"))
        .field(
            FieldMeta::select("status", "Pending\nCompleted\nError")
                .label("Status")
                .default_value("Pending")
                .in_list_view(),
        )
        .field(FieldMeta::attach("file").label("File"))
        .permission(system_manager())
}

/// A rule that overrides the naming series of one DocType.
#[must_use]
pub fn document_naming_rule_meta() -> DoctypeMeta {
    DoctypeMeta::new("Document Naming Rule", DATA_MODULE)
        .description("Overrides the naming series of one DocType.")
        .title_field("document_type")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("prefix").label("Prefix").in_list_view())
        .field(
            FieldMeta::int("counter")
                .label("Counter")
                .default_value("0")
                .in_list_view(),
        )
        .field(
            FieldMeta::int("prefix_digits")
                .label("Digits")
                .default_value("5")
                .description("Zero-padding width of the counter."),
        )
        .field(
            FieldMeta::int("priority")
                .label("Priority")
                .default_value("0")
                .in_list_view()
                .description("Higher priority rules are applied first."),
        )
        .field(
            FieldMeta::table("conditions", "Document Naming Rule Condition")
                .label("Conditions")
                .description("All conditions must match for the rule to apply."),
        )
        // Older Frappe releases expressed the same idea with one `apply_if`
        // pair; both shapes are read by the engine.
        .field(
            FieldMeta::select("apply_if", "Fieldname\nCondition")
                .label("Apply If")
                .default_value("Fieldname"),
        )
        .field(FieldMeta::data("fieldname").label("Fieldname"))
        .field(FieldMeta::small_text("condition").label("Condition"))
        .field(
            FieldMeta::check("disabled")
                .label("Disabled")
                .default_value("0"),
        )
        .permission(system_manager())
}

/// One condition of a [`document_naming_rule_meta`] rule.
///
/// Mirrors Frappe's `Document Naming Rule Condition` child table: the engine
/// ANDs every row's `field condition value` before the rule may name a
/// document.
#[must_use]
pub fn document_naming_rule_condition_meta() -> DoctypeMeta {
    DoctypeMeta::new("Document Naming Rule Condition", DATA_MODULE)
        .description("One condition of a Document Naming Rule.")
        .child_table()
        .field(FieldMeta::data("field").label("Field").required())
        .field(
            FieldMeta::select("condition", "=\n!=\n>\n<\n>=\n<=")
                .label("Condition")
                .default_value("=")
                .required(),
        )
        .field(FieldMeta::data("value").label("Value"))
        .permission(system_manager())
}

/// Single: install-wide naming-series defaults.
#[must_use]
pub fn document_naming_settings_meta() -> DoctypeMeta {
    DoctypeMeta::new("Document Naming Settings", DATA_MODULE)
        .description("Install-wide naming-series defaults.")
        .single()
        .sort("modified", "desc")
        .field(FieldMeta::data("default_naming_series").label("Default Naming Series"))
        .permission(system_manager_single())
}

/// One imported row's outcome within a [`data_import_meta`] run.
#[must_use]
pub fn data_import_log_meta() -> DoctypeMeta {
    DoctypeMeta::new("Data Import Log", DATA_MODULE)
        .description("The outcome of one row of a Data Import run.")
        .sort("creation", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("data_import", "Data Import")
                .label("Data Import")
                .in_list_view(),
        )
        .field(
            FieldMeta::int("row_indexes")
                .label("Row Indexes")
                .in_list_view(),
        )
        .field(
            FieldMeta::data("docname")
                .label("Document Name")
                .in_list_view(),
        )
        .field(
            FieldMeta::check("success")
                .label("Success")
                .default_value("0"),
        )
        .field(FieldMeta::text("messages").label("Messages"))
        .field(FieldMeta::text("exception").label("Exception"))
        .permission(system_manager())
}

/// The stored result of a report prepared in the background.
#[must_use]
pub fn prepared_report_meta() -> DoctypeMeta {
    DoctypeMeta::new("Prepared Report", DATA_MODULE)
        .description("A report generated in the background and stored.")
        .title_field("report")
        .sort("creation", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::link("report", "Report")
                .label("Report")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::select("status", "Queued\nCompleted\nError")
                .label("Status")
                .default_value("Queued")
                .in_list_view(),
        )
        .field(FieldMeta::code("filters").label("Filters"))
        .field(FieldMeta::code("columns").label("Columns"))
        .field(FieldMeta::code("data").label("Data"))
        .field(FieldMeta::attach("file").label("File"))
        .field(FieldMeta::text("error").label("Error"))
        .permission(system_manager())
}
