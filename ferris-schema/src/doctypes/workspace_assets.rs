//! DocTypes the `Workspace` declarations link to.
//!
//! Frappe's `Workspace` child tables name dashboard charts, number cards,
//! custom HTML blocks and `DocType Layout`s, and `Workspace` itself names a
//! `Module Onboarding`. The host did not declare those targets, so every one of
//! those links pointed at a DocType the registry did not know: the Desk could
//! store the row but never open its target. These minimal declarations close
//! that hole; the document engine stores them generically and a vertical can
//! grow fields when it starts to render them.

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;
use super::permissions::system_manager;

/// A chart definition a `Workspace Chart` points at.
#[must_use]
pub fn dashboard_chart_meta() -> DoctypeMeta {
    DoctypeMeta::new("Dashboard Chart", BUILD_MODULE)
        .description("A chart definition shown by a workspace or a dashboard.")
        .title_field("chart_name")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("chart_name")
                .label("Chart Name")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::select("chart_type", "Line\nBar\nDonut\nPercentage")
                .label("Chart Type")
                .default_value("Line")
                .in_list_view(),
        )
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(FieldMeta::data("based_on").label("Based On"))
        .field(FieldMeta::data("value_field").label("Value Field"))
        .field(
            FieldMeta::code("filters_json")
                .label("Filters")
                .options("JSON"),
        )
        .field(
            FieldMeta::check("is_public")
                .label("Is Public")
                .default_value("1"),
        )
        .permission(system_manager())
}

/// A number card a `Workspace Number Card` points at.
#[must_use]
pub fn number_card_meta() -> DoctypeMeta {
    DoctypeMeta::new("Number Card", BUILD_MODULE)
        .description("A single aggregate shown as a number card.")
        .title_field("label")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("label")
                .label("Label")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .in_list_view(),
        )
        .field(
            FieldMeta::select("function", "Count\nSum\nAverage\nMinimum\nMaximum")
                .label("Function")
                .default_value("Count")
                .in_list_view(),
        )
        .field(FieldMeta::data("aggregate_function_based_on").label("Aggregate Function Based On"))
        .field(
            FieldMeta::code("filters_json")
                .label("Filters")
                .options("JSON"),
        )
        .field(
            FieldMeta::check("is_public")
                .label("Is Public")
                .default_value("1"),
        )
        .permission(system_manager())
}

/// A reusable HTML block a `Workspace Custom Block` points at.
#[must_use]
pub fn custom_html_block_meta() -> DoctypeMeta {
    DoctypeMeta::new("Custom HTML Block", BUILD_MODULE)
        .description("Reusable HTML, CSS and JavaScript rendered as a workspace block.")
        .title_field("block_name")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("block_name")
                .label("Block Name")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::check("private")
                .label("Private")
                .default_value("0")
                .in_list_view(),
        )
        .field(
            FieldMeta::code("html")
                .label("HTML")
                .options("HTML")
                .required(),
        )
        .field(FieldMeta::code("style").label("CSS").options("CSS"))
        .field(
            FieldMeta::code("script")
                .label("JavaScript")
                .options("JavaScript"),
        )
        .permission(system_manager())
}

/// A form layout a `Workspace Link` can point at.
#[must_use]
pub fn doctype_layout_meta() -> DoctypeMeta {
    DoctypeMeta::new("DocType Layout", BUILD_MODULE)
        .description("A named form layout for one DocType.")
        .title_field("layout_name")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::data("layout_name")
                .label("Layout Name")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("Document Type")
                .required()
                .in_list_view()
                .in_standard_filter(),
        )
        .field(FieldMeta::data("route").label("Route"))
        .field(
            FieldMeta::code("layout")
                .label("Layout")
                .options("JSON")
                .description("Ordered field list and layout markers for this form."),
        )
        .permission(system_manager())
}

/// Onboarding steps attached to a module and surfaced by a `Workspace`.
#[must_use]
pub fn module_onboarding_meta() -> DoctypeMeta {
    DoctypeMeta::new("Module Onboarding", BUILD_MODULE)
        .description("A short guided setup flow for one module.")
        .title_field("title")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("title")
                .label("Title")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::link("module", "Module Def")
                .label("Module")
                .in_list_view()
                .in_standard_filter(),
        )
        .field(FieldMeta::data("route").label("Route"))
        .field(FieldMeta::small_text("success_message").label("Success Message"))
        .field(
            FieldMeta::check("is_complete")
                .label("Is Complete")
                .default_value("0")
                .in_list_view(),
        )
        .permission(system_manager())
}
