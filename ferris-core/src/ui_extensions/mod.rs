//! OpenUI extensions: app-declared slots, dashboards, widget catalog and website routes.
//!
//! The host defines *slots* — designated mount points in its views
//! (`form.toolbar.right`, `list.before_table`, `shell.sidebar.bottom`, …).
//! Apps decide which slot they want to extend and declare a Web Component
//! (or, in a later phase, an iframe) to mount there.
//!
//! Widgets are a two-level contract (Fase 3):
//!
//! - a **`WidgetDefSpec`** is a reusable *definition* an app publishes to the
//!   global catalog (`widget-def` in `describe()`), with the parameters it
//!   accepts;
//! - a **`WidgetInstanceSpec`** is one *placement* of a definition inside a
//!   dashboard, carrying its own parameter values and grid span. References
//!   are `owner.name` (or bare `name` for the dashboard's own app), so any
//!   app — including the host itself — can instance widgets published by
//!   anyone else.
//!
//! The host publishes its own definitions too ([`host_widget_defs`], owner
//! `"host"`): the parametrized replacements for the dashboards that used to
//! be hardcoded views.
//!
//! Apps register their declarations either by calling
//! [`register_app_ui`] at startup, or (the normal path) by declaring
//! `"ui_extensions"`, `"dashboards"`, `"widgets"` and `"website"` arrays in
//! the plugin's `describe()` — the framework registers them when the plugin
//! loads. Registration is keyed by an owner (the plugin/app id): calling it
//! again with the same owner atomically replaces that app's previous set, so
//! hot-reloading a plugin never leaves stale UI declarations behind.
//!
//! `GET /api/ui-extensions` (in `ferris-api`) exposes every registered set to
//! the Desk, which mounts the declared components into the slots they name
//! and resolves widget references against the catalog.
//!
//! The store itself lives in `crate::registry::OwnerRegistry`, the shared
//! primitive also used by `workspaces`.

mod access;
mod host_widgets;
mod registry;
mod specs;
mod validation;

pub use access::{
    extension_for, find_widget, find_widgets, has_ui_extensions, registered_dashboards,
    registered_extensions, registered_ui, registered_websites, registered_widgets, widget_style,
    widget_style_by_reference,
};
pub use host_widgets::{host_widget_defs, register_host_widgets};
pub use registry::{Registration, register_app_ui, replace_widgets};
pub use specs::{
    AppUi, AppUiSpec, DashboardSpec, DashboardStyle, GridSizeSpec, StyleSpec, UiExtensionSpec,
    WebsiteRouteSpec, WebsiteSpec, WidgetDefSpec, WidgetInstanceSpec, WidgetParamSpec,
};
pub use validation::dashboard_route_problem;

#[cfg(test)]
mod tests;
