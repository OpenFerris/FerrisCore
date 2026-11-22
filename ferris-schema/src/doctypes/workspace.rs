//! The `Workspace` DocType and its child tables (Desk section **Build**).

use crate::builders::RolePermission;
use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;

/// Full read/write/print/export/share/report for a desk role.
///
/// `Workspace` grants the same rights to both desk roles in Frappe, so this is
/// the shared rule for `Workspace Manager` and `Desk User`.
#[must_use]
fn workspace_role(role: &str) -> RolePermission {
    RolePermission::new(role)
        .read()
        .write()
        .create()
        .delete()
        .print()
        .email()
        .export()
        .share()
        .report()
}

/// The `Workspace` DocType — a Desk workspace definition.
///
/// This is the faithful projection of Frappe's `desk/doctype/workspace`
/// (`workspace.json`): the same fields, the same six child tables and the same
/// two desk-role permissions. The compiled `ferris_core::workspaces` registry
/// stays the in-code seed (see `ferris-api::workspace_store`), and a `Workspace`
/// document is the persisted, inspectable form of one declared workspace.
///
/// Two deliberate deviations from Frappe:
///
/// * the layout-only breaks (`Tab Break`/`Section Break`/`Column Break`) are
///   omitted, as in every other ferris declaration — a declaration describes
///   data, not form grouping;
/// * the document is named after the ferris workspace id (`data`, `build`, …)
///   rather than `autoname: field:label`, because the id is the stable key the
///   registry, the grid and every saved arrangement already use.
#[must_use]
pub fn workspace_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace", BUILD_MODULE)
        .description("A Desk workspace: a group of links, shortcuts and dashboards.")
        .title_field("label")
        .sort("modified", "desc")
        .allow_rename()
        .track_changes()
        .allow_delete()
        .fields(workspace_fields())
        .fields(workspace_child_fields())
        .permissions([
            workspace_role("Workspace Manager"),
            workspace_role("Desk User"),
        ])
}

/// The scalar fields of `Workspace`.
///
/// Only the layout-only breaks are missing; see [`workspace_meta`].
fn workspace_fields() -> Vec<FieldMeta> {
    vec![
        // Identity and destination.
        FieldMeta::data("label")
            .label("Name")
            .required()
            .unique()
            .in_list_view(),
        // `Icon` used to be the fieldtype here: a name no producer of the
        // framework emits and that is not in Frappe's list either, so with the allowlist enforced it would now be rejected at
        // registration. The value is a plain icon name, hence `Data`.
        FieldMeta::data("icon").label("Icon"),
        FieldMeta::data("title").label("Title").required().in_list_view(),
        FieldMeta::float("sequence_id").label("Sequence Id"),
        FieldMeta::data("for_user").label("For User"),
        FieldMeta::link("parent_page", "Workspace").label("Parent Page"),
        FieldMeta::link("module", "Module Def")
            .label("Module")
            .required()
            .in_list_view()
            .in_standard_filter()
            .description(
                "The module this workspace belongs to. Decides which module's dock \
                 entry and sidebar it appears under.",
            ),
        // Workspace vs. a plain link tile.
        FieldMeta::select("type", "Workspace\nLink\nURL")
            .label("Type")
            .default_value("Workspace")
            .required(),
        FieldMeta::select("link_type", "DocType\nPage\nReport").label("Link Type"),
        FieldMeta::dynamic_link("link_to", "link_type").label("Link To"),
        FieldMeta::data("external_link")
            .label("External Link")
            .options("URL"),
        FieldMeta::check("standard").label("Standard").default_value("0"),
        FieldMeta::select(
            "indicator_color",
            "green\ncyan\nblue\norange\nyellow\ngray\ngrey\nred\npink\ndarkgrey\npurple\nlight-blue",
        )
        .label("Indicator Color"),
        FieldMeta::link("restrict_to_domain", "Domain").label("Restrict to Domain"),
        FieldMeta::check("hide_custom")
            .label("Hide Custom DocTypes and Reports")
            .default_value("0")
            .description(
                "Checking this will hide custom doctypes and reports cards in the \
                 Links section",
            ),
        FieldMeta::check("public")
            .label("Public")
            .default_value("0")
            .read_only()
            .in_list_view()
            .in_standard_filter(),
        FieldMeta::check("is_hidden")
            .label("Is Hidden")
            .default_value("0"),
        FieldMeta::new("content", "Long Text")
            .label("Content")
            .hidden()
            .default_value("[]"),
        FieldMeta::link("module_onboarding", "Module Onboarding").label("Module Onboarding"),
    ]
}

/// The child tables of `Workspace`: its six Frappe tables plus the `roles`
/// (`Has Role`) list.
fn workspace_child_fields() -> Vec<FieldMeta> {
    vec![
        FieldMeta::table("number_cards", "Workspace Number Card").label("Number Cards"),
        FieldMeta::table("charts", "Workspace Chart").label("Charts"),
        FieldMeta::table("shortcuts", "Workspace Shortcut").label("Shortcuts"),
        FieldMeta::table("links", "Workspace Link").label("Links"),
        FieldMeta::table("quick_lists", "Workspace Quick List").label("Quick Lists"),
        FieldMeta::table("custom_blocks", "Workspace Custom Block").label("Custom Blocks"),
        FieldMeta::table("roles", "Has Role").label("Roles"),
    ]
}

/// The `Workspace Link` child table: one link card in a workspace sidebar.
///
/// Frappe's `desk/doctype/workspace_link`. A `type` of `Card Break` starts a new
/// link card; the `Link` rows that follow belong to it.
#[must_use]
pub fn workspace_link_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace Link", BUILD_MODULE)
        .description("One link or card break in a workspace.")
        .child_table()
        .field(
            FieldMeta::select("type", "Link\nCard Break")
                .label("Type")
                .default_value("Link")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("label")
                .label("Label")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("icon").label("Icon"))
        .field(FieldMeta::html("description").label("Description"))
        .field(
            FieldMeta::check("hidden")
                .label("Hidden")
                .default_value("0"),
        )
        .field(
            FieldMeta::select("link_type", "DocType\nPage\nReport")
                .label("Link Type")
                .in_list_view(),
        )
        .field(
            FieldMeta::dynamic_link("link_to", "link_type")
                .label("Link To")
                .in_list_view(),
        )
        .field(FieldMeta::link("doctype_layout", "DocType Layout").label("DocType Layout"))
        .field(FieldMeta::data("dependencies").label("Dependencies"))
        .field(FieldMeta::link("only_for", "Country").label("Only for"))
        .field(
            FieldMeta::check("onboard")
                .label("Onboard")
                .default_value("0"),
        )
        .field(
            FieldMeta::check("is_query_report")
                .label("Is Query Report")
                .default_value("0"),
        )
        .field(FieldMeta::int("link_count").label("Link Count").hidden())
        .field(
            FieldMeta::link("report_ref_doctype", "DocType")
                .label("Report Ref DocType")
                .read_only(),
        )
}

/// The `Workspace Shortcut` child table: a shortcut tile.
///
/// Frappe's `desk/doctype/workspace_shortcut`.
#[must_use]
pub fn workspace_shortcut_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace Shortcut", BUILD_MODULE)
        .description("One shortcut tile in a workspace.")
        .child_table()
        .field(
            FieldMeta::select("type", "DocType\nReport\nPage\nDashboard\nURL")
                .label("Type")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::dynamic_link("link_to", "type")
                .label("Link To")
                .in_list_view(),
        )
        .field(FieldMeta::data("url").label("URL").in_list_view())
        .field(
            FieldMeta::select(
                "doc_view",
                "\nList\nReport Builder\nDashboard\nTree\nNew\nCalendar\nKanban\nImage",
            )
            .label("DocType View")
            .in_list_view()
            .description("Which view of the associated DocType should this shortcut take you to?"),
        )
        .field(FieldMeta::link("kanban_board", "Kanban Board").label("Kanban Board"))
        .field(
            FieldMeta::data("label")
                .label("Label")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("icon").label("Icon"))
        .field(FieldMeta::link("restrict_to_domain", "Domain").label("Restrict to Domain"))
        .field(
            FieldMeta::link("report_ref_doctype", "DocType")
                .label("Report Ref DocType")
                .read_only(),
        )
        .field(
            FieldMeta::code("stats_filter")
                .label("Count Filter")
                .options("JSON"),
        )
        .field(FieldMeta::new("color", "Color").label("Color"))
        .field(
            FieldMeta::data("format")
                .label("Format")
                .description("For example: {} Open"),
        )
}

/// The `Workspace Chart` child table: a dashboard chart shown in a workspace.
///
/// Frappe's `desk/doctype/workspace_chart`.
#[must_use]
pub fn workspace_chart_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace Chart", BUILD_MODULE)
        .description("A dashboard chart shown in a workspace.")
        .child_table()
        .field(
            FieldMeta::link("chart_name", "Dashboard Chart")
                .label("Chart Name")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
}

/// The `Workspace Number Card` child table: a number card shown in a workspace.
///
/// Frappe's `desk/doctype/workspace_number_card`.
#[must_use]
pub fn workspace_number_card_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace Number Card", BUILD_MODULE)
        .description("A number card shown in a workspace.")
        .child_table()
        .field(
            FieldMeta::link("number_card_name", "Number Card")
                .label("Number Card Name")
                .required()
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
}

/// The `Workspace Quick List` child table: a filtered list shown in a workspace.
///
/// Frappe's `desk/doctype/workspace_quick_list`.
#[must_use]
pub fn workspace_quick_list_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace Quick List", BUILD_MODULE)
        .description("A filtered list shown in a workspace.")
        .child_table()
        .field(
            FieldMeta::link("document_type", "DocType")
                .label("DocType")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::data("label")
                .label("Label")
                .required()
                .in_list_view(),
        )
        .field(
            FieldMeta::code("quick_list_filter")
                .label("Quick List Filter")
                .options("JSON")
                .in_list_view(),
        )
}

/// The `Workspace Custom Block` child table: a custom HTML block.
///
/// Frappe's `desk/doctype/workspace_custom_block`.
#[must_use]
pub fn workspace_custom_block_meta() -> DoctypeMeta {
    DoctypeMeta::new("Workspace Custom Block", BUILD_MODULE)
        .description("A custom HTML block shown in a workspace.")
        .child_table()
        .field(
            FieldMeta::link("custom_block_name", "Custom HTML Block")
                .label("Custom Block Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("label").label("Label").in_list_view())
}
