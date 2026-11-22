//! The process-wide UI-extension store and its registration entry point.

use crate::registry::OwnerRegistry;

use super::specs::{AppUi, AppUiSpec, WidgetDefSpec};
use super::validation::{collect_dashboards, collect_widget_defs, resolve_widget_in};

/// Registered UI sets, in registration order.
pub(super) static UI: OwnerRegistry<AppUi> = OwnerRegistry::new();

/// Outcome of a registration: the registered component names plus any
/// validation warnings the caller should surface.
///
/// Warnings describe declarations that were dropped or corrected — the
/// registration itself always succeeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registration {
    /// Custom element names of the registered slot extensions.
    pub components: Vec<String>,
    /// Human-readable validation messages (dropped routes, duplicate names,
    /// unresolved widget references, …).
    pub warnings: Vec<String>,
}

/// Register (or replace) the UI extension set owned by `owner`.
///
/// Re-calling with the same owner atomically removes that owner's previous set
/// before inserting the new one, so hot-reloaded plugins never leave stale
/// slot/dashboard/widget/website declarations behind — including the case
/// where the reloaded `describe()` declares nothing at all.
///
/// The declaration is validated while it is inserted (reserved/duplicate
/// dashboard routes, the single `default` flag, kebab-case widget names,
/// unresolved widget references); invalid pieces are dropped and reported in
/// [`Registration::warnings`] instead of failing the registration.
pub fn register_app_ui(owner: &str, spec: AppUiSpec) -> Registration {
    let owner = owner.trim();
    let mut warnings = Vec::new();
    let components: Vec<String> = spec
        .extensions
        .iter()
        .map(|e| e.component.clone())
        .collect();

    let registered = UI.write(|all| {
        // Replace everything this owner previously registered FIRST, so the
        // validations below never collide with the owner's own old set (a
        // default dashboard re-registered under the same owner keeps its flag).
        all.retain(|ui| ui.owner != owner);

        if spec.is_empty() {
            // Nothing to declare: the retain above already dropped the
            // previous set. Keep the flag as-is — it is only a cheap gate.
            return false;
        }

        let AppUiSpec {
            extensions,
            dashboards,
            widgets,
            website,
            assets,
        } = spec;
        let defs = collect_widget_defs(widgets, owner, &mut warnings);
        let kept_dashboards = collect_dashboards(dashboards, owner, all.as_slice(), &mut warnings);

        all.push(AppUi {
            owner: owner.to_string(),
            extensions,
            dashboards: kept_dashboards,
            widgets: defs,
            website,
            assets,
        });

        // Unresolved widget references: report them (the Desk renders a
        // placeholder either way), but do not drop the dashboard — the
        // referenced app may register later.
        let my_instances: Vec<(String, String)> = all
            .iter()
            .find(|ui| ui.owner == owner)
            .map(|ui| {
                ui.dashboards
                    .iter()
                    .flat_map(|d| {
                        d.widgets
                            .iter()
                            .map(move |w| (d.id.clone(), w.widget.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (dash_id, reference) in my_instances {
            if resolve_widget_in(all.as_slice(), &reference, owner).is_none() {
                warnings.push(format!(
                    "dashboard `{dash_id}` (owner `{owner}`): unresolved widget reference `{reference}` — no widget-def declares it (yet)"
                ));
            }
        }
        true
    });
    if registered {
        UI.mark_registered();
    }
    Registration {
        components,
        warnings,
    }
}

#[cfg(test)]
pub(super) fn reset_registry() {
    UI.reset();
}

/// Replace only the widget catalog of `owner`, leaving its slots, dashboards,
/// website routes and assets untouched.
///
/// This is the cache-write half of the `Widget` DocType migration: the API
/// reads the rows (the source of truth) and republishes them here so the
/// synchronous layout/validation helpers ([`super::find_widgets`]) see the
/// stored definitions, including an admin's edits. An owner with no previous
/// registration gets a widget-only entry, which is how a user-created widget
/// (app `custom`, no plugin) still resolves.
pub fn replace_widgets(owner: &str, widgets: Vec<WidgetDefSpec>) {
    let owner = owner.trim();
    let populated = !widgets.is_empty();
    UI.write(|all| {
        if let Some(ui) = all.iter_mut().find(|ui| ui.owner == owner) {
            ui.widgets = widgets;
        } else if populated {
            all.push(AppUi {
                owner: owner.to_string(),
                widgets,
                ..AppUi::default()
            });
        }
    });
    if populated {
        UI.mark_registered();
    }
}
