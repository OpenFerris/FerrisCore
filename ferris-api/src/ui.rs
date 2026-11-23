//! `OpenUI` extensions: API surface for the slot/dashboard/widget/website registry.
//!
//! The registry itself lives in `ferris-core`
//! ([`ferris_core::ui_extensions`]) so the plugin host (`ferris-wasm`) can
//! register UI declarations without a dependency cycle. This module owns the
//! `GET /api/ui-extensions` handler.
//!
//! Two sets that used to be registry-only are now read from `DocType` rows on every
//! request:
//!
//! - **widgets** — [`crate::widgets`] migrates `describe().widgets` into the
//!   `Widget` `DocType` and publishes the stored rows. The `DocType` is the source
//!   of truth; an admin edit reaches the Desk on the next fetch.
//! - **dashboards** — [`crate::dashboards`] reads every `Dashboard` row; the
//!   rows arrive here as one extra [`contract::AppUi`].
//!
//! The response is an envelope ([`contract::UiFeed`]): `apps` plus a
//! `revision` fingerprint of the whole payload, so the Desk can skip
//! refetching while nothing changed and notice hot reloads, a saved dashboard
//! document or an edited widget when it did.
//!
//! # Failure policy
//!
//! Each database read is additive: if the `Widget` or `Dashboard` table cannot
//! be read, its registry set keeps serving and the failure is logged. The Desk
//! never loses the whole feed because one `DocType` is unavailable.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use ferris_core::i18n::{I18n, Locale, MetaTranslate};
use ferris_core::ui_extensions as registry;
use ferris_schema::contract;

use crate::ApiState;
use crate::router::RequestLocale;

/// `GET /api/ui-extensions` — every app's registered slot extensions,
/// dashboards, widget catalog and website routes, for the Desk to mount.
///
/// The widget migration runs first (idempotent, hash-guarded), then the stored
/// widgets and dashboards are read. Both reads are additive.
pub async fn ui_extensions(
    State(state): State<Arc<ApiState>>,
    RequestLocale(locale): RequestLocale,
) -> Json<contract::UiFeed> {
    // A failed migration keeps the registry's seed: publishing a table that is
    // missing the rows that failed to upsert would silently drop widgets.
    let widgets = if crate::widgets::sync(&state.pool).await {
        crate::widgets::list_apps(&state.pool).await
    } else {
        None
    };
    let documents = crate::dashboards::db_app(&state.pool)
        .await
        .into_iter()
        .collect();
    let mut feed = compose(documents, widgets);
    localize_feed(&mut feed, &state.i18n, &locale);
    // The fingerprint covers the localized payload: two languages are two
    // bodies, and a client that switches language must not keep the previous
    // one because the revision looked unchanged.
    feed.revision = contract::revision_of(&serde_json::to_vec(&feed.apps).unwrap_or_default());
    Json(feed)
}

/// Translate dashboard, widget, parameter and extension labels in place.
///
/// Dashboards and widgets derive their key from their id/name; parameter and
/// extension labels are free-form, so they are translated only when they are
/// themselves keys (`app.<app>.<section>`), which is how an app names its own
/// strings without the host knowing the shape of its catalogue.
pub fn localize_feed(feed: &mut contract::UiFeed, i18n: &I18n, locale: &Locale) {
    let t = MetaTranslate::new(i18n, locale);
    for app in &mut feed.apps {
        for extension in &mut app.extensions {
            if let Some(label) = extension.label.take() {
                extension.label = Some(t.literal(&label).into_owned());
            }
        }
        for dashboard in &mut app.dashboards {
            dashboard.label = t
                .dashboard_label(&dashboard.id, &dashboard.label)
                .into_owned();
            for widget in &mut dashboard.widgets {
                if let Some(label) = widget.label.take() {
                    widget.label = Some(t.literal(&label).into_owned());
                }
            }
        }
        for widget in &mut app.widgets {
            widget.label = t.widget_label(&widget.name, &widget.label).into_owned();
            if let Some(description) = widget.description.take() {
                widget.description = Some(t.literal(&description).into_owned());
            }
            for param in &mut widget.params {
                if let Some(label) = param.label.take() {
                    param.label = Some(t.literal(&label).into_owned());
                }
            }
        }
    }
}

/// Merge the stored widget catalog into the registry's sets.
///
/// A `None` widget set means the `DocType` read failed: the registry's seed keeps
/// serving. A `Some` set replaces each registry app's widgets — including with
/// an empty list, so disabling the last row really removes it — and appends the
/// owners that exist only in the table (the host and user-created widgets).
fn compose(
    documents: Vec<contract::AppUi>,
    widget_apps: Option<Vec<contract::AppUi>>,
) -> contract::UiFeed {
    let mut apps = registered_ui();
    if let Some(widget_apps) = widget_apps {
        let mut by_owner: BTreeMap<String, contract::AppUi> = widget_apps
            .into_iter()
            .map(|app| (app.owner.clone(), app))
            .collect();
        for app in &mut apps {
            app.widgets = by_owner
                .remove(&app.owner)
                .map(|stored| stored.widgets)
                .unwrap_or_default();
        }
        apps.extend(by_owner.into_values());
    }
    apps.extend(documents);
    let bytes = serde_json::to_vec(&apps).unwrap_or_default();
    contract::UiFeed {
        revision: contract::revision_of(&bytes),
        apps,
    }
}

/// The registered UI sets, serialized through the wire contract.
fn registered_ui() -> Vec<contract::AppUi> {
    registry::registered_ui()
        .into_iter()
        .map(into_contract)
        .collect()
}

/// The spec-side footprint as the wire shape. One conversion, so the two
/// definitions of "a cell box" cannot drift.
const fn grid_size(size: registry::GridSizeSpec) -> contract::GridSize {
    contract::GridSize {
        span: size.span,
        rows: size.rows,
    }
}

/// The spec-side widget appearance as the wire shape. Field for field: the two
/// are kept compatible by design (the registry mirror exists only because
/// `ferris-core` sits below `ferris-schema`), so the projection is named here
/// once and a field added to either side breaks this function rather than
/// silently dropping out of the wire.
fn style_spec(style: registry::StyleSpec) -> contract::StyleSpec {
    contract::StyleSpec {
        background: style.background,
        background_custom: style.background_custom,
        border_color: style.border_color,
        border_width: style.border_width,
        border_radius: style.border_radius,
        padding: style.padding,
        shadow: style.shadow,
    }
}

/// The spec-side board appearance as the wire shape, mirroring [`style_spec`].
fn dashboard_style(style: registry::DashboardStyle) -> contract::DashboardStyle {
    contract::DashboardStyle {
        background: style.background,
        background_custom: style.background_custom,
        gap_px: style.gap_px,
        padding_px: style.padding_px,
        border_radius: style.border_radius,
        title_full_screen: style.title_full_screen,
    }
}

/// Convert one registry entry into its wire shape.
fn into_contract(ui: registry::AppUi) -> contract::AppUi {
    contract::AppUi {
        owner: ui.owner,
        extensions: ui
            .extensions
            .into_iter()
            .map(|e| contract::UiExtension {
                slot: e.slot,
                doctype: e.doctype,
                component: e.component,
                label: e.label,
                props: e.props,
                requires: e.requires,
            })
            .collect(),
        dashboards: ui
            .dashboards
            .into_iter()
            .map(|d| contract::Dashboard {
                id: d.id,
                label: d.label,
                route: d.route,
                default: d.default,
                style: d.style.map(dashboard_style),
                widgets: d
                    .widgets
                    .into_iter()
                    .map(|w| contract::WidgetInstance {
                        id: w.id,
                        widget: w.widget,
                        label: w.label,
                        // The WIT seed describes content; the title/subtitle
                        // chrome belongs to the Desk's layout, not the app's
                        // `describe()`. Absent means the host defaults.
                        show_title: None,
                        subtitle: None,
                        show_subtitle: None,
                        params: w.params,
                        span: w.span,
                        col: w.col,
                        row: w.row,
                        rows: w.rows,
                        style: w.style.map(style_spec),
                    })
                    .collect(),
            })
            .collect(),
        widgets: ui
            .widgets
            .into_iter()
            .map(|w| contract::WidgetDef {
                name: w.name,
                label: w.label,
                component: w.component,
                kind: w.kind,
                params: w
                    .params
                    .into_iter()
                    .map(|p| contract::WidgetParam {
                        name: p.name,
                        label: p.label,
                        value_type: p.value_type,
                        default: p.default,
                        options: p.options,
                        required: p.required,
                        description: p.description,
                        group: p.group,
                        placeholder: p.placeholder,
                        depends_on: p.depends_on,
                        read_only: p.read_only,
                        hidden: p.hidden,
                    })
                    .collect(),
                description: w.description,
                requires: w.requires,
                events: w.events,
                icon: w.icon,
                category: w.category,
                preview: w.preview,
                code_url: None,
                style_url: None,
                size: w.size.map(grid_size),
                min: w.min.map(grid_size),
                style: w.style.map(style_spec),
            })
            .collect(),
        website: ui.website.map(|w| contract::Website {
            routes: w
                .routes
                .into_iter()
                .map(|r| contract::WebsiteRoute {
                    path: r.path,
                    component: r.component,
                    kind: r.kind,
                })
                .collect(),
        }),
        assets: ui.assets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The core registry is process-global: the tests in this module must not
    /// interleave their registrations (each assertion reads the whole feed).
    static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        REGISTRY_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The full sample app the round trip reads back: one of everything the
    /// registry carries, so a field renamed on either side shows up here.
    ///
    /// Registered from inside the test, not at module load: the registry is
    /// process-global, so this must stay inside `lock()`.
    fn register_round_trip_fixture() {
        registry::register_app_ui(
            "api-ui-test",
            registry::AppUiSpec {
                extensions: vec![registry::UiExtensionSpec {
                    slot: "form.toolbar.right".into(),
                    doctype: Some("Invoice".into()),
                    component: "invoice-actions".into(),
                    label: Some("Acciones de factura".into()),
                    props: vec![("mode".into(), "compact".into())],
                    requires: None,
                }],
                dashboards: vec![registry::DashboardSpec {
                    id: "sales-overview".into(),
                    label: "Resumen de Ventas".into(),
                    route: "/sales-overview".into(),
                    default: false,
                    style: Some(registry::DashboardStyle {
                        background: Some("muted".into()),
                        background_custom: None,
                        gap_px: Some(20),
                        padding_px: None,
                        border_radius: None,
                        title_full_screen: None,
                    }),
                    widgets: vec![registry::WidgetInstanceSpec {
                        id: "monthly-sales".into(),
                        widget: "sales-chart".into(),
                        label: Some("Ventas por mes".into()),
                        params: vec![("doctype".into(), "Invoice".into())],
                        span: Some(6),
                        col: Some(0),
                        row: Some(1),
                        rows: Some(4),
                        style: Some(registry::StyleSpec {
                            background: None,
                            background_custom: Some("oklch(0.4 0.1 20)".into()),
                            border_color: None,
                            border_width: None,
                            border_radius: None,
                            padding: None,
                            shadow: None,
                        }),
                    }],
                }],
                widgets: vec![registry::WidgetDefSpec {
                    name: "sales-chart".into(),
                    label: "Ventas".into(),
                    component: "sales-chart".into(),
                    kind: "chart".into(),
                    params: vec![registry::WidgetParamSpec {
                        name: "doctype".into(),
                        label: Some("DocType".into()),
                        value_type: "doctype".into(),
                        default: None,
                        options: vec![],
                        required: true,
                        description: None,
                        group: Some("Data".into()),
                        placeholder: None,
                        depends_on: None,
                        read_only: false,
                        hidden: false,
                    }],
                    description: None,
                    requires: None,
                    events: vec![],
                    icon: Some("📈".into()),
                    category: Some("Charts".into()),
                    preview: None,
                    size: Some(registry::GridSizeSpec { span: 6, rows: 4 }),
                    min: Some(registry::GridSizeSpec { span: 3, rows: 3 }),
                    style: Some(registry::StyleSpec {
                        background: Some("card".into()),
                        background_custom: None,
                        border_color: None,
                        border_width: None,
                        border_radius: None,
                        padding: Some(12),
                        shadow: None,
                    }),
                    ..registry::WidgetDefSpec::default()
                }],
                website: Some(registry::WebsiteSpec {
                    routes: vec![registry::WebsiteRouteSpec {
                        path: "/blog".into(),
                        component: "blog-index".into(),
                        kind: "page".into(),
                    }],
                }),
                assets: vec!["ui/invoice-actions.js".into(), "ui/ui.css".into()],
            },
        );
    }

    /// Every field the wire contract carries must survive a round trip
    /// through the registry type and back: a field renamed on either side
    /// breaks here rather than as a `null` in the browser.
    #[test]
    fn every_registry_entry_survives_the_contract_round_trip() {
        let _g = lock();
        register_round_trip_fixture();

        let wire = registered_ui();
        let ui = wire
            .iter()
            .find(|ui| ui.owner == "api-ui-test")
            .expect("the registered set is in the feed");
        assert_eq!(ui.owner, "api-ui-test");
        let ext = &ui.extensions[0];
        assert_eq!(ext.slot, "form.toolbar.right");
        assert_eq!(ext.doctype.as_deref(), Some("Invoice"));
        assert_eq!(ext.component, "invoice-actions");
        assert_eq!(ext.label.as_deref(), Some("Acciones de factura"));
        assert_eq!(ext.props, vec![("mode".into(), "compact".into())]);

        let dash = &ui.dashboards[0];
        assert_eq!(dash.id, "sales-overview");
        assert_eq!(dash.route, "/sales-overview");
        assert_eq!(dash.widgets[0].widget, "sales-chart");
        assert_eq!(dash.widgets[0].params[0].1, "Invoice");
        assert_eq!(dash.widgets[0].span, Some(6));
        assert_eq!(dash.widgets[0].col, Some(0));
        assert_eq!(dash.widgets[0].row, Some(1));
        assert_eq!(dash.widgets[0].rows, Some(4));
        // The 10.0.0 style wave survives the registry mirror: the board's style,
        // the instance's override and the definition's default each land on the
        // wire field the Desk reads, which is what keeps the two structural
        // twins (`ferris-core::ui_extensions::specs` and the contract) honest.
        let dash_style = dash.style.as_ref().expect("the board style is on the wire");
        assert_eq!(dash_style.background.as_deref(), Some("muted"));
        assert_eq!(dash_style.gap_px, Some(20));
        let inst_style = dash.widgets[0]
            .style
            .as_ref()
            .expect("the instance style is on the wire");
        assert_eq!(
            inst_style.background_custom.as_deref(),
            Some("oklch(0.4 0.1 20)")
        );

        let def = &ui.widgets[0];
        assert_eq!(def.name, "sales-chart");
        assert_eq!(def.kind, "chart");
        assert_eq!(def.params[0].value_type, "doctype");
        assert!(def.params[0].required);
        assert_eq!(def.params[0].group.as_deref(), Some("Data"));
        assert_eq!(def.icon.as_deref(), Some("📈"));
        assert_eq!(def.category.as_deref(), Some("Charts"));
        assert_eq!(def.size, Some(contract::GridSize { span: 6, rows: 4 }));
        assert_eq!(def.min, Some(contract::GridSize { span: 3, rows: 3 }));
        let def_style = def
            .style
            .as_ref()
            .expect("the definition style is on the wire");
        assert_eq!(def_style.background.as_deref(), Some("card"));
        assert_eq!(def_style.padding, Some(12));

        let site = ui.website.as_ref().expect("website registered");
        assert_eq!(site.routes[0].path, "/blog");
        assert_eq!(site.routes[0].component, "blog-index");
        assert_eq!(site.routes[0].kind, "page");

        // The asset manifest survives too, so the Desk can load the scripts.
        assert_eq!(ui.assets, vec!["ui/invoice-actions.js", "ui/ui.css"]);

        // And the wire shape serializes to the JSON the browser gets.
        let json = serde_json::to_value(ui).unwrap();
        assert_eq!(json["extensions"][0]["component"], "invoice-actions");
        assert_eq!(json["widgets"][0]["type"], "chart");
        assert_eq!(json["widgets"][0]["params"][0]["value-type"], "doctype");
        assert_eq!(json["widgets"][0]["size"]["span"], 6);
        assert_eq!(json["widgets"][0]["size"]["rows"], 4);
        assert_eq!(json["dashboards"][0]["widgets"][0]["span"], 6);
        assert_eq!(json["dashboards"][0]["widgets"][0]["col"], 0);
        assert_eq!(json["dashboards"][0]["widgets"][0]["rows"], 4);
    }

    /// The envelope carries a revision that fingerprints the payload: the
    /// same registry yields the same revision, a changed one does not.
    #[test]
    fn the_feed_revision_fingerprints_the_payload() {
        let _g = lock();
        registry::register_app_ui(
            "feed-test",
            registry::AppUiSpec {
                dashboards: vec![registry::DashboardSpec {
                    id: "d".into(),
                    label: "D".into(),
                    route: "/feed-test".into(),
                    default: false,
                    style: None,
                    widgets: vec![],
                }],
                ..registry::AppUiSpec::default()
            },
        );

        let before = compose(Vec::new(), None);
        assert!(!before.revision.is_empty());
        assert_eq!(
            before.revision,
            compose(Vec::new(), None).revision,
            "stable for a stable registry"
        );

        registry::register_app_ui(
            "feed-test",
            registry::AppUiSpec {
                dashboards: vec![registry::DashboardSpec {
                    id: "d".into(),
                    label: "Changed".into(),
                    route: "/feed-test".into(),
                    default: false,
                    style: None,
                    widgets: vec![],
                }],
                ..registry::AppUiSpec::default()
            },
        );
        let after = compose(Vec::new(), None);
        assert_ne!(
            before.revision, after.revision,
            "a reload changes the revision"
        );
    }

    /// The document sets are appended after the registry's, so a plugin's
    /// dashboard always wins a route collision and the payload (and its
    /// revision) covers the documents too.
    #[test]
    fn the_envelope_appends_document_sets_after_the_registry() {
        let _g = lock();
        let documents = vec![contract::AppUi {
            owner: "db".into(),
            ..contract::AppUi::default()
        }];

        let with_documents = compose(documents, None);
        let registry_only = compose(Vec::new(), None);
        assert_eq!(with_documents.apps.len(), registry_only.apps.len() + 1);
        assert_eq!(
            with_documents.apps.last().map(|app| app.owner.as_str()),
            Some("db")
        );
        assert_ne!(
            with_documents.revision, registry_only.revision,
            "a document dashboard changes the fingerprint, so the Desk refetches"
        );
    }
}
