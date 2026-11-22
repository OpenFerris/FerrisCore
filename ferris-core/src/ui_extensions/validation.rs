//! Validation/collection helpers for app-declared UI extensions.

use super::specs::{AppUi, DashboardSpec, RESERVED_ROUTE_PREFIXES, WidgetDefSpec};

/// Widget definitions from a `describe()`: kebab-case names, last declaration
/// wins on duplicates (in place, so declaration order stays stable).
pub(super) fn collect_widget_defs(
    widgets: Vec<WidgetDefSpec>,
    owner: &str,
    warnings: &mut Vec<String>,
) -> Vec<WidgetDefSpec> {
    let mut defs: Vec<WidgetDefSpec> = Vec::new();
    for def in widgets {
        if !is_kebab(&def.name) {
            warnings.push(format!(
                "widget-def `{}` (owner `{owner}`): name must be kebab-case ([a-z0-9][a-z0-9-]*) — dropped",
                def.name
            ));
            continue;
        }
        if let Some(pos) = defs.iter().position(|d| d.name == def.name) {
            warnings.push(format!(
                "widget-def `{}` (owner `{owner}`): duplicate name in the same app — last declaration wins",
                def.name
            ));
            defs[pos] = def;
        } else {
            defs.push(def);
        }
    }
    defs
}

/// Dashboards from a `describe()`: the route must be usable and unique, and
/// `default` is single-registry ("first registered wins").
pub(super) fn collect_dashboards(
    dashboards: Vec<DashboardSpec>,
    owner: &str,
    all: &[AppUi],
    warnings: &mut Vec<String>,
) -> Vec<DashboardSpec> {
    let taken_routes: Vec<String> = all
        .iter()
        .flat_map(|ui| ui.dashboards.iter().map(|d| d.route.clone()))
        .collect();
    let mut default_taken = all.iter().any(|ui| ui.dashboards.iter().any(|d| d.default));
    let mut kept_dashboards: Vec<DashboardSpec> = Vec::new();
    for mut dash in dashboards {
        if let Some(problem) = dashboard_route_problem(&dash.route) {
            warnings.push(format!(
                "dashboard `{}` (owner `{owner}`): {problem} — dropped (route `{}`)",
                dash.id, dash.route
            ));
            continue;
        }
        if taken_routes.contains(&dash.route)
            || kept_dashboards.iter().any(|d| d.route == dash.route)
        {
            warnings.push(format!(
                "dashboard `{}` (owner `{owner}`): route `{}` is already taken — dropped",
                dash.id, dash.route
            ));
            continue;
        }
        if dash.default && default_taken {
            warnings.push(format!(
                "dashboard `{}` (owner `{owner}`): another default dashboard is already registered — `default` ignored",
                dash.id
            ));
            dash.default = false;
        }
        if dash.default {
            default_taken = true;
        }
        kept_dashboards.push(dash);
    }
    kept_dashboards
}

/// Why `route` cannot be registered for a dashboard, or `None` when it can.
///
/// Public so a caller that builds dashboards outside `describe()` — the
/// `Dashboard` `DocType` merge in `ferris-api` — applies the *same* route rules
/// the registry does to a plugin's declaration, instead of a second copy that
/// can drift.
#[must_use]
pub fn dashboard_route_problem(route: &str) -> Option<&'static str> {
    if route.is_empty() {
        return Some("route is empty");
    }
    if !route.starts_with('/') {
        return Some("route must start with `/`");
    }
    if route == "/" {
        return Some("route `/` is reserved for the home view");
    }
    if RESERVED_ROUTE_PREFIXES.iter().any(|prefix| {
        route == *prefix
            || route
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    }) {
        return Some("route shadows a reserved desk prefix (/api, /app, /dashboard, …)");
    }
    None
}

/// Kebab-case app-id/widget-name shape: lowercase alphanumerics and dashes,
/// starting with a lowercase letter or digit.
fn is_kebab(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Resolve a widget reference (`owner.name`, or bare `name` against
/// `default_owner`) against a set of registered UI declarations.
pub(super) fn resolve_widget_in(
    apps: &[AppUi],
    reference: &str,
    default_owner: &str,
) -> Option<(String, WidgetDefSpec)> {
    let (owner, name) = match reference.split_once('.') {
        Some((owner, name)) => (owner, name),
        None => (default_owner, reference),
    };
    apps.iter()
        .find(|ui| ui.owner == owner)
        .and_then(|ui| ui.widgets.iter().find(|w| w.name == name))
        .map(|w| (owner.to_string(), w.clone()))
}
