//! The Data section DocTypes: import/export runs and their logs, document
//! naming rules and settings, and background-prepared reports.
//!
//! They are visible Desk items ([`super::constants::DATA_MODULE`]) unlike the
//! framework feeds, because an operator drives them from the workspace.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::constants::DATA_MODULE;
use super::support::{system_manager, system_manager_single};

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
