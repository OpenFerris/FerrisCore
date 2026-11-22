//! The host-owned reusable widget catalog.

use super::registry::{Registration, register_app_ui};
use super::specs::{AppUiSpec, GridSizeSpec, StyleSpec, WidgetDefSpec, WidgetParamSpec};

/// Build a [`WidgetParamSpec`] concisely (used by [`host_widget_defs`] and tests).
///
/// `group` is the properties-dialog section the parameter belongs to; the
/// builder renders the form grouped by it, so a widget with three data
/// parameters and two presentation ones reads as two sections instead of one
/// flat list.
#[allow(clippy::too_many_arguments)] // a spec literal, not a callable API
pub(super) fn param(
    name: &str,
    label: &str,
    group: &str,
    value_type: &str,
    default: Option<&str>,
    options: &[&str],
    required: bool,
) -> WidgetParamSpec {
    WidgetParamSpec {
        name: name.into(),
        label: Some(label.into()),
        value_type: value_type.into(),
        default: default.map(str::to_string),
        options: options.iter().map(ToString::to_string).collect(),
        required,
        description: None,
        group: Some(group.into()),
        placeholder: None,
        depends_on: None,
        read_only: false,
        hidden: false,
    }
}

/// The footprint hint a host widget declares alongside its parameters.
const fn size(span: u8, rows: u8) -> GridSizeSpec {
    GridSizeSpec { span, rows }
}

/// The appearance a host widget declares as its default.
///
/// These are the values the host's own cards have always rendered with, now
/// stated where a user can see and override them: a card surface, the theme's
/// border, the standard radius and an inner padding sized to the host's
/// spacing scale. A user overriding one field of a host widget therefore
/// starts from exactly what the widget looked like before this feature.
fn style(background: &str, padding: u8) -> StyleSpec {
    StyleSpec {
        background: Some(background.into()),
        border_color: Some("border".into()),
        border_width: Some(1),
        border_radius: Some(8),
        padding: Some(padding),
        shadow: Some("xs".into()),
        ..StyleSpec::default()
    }
}

/// The host's own reusable widget catalog.
///
/// The parametrized replacements for the dashboards that used to be hardcoded
/// views (`/` landing and `/dashboard/:name`). Registered under owner `"host"`
/// by `ferris-server::build_router`; rendered natively by the Desk — they
/// never travel the `/plugins/*` asset or SSR channels.
#[must_use]
pub fn host_widget_defs() -> Vec<WidgetDefSpec> {
    vec![
        WidgetDefSpec {
            name: "stat".into(),
            label: "Stat".into(),
            component: "host-stat".into(),
            kind: "stat".into(),
            params: vec![
                param("doctype", "DocType", "Data", "doctype", None, &[], true),
                param("title", "Title", "Presentation", "string", None, &[], false),
                param(
                    "docstatus",
                    "DocStatus",
                    "Data",
                    "string",
                    Some("all"),
                    &["0", "1", "all"],
                    false,
                ),
            ],
            description: Some(
                "Count of documents of a DocType (all, drafts or submitted) as a stat card.".into(),
            ),
            requires: None,
            events: Vec::new(),
            icon: Some("🔢".into()),
            category: Some("Metrics".into()),
            preview: Some("/static/previews/stat.svg".into()),
            size: Some(size(3, 2)),
            min: Some(size(2, 2)),
            style: Some(style("card", 12)),
            renderer: Some("stat".into()),
            version: Some("1".into()),
            code_path: None,
            script: None,
            stylesheet: None,
            ssr: None,
        },
        WidgetDefSpec {
            name: "trend".into(),
            label: "Trend".into(),
            component: "host-trend".into(),
            kind: "chart".into(),
            params: vec![
                param("doctype", "DocType", "Data", "doctype", None, &[], true),
                param("title", "Title", "Presentation", "string", None, &[], false),
                param("days", "Days", "Data", "number", Some("30"), &[], false),
            ],
            description: Some("Documents created per day over the last N days, as a chart.".into()),
            requires: None,
            events: Vec::new(),
            icon: Some("📈".into()),
            category: Some("Charts".into()),
            preview: Some("/static/previews/chart.svg".into()),
            size: Some(size(6, 4)),
            min: Some(size(3, 3)),
            style: Some(style("card", 12)),
            renderer: Some("trend".into()),
            version: Some("1".into()),
            code_path: None,
            script: None,
            stylesheet: None,
            ssr: None,
        },
        WidgetDefSpec {
            name: "table".into(),
            label: "Recent rows".into(),
            component: "host-table".into(),
            kind: "table".into(),
            params: vec![
                param("doctype", "DocType", "Data", "doctype", None, &[], true),
                param("title", "Title", "Presentation", "string", None, &[], false),
                param("limit", "Rows", "Data", "number", Some("10"), &[], false),
                param(
                    "order",
                    "Order",
                    "Data",
                    "string",
                    Some("desc"),
                    &["asc", "desc"],
                    false,
                ),
            ],
            description: Some("The most recent documents of a DocType, newest first.".into()),
            requires: None,
            events: Vec::new(),
            icon: Some("🧾".into()),
            category: Some("Data".into()),
            preview: Some("/static/previews/table.svg".into()),
            size: Some(size(12, 5)),
            min: Some(size(6, 3)),
            style: Some(style("card", 12)),
            renderer: Some("table".into()),
            version: Some("1".into()),
            code_path: None,
            script: None,
            stylesheet: None,
            ssr: None,
        },
    ]
}

/// Register the host's widget catalog (owner `"host"`). Called on every
/// `build_router`; idempotent, like `register_core_doctypes`.
#[must_use]
pub fn register_host_widgets() -> Registration {
    register_app_ui(
        "host",
        AppUiSpec {
            widgets: host_widget_defs(),
            ..AppUiSpec::default()
        },
    )
}
