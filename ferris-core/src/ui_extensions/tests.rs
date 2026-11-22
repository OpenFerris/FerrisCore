use super::host_widgets::param;
use super::registry::reset_registry;
use super::specs::RESERVED_ROUTE_PREFIXES;
use super::validation::dashboard_route_problem;

use super::*;
use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap()
}

fn reset() {
    reset_registry();
}

fn ext(slot: &str, doctype: Option<&str>, component: &str) -> UiExtensionSpec {
    UiExtensionSpec {
        slot: slot.into(),
        doctype: doctype.map(str::to_string),
        component: component.into(),
        label: None,
        props: Vec::new(),
        requires: None,
    }
}

fn register(owner: &str, spec: AppUiSpec) -> Registration {
    register_app_ui(owner, spec)
}

#[test]
fn registering_an_empty_set_registers_nothing() {
    let _g = lock();
    reset();
    register("app-a", AppUiSpec::default());
    assert!(!has_ui_extensions());
    assert!(registered_ui().is_empty());
}

#[test]
fn re_registering_empty_clears_the_previous_set() {
    let _g = lock();
    reset();
    register(
        "app-a",
        AppUiSpec {
            extensions: vec![ext("form.toolbar.right", None, "invoice-actions")],
            ..AppUiSpec::default()
        },
    );
    assert_eq!(registered_ui().len(), 1);
    // The reload declared nothing: the stale set must go with it.
    register("app-a", AppUiSpec::default());
    assert!(registered_ui().is_empty());
}

#[test]
fn registering_replaces_own_previous_set() {
    let _g = lock();
    reset();
    register(
        "app-a",
        AppUiSpec {
            extensions: vec![ext("form.toolbar.right", None, "invoice-actions")],
            ..AppUiSpec::default()
        },
    );
    register(
        "app-b",
        AppUiSpec {
            extensions: vec![ext("shell.topbar.right", None, "health-indicator")],
            ..AppUiSpec::default()
        },
    );
    register(
        "app-a",
        AppUiSpec {
            extensions: vec![ext("form.toolbar.right", None, "audit-actions")],
            ..AppUiSpec::default()
        },
    );

    let sets = registered_ui();
    assert_eq!(sets.len(), 2);
    // Fase 4: the mount order is owner-sorted (stable), not arrival
    // order — so it no longer depends on which plugin loaded first.
    assert_eq!(sets[0].owner, "app-a");
    assert_eq!(sets[0].extensions[0].component, "audit-actions");
    assert_eq!(sets[1].owner, "app-b");
    assert_eq!(sets[1].extensions[0].component, "health-indicator");
}

#[test]
fn registration_order_is_owner_stable() {
    let _g = lock();
    reset();
    register(
        "zeta",
        AppUiSpec {
            extensions: vec![
                ext("shell.sidebar.right", None, "zeta-one"),
                ext("shell.sidebar.right", None, "zeta-two"),
            ],
            ..AppUiSpec::default()
        },
    );
    register(
        "alpha",
        AppUiSpec {
            extensions: vec![ext("shell.sidebar.right", None, "alpha-one")],
            ..AppUiSpec::default()
        },
    );

    // Owner-sorted regardless of registration order, and each app's own
    // declarations keep their order — the guarantee slots rely on.
    let sets = registered_ui();
    let owners: Vec<&str> = sets.iter().map(|ui| ui.owner.as_str()).collect();
    assert_eq!(owners, vec!["alpha", "zeta"]);
    let components: Vec<&str> = sets[1]
        .extensions
        .iter()
        .map(|e| e.component.as_str())
        .collect();
    assert_eq!(components, vec!["zeta-one", "zeta-two"]);
}

#[test]
fn doctype_scoped_extensions_match() {
    let _g = lock();
    reset();
    register(
        "app-a",
        AppUiSpec {
            extensions: vec![
                ext("form.toolbar.right", None, "global-actions"),
                ext("form.toolbar.right", Some("Invoice"), "invoice-actions"),
            ],
            ..AppUiSpec::default()
        },
    );

    assert_eq!(
        extension_for("form.toolbar.right", Some("Invoice"))
            .map(|e| e.component)
            .as_deref(),
        Some("invoice-actions"),
        "doctype-scoped wins over the global fallback"
    );
    assert_eq!(
        extension_for("form.toolbar.right", Some("Task"))
            .map(|e| e.component)
            .as_deref(),
        Some("global-actions"),
        "a doctype without a scoped extension falls back to the global one"
    );
    assert_eq!(
        extension_for("form.toolbar.right", None)
            .map(|e| e.component)
            .as_deref(),
        Some("global-actions")
    );
    assert_eq!(
        extension_for("shell.topbar.right", None),
        None,
        "an undeclared slot has no extension"
    );
}

fn dashboard(id: &str, route: &str, default: bool) -> DashboardSpec {
    DashboardSpec {
        id: id.into(),
        label: id.into(),
        route: route.into(),
        default,
        style: None,
        widgets: Vec::new(),
    }
}

fn instance(id: &str, widget: &str) -> WidgetInstanceSpec {
    WidgetInstanceSpec {
        id: id.into(),
        widget: widget.into(),
        label: None,
        params: Vec::new(),
        span: None,
        col: None,
        row: None,
        rows: None,
        style: None,
    }
}

fn def(name: &str) -> WidgetDefSpec {
    WidgetDefSpec {
        name: name.into(),
        label: name.into(),
        component: name.into(),
        kind: "chart".into(),
        params: Vec::new(),
        description: None,
        requires: None,
        events: Vec::new(),
        icon: None,
        category: None,
        preview: None,
        size: None,
        min: None,
        style: None,
        ..WidgetDefSpec::default()
    }
}

#[test]
fn reserved_and_malformed_routes_are_dropped_with_warnings() {
    let _g = lock();
    reset();
    let result = register(
        "app-a",
        AppUiSpec {
            dashboards: vec![
                dashboard("bad", "no-slash", false),
                dashboard("root", "/", false),
                dashboard("api", "/api/dashboards", false),
                dashboard("app-shadow", "/ops", false),
                dashboard("ok", "/ops", false),
            ],
            ..AppUiSpec::default()
        },
    );
    let kept = registered_dashboards();
    let routes: Vec<&str> = kept.iter().map(|d| d.route.as_str()).collect();
    assert_eq!(routes, vec!["/ops"], "only the clean route survives");
    assert_eq!(result.warnings.len(), 4, "each drop is reported");
}

#[test]
fn duplicate_routes_keep_the_first_registration() {
    let _g = lock();
    reset();
    register(
        "app-a",
        AppUiSpec {
            dashboards: vec![dashboard("one", "/shared", false)],
            ..AppUiSpec::default()
        },
    );
    let result = register(
        "app-b",
        AppUiSpec {
            dashboards: vec![dashboard("two", "/shared", false)],
            ..AppUiSpec::default()
        },
    );
    assert_eq!(registered_dashboards().len(), 1);
    assert!(result.warnings.iter().any(|w| w.contains("already taken")));
}

#[test]
fn only_one_default_dashboard_wins_and_re_registering_keeps_it() {
    let _g = lock();
    reset();
    register(
        "app-a",
        AppUiSpec {
            dashboards: vec![dashboard("home-a", "/home-a", true)],
            ..AppUiSpec::default()
        },
    );
    let second = register(
        "app-b",
        AppUiSpec {
            dashboards: vec![dashboard("home-b", "/home-b", true)],
            ..AppUiSpec::default()
        },
    );
    assert!(
        second.warnings.iter().any(|w| w.contains("default")),
        "the losing default is reported: {:?}",
        second.warnings
    );
    let kept = registered_dashboards();
    let defaults: Vec<&str> = kept
        .iter()
        .filter(|d| d.default)
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(defaults, vec!["home-a"], "first registered wins");

    // Re-registering app-a (its own set is dropped first) must keep its flag.
    register(
        "app-a",
        AppUiSpec {
            dashboards: vec![dashboard("home-a", "/home-a", true)],
            ..AppUiSpec::default()
        },
    );
    assert!(
        registered_dashboards()
            .iter()
            .any(|d| d.default && d.id == "home-a"),
        "the owner re-registering its own default does not collide with itself"
    );
}

#[test]
fn widget_def_names_must_be_kebab_case_and_duplicates_last_win() {
    let _g = lock();
    reset();
    let mut dup = def("task-chart");
    dup.label = "first".into();
    let mut dup2 = def("task-chart");
    dup2.label = "second".into();
    let result = register(
        "app-a",
        AppUiSpec {
            widgets: vec![def("Task_Chart"), dup, dup2],
            ..AppUiSpec::default()
        },
    );
    let widgets = &registered_ui()[0].widgets;
    assert_eq!(widgets.len(), 1, "invalid + duplicate collapsed to one");
    assert_eq!(widgets[0].label, "second", "last declaration wins");
    assert!(result.warnings.iter().any(|w| w.contains("kebab-case")));
    assert!(result.warnings.iter().any(|w| w.contains("duplicate name")));
}

#[test]
fn widget_references_resolve_qualified_bare_and_cross_app() {
    let _g = lock();
    reset();
    let _ = register_host_widgets();
    register(
        "app-a",
        AppUiSpec {
            widgets: vec![def("kpi")],
            dashboards: vec![DashboardSpec {
                id: "d".into(),
                label: "D".into(),
                route: "/d".into(),
                default: false,
                style: None,
                widgets: vec![
                    instance("own", "kpi"),
                    instance("host", "host.stat"),
                    instance("broken", "nope.missing"),
                ],
            }],
            ..AppUiSpec::default()
        },
    );
    // Bare reference resolves against the requesting owner.
    let (owner, d) = find_widget("kpi", "app-a").expect("own widget");
    assert_eq!(owner, "app-a");
    assert_eq!(d.name, "kpi");
    // Qualified reference selects the host's catalog.
    let (owner, d) = find_widget("host.stat", "app-a").expect("host widget");
    assert_eq!(owner, "host");
    assert_eq!(d.kind, "stat");
    assert!(d.params.iter().any(|p| p.name == "doctype" && p.required));
    // Wrong default owner: a bare name does not leak across namespaces.
    assert!(find_widget("kpi", "app-b").is_none());
    assert!(find_widget("nope.missing", "app-a").is_none());
}

#[test]
fn unresolved_widget_references_are_reported_but_kept() {
    let _g = lock();
    reset();
    let result = register(
        "app-a",
        AppUiSpec {
            dashboards: vec![DashboardSpec {
                id: "d".into(),
                label: "D".into(),
                route: "/d".into(),
                default: false,
                style: None,
                widgets: vec![instance("w", "ghost.kpi")],
            }],
            ..AppUiSpec::default()
        },
    );
    assert_eq!(registered_dashboards().len(), 1, "the dashboard stays");
    assert!(
        result.warnings.iter().any(|w| w.contains("ghost.kpi")),
        "the unresolved reference is reported: {:?}",
        result.warnings
    );
}

#[test]
fn host_widget_catalog_is_parametrized() {
    let _g = lock();
    reset();
    let _ = register_host_widgets();
    let defs = registered_widgets();
    let names: Vec<&str> = defs
        .iter()
        .filter(|(owner, _)| owner == "host")
        .map(|(_, d)| d.name.as_str())
        .collect();
    assert_eq!(names, vec!["stat", "trend", "table"]);
    // Every host widget requires the DocType parameter — that is what
    // makes one definition serve every doctype the old hardcoded
    // dashboard served.
    for (_, d) in defs.iter().filter(|(owner, _)| owner == "host") {
        assert!(
            d.params
                .iter()
                .any(|p| p.name == "doctype" && p.required && p.value_type == "doctype"),
            "{} must require a doctype param",
            d.name
        );
    }
}

#[test]
fn describe_json_roundtrips() {
    let _g = lock();
    reset();
    let ui = AppUi {
        owner: "app-a".into(),
        extensions: vec![UiExtensionSpec {
            slot: "list.before_table".into(),
            doctype: None,
            component: "quick-filter".into(),
            label: None,
            props: vec![("placeholder".into(), "Buscar…".into())],
            requires: None,
        }],
        dashboards: vec![DashboardSpec {
            id: "sales-overview".into(),
            label: "Resumen de Ventas".into(),
            route: "/sales-overview".into(),
            default: false,
            style: None,
            widgets: vec![WidgetInstanceSpec {
                id: "monthly-sales".into(),
                widget: "sales-chart".into(),
                label: Some("Ventas por mes".into()),
                params: vec![("doctype".into(), "Invoice".into())],
                span: Some(6),
                col: Some(3),
                row: Some(0),
                rows: Some(4),
                style: None,
            }],
        }],
        widgets: vec![WidgetDefSpec {
            name: "sales-chart".into(),
            label: "Ventas".into(),
            component: "sales-chart".into(),
            kind: "chart".into(),
            params: vec![param(
                "doctype",
                "DocType",
                "Data",
                "doctype",
                None,
                &[],
                true,
            )],
            description: None,
            requires: None,
            events: Vec::new(),
            icon: Some("📈".into()),
            category: Some("Charts".into()),
            preview: None,
            size: Some(GridSizeSpec { span: 6, rows: 4 }),
            min: Some(GridSizeSpec { span: 3, rows: 3 }),
            style: None,
            ..WidgetDefSpec::default()
        }],
        website: Some(WebsiteSpec {
            routes: vec![WebsiteRouteSpec {
                path: "/blog".into(),
                component: "blog-index".into(),
                kind: "page".into(),
            }],
        }),
        assets: vec!["ui/quick-filter.js".into(), "ui/ui.css".into()],
    };
    let json = serde_json::to_string(&ui).unwrap();
    let back: AppUi = serde_json::from_str(&json).unwrap();
    assert_eq!(ui, back);

    // Wire names the browser relies on.
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["widgets"][0]["params"][0]["value-type"], "doctype");
    assert_eq!(value["dashboards"][0]["widgets"][0]["span"], 6);
    assert_eq!(value["dashboards"][0]["route"], "/sales-overview");
}

/// Old JSON (no `widgets`, no `props`, widget-instance keys) must still
/// parse: the feed only grows.
#[test]
fn pre_widget_json_still_parses() {
    let ui: AppUi = serde_json::from_str(
        r#"{
            "owner": "app-a",
            "extensions": [{"slot": "form.toolbar.right", "component": "x"}],
            "dashboards": [{
                "id": "d", "label": "D", "route": "/d",
                "widgets": [{"id": "w", "label": "L", "component": "c", "type": "chart"}]
            }]
        }"#,
    )
    .expect("old-shape JSON parses");
    assert!(ui.widgets.is_empty());
    assert!(ui.extensions[0].props.is_empty());
    // Instance fields absent from the old JSON default sensibly.
    let w = &ui.dashboards[0].widgets[0];
    assert_eq!(w.widget, "");
    assert!(w.params.is_empty());
    assert_eq!(w.span, None);
    // `value-type` defaults to `string` when the old JSON omits it.
    let d: WidgetDefSpec =
        serde_json::from_str(r#"{"name": "x", "component": "x", "type": "chart"}"#)
            .expect("old-shape def parses");
    assert_eq!(d.params, vec![]);
    let p: WidgetParamSpec =
        serde_json::from_str(r#"{"name": "size"}"#).expect("param without value-type parses");
    assert_eq!(p.value_type, "string");
}

#[test]
fn route_problem_covers_the_reserved_prefixes() {
    for reserved in RESERVED_ROUTE_PREFIXES.iter().copied() {
        assert!(
            dashboard_route_problem(reserved).is_some(),
            "{reserved} itself must be rejected"
        );
    }
    // Prefix match is segment-wise: `/dashboards` is NOT `/dashboard`.
    assert_eq!(dashboard_route_problem("/dashboards"), None);
    assert!(dashboard_route_problem("/app/ops").is_some());
    assert!(dashboard_route_problem("/socket.io/x").is_some());
}

/// The host's own catalog declares a style default for every widget it
/// publishes — that is the "assign a default" half of the request, and it is
/// what a user overriding one field starts from.
#[test]
fn host_widgets_declare_a_default_style() {
    let _g = lock();
    reset();
    let _ = register_host_widgets();
    for (owner, def) in registered_widgets() {
        if owner != "host" {
            continue;
        }
        let style = def
            .style
            .as_ref()
            .unwrap_or_else(|| panic!("host widget {} must declare a default style", def.name));
        assert_eq!(style.background.as_deref(), Some("card"), "{}", def.name);
        assert_eq!(style.border_width, Some(1), "{}", def.name);
        assert_eq!(style.border_radius, Some(8), "{}", def.name);
        assert!(style.padding.is_some(), "{}", def.name);
        // A token, never a literal colour (AGENTS.md §5).
        assert!(!style.background.as_deref().unwrap().contains('#'));
    }
}

/// The spec-side precedence rule: an instance's own style wins field by field,
/// an unset field falls through to the definition, and neither declaring
/// anything resolves to `None` (which the Desk reads as "the global default").
#[test]
fn widget_style_layers_instance_over_definition() {
    let base = StyleSpec {
        background: Some("card".into()),
        border_width: Some(1),
        padding: Some(8),
        ..StyleSpec::default()
    };
    let definition = WidgetDefSpec {
        style: Some(base.clone()),
        ..def("chart")
    };

    // Nothing declared anywhere → nothing to emit.
    assert!(widget_style(None, &def("plain")).is_none());

    // The definition alone is used as-is.
    let only_def = widget_style(None, &definition).expect("definition style");
    assert_eq!(only_def.background.as_deref(), Some("card"));
    assert_eq!(only_def.padding, Some(8));

    // The instance wins per field and inherits the rest.
    let instance = StyleSpec {
        background: Some("accent".into()),
        ..StyleSpec::default()
    };
    let merged = widget_style(Some(&instance), &definition).expect("merged");
    assert_eq!(
        merged.background.as_deref(),
        Some("accent"),
        "instance wins"
    );
    assert_eq!(merged.border_width, Some(1), "unset falls to definition");
    assert_eq!(merged.padding, Some(8), "unset falls to definition");
}

/// An instance whose reference resolves to a definition picks up that
/// definition's default through the lookup twin, and an unresolved reference
/// yields nothing rather than a half-resolved style.
#[test]
fn widget_style_by_reference_resolves_through_the_catalog() {
    let _g = lock();
    reset();
    let _ = register_host_widgets();
    // The host's `stat` declares a card default; an instance overriding only
    // the padding keeps the rest.
    let instance = StyleSpec {
        padding: Some(24),
        ..StyleSpec::default()
    };
    let resolved = widget_style_by_reference("host.stat", "app-a", Some(&instance))
        .expect("host.stat resolves");
    assert_eq!(resolved.padding, Some(24), "instance wins");
    assert_eq!(
        resolved.background.as_deref(),
        Some("card"),
        "from definition"
    );

    // A bare reference resolves against the requesting owner.
    let _ = register_host_widgets();
    assert!(widget_style_by_reference("ghost.kpi", "app-a", None).is_none());
}

/// A style-less spec round-trips byte-identically: the feed a Desk with no
/// style configuration produces is the feed it produced before this feature.
#[test]
fn style_less_specs_serialize_without_the_field() {
    let ui = AppUi {
        owner: "app-a".into(),
        dashboards: vec![dashboard("d", "/d", false)],
        widgets: vec![def("kpi")],
        ..AppUi::default()
    };
    let wire: serde_json::Value = serde_json::to_value(&ui).expect("serializes");
    assert!(wire["dashboards"][0].get("style").is_none());
    assert!(wire["widgets"][0].get("style").is_none());
    // And the old shape parses: a payload without `style` is still legal.
    let back: AppUi = serde_json::from_str(
        r#"{"owner":"app-a","dashboards":[{"id":"d","label":"D","route":"/d"}],
            "widgets":[{"name":"k","component":"k","type":"chart"}]}"#,
    )
    .expect("style-less feed parses");
    assert!(back.widgets[0].style.is_none());
    assert!(back.dashboards[0].style.is_none());
}
