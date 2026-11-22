//! The declared seed of a dashboard and its existence check.
//!
//! A dashboard is declared either by a plugin (`ui_extensions`) or by a
//! [`crate::dashboards`] document. Existence and the seed are answered
//! separately because a dashboard with zero widgets is still a dashboard — the
//! distinction that keeps a zero-widget plugin board from being answered by the
//! `Dashboard` `DocType` instead.

use ferris_db::Db;
use ferris_schema::contract::WidgetInstance;

use super::layout::{normalize_widgets, style_spec};

/// The registry's declared seed of one dashboard, or `None` when no app with
/// that owner declares a dashboard of that id.
///
/// Distinct from "empty": a plugin dashboard may legitimately declare zero
/// widgets, and that must not make [`crate::dashboards`] answer for it.
pub(super) fn registry_seed(owner: &str, dashboard: &str) -> Option<Vec<WidgetInstance>> {
    ferris_core::ui_extensions::registered_ui()
        .into_iter()
        .find(|app| app.owner == owner)
        .and_then(|app| app.dashboards.into_iter().find(|d| d.id == dashboard))
        .map(|declared| {
            declared
                .widgets
                .into_iter()
                .map(|w| WidgetInstance {
                    id: w.id,
                    widget: w.widget,
                    label: w.label,
                    // A seed carries no chrome configuration: the host's
                    // defaults apply until the Desk edits the instance. The
                    // WIT instance deliberately stays unchanged (the app
                    // declares content, the host decides how to frame it).
                    show_title: None,
                    subtitle: None,
                    show_subtitle: None,
                    params: w.params,
                    span: w.span,
                    col: w.col,
                    row: w.row,
                    rows: w.rows,
                    // The seed carries the style the instance declares; an
                    // absent one stays absent and the Desk layers the
                    // definition's default, then the host's, on top of it.
                    style: w.style.map(style_spec),
                })
                .collect()
        })
}

/// Whether the app/dashboard pair exists at all, for the `PUT` gate: a
/// dashboard is addressable when a plugin declares it **or** a `Dashboard`
/// document defines it. Existence is checked separately from the seed because a
/// dashboard with zero widgets is still a dashboard.
pub(super) async fn dashboard_exists(pool: &Db, owner: &str, dashboard: &str) -> bool {
    if registry_seed(owner, dashboard).is_some() {
        return true;
    }
    owner == crate::dashboards::OWNER
        && crate::dashboards::dashboard_widgets(pool, dashboard)
            .await
            .is_some()
}

/// The declared seed of one dashboard: the registry entry's instances for a
/// plugin dashboard, or the widgets of the `Dashboard` document for the
/// [`crate::dashboards::OWNER`] owner, laid out by the shared placement rule.
/// Empty when neither declares it.
pub(super) async fn seed_widgets(pool: &Db, owner: &str, dashboard: &str) -> Vec<WidgetInstance> {
    let mut widgets = match registry_seed(owner, dashboard) {
        Some(widgets) => widgets,
        None if owner == crate::dashboards::OWNER => {
            crate::dashboards::dashboard_widgets(pool, dashboard)
                .await
                .unwrap_or_default()
        }
        None => Vec::new(),
    };
    normalize_widgets(owner, &mut widgets);
    widgets
}
