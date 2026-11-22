//! Behavioural checks on the placement rule and the shared document key.
//!
//! They exercise the pure functions directly: the HTTP handlers need a router
//! and a signed-in caller, so their contract (the effective answer, the edit
//! gate) is covered by the router tests instead.

use super::layout::doc_name;
use super::seed::registry_seed;
use super::{normalize_widgets, sanitize_widgets};
use ferris_schema::contract;
use ferris_schema::contract::{DashboardConfig, WidgetInstance};

#[test]
fn sanitize_keeps_only_resolvable_unique_widgets_and_clamps_spans() {
    let widgets = vec![
        WidgetInstance {
            id: "a".to_string(),
            widget: "definitely-not-registered.kpi".to_string(),
            ..Default::default()
        },
        WidgetInstance {
            id: "b".to_string(),
            widget: "host.stat".to_string(),
            span: Some(99),
            ..Default::default()
        },
        WidgetInstance {
            id: "c".to_string(),
            widget: "host.stat".to_string(),
            span: Some(0),
            ..Default::default()
        },
        WidgetInstance {
            id: String::new(),
            widget: "host.trend".to_string(),
            ..Default::default()
        },
    ];

    // Register the host catalog so `host.stat`/`host.trend` resolve.
    let _ = ferris_core::ui_extensions::register_host_widgets();
    let sanitized = sanitize_widgets("host", &widgets);

    assert_eq!(sanitized.len(), 2, "invalid refs and blank ids are dropped");
    assert_eq!(sanitized[0].id, "b");
    assert_eq!(sanitized[0].span, Some(12), "99 clamps to 12");
    assert_eq!(sanitized[1].id, "c");
    assert_eq!(
        sanitized[1].span,
        Some(2),
        "0 is raised to host.stat's declared minimum"
    );
    // Every kept widget leaves with a resolved cell, so the Desk renders
    // exactly what was stored.
    assert_eq!((sanitized[0].col, sanitized[0].row), (Some(0), Some(0)));
    assert!(sanitized.iter().all(|w| w.col.is_some() && w.row.is_some()));
}

/// The subtitle is free text from the editor: a real value is trimmed and a
/// whitespace-only one becomes "no subtitle", so the host falls back to the
/// definition's description. The chrome flags travel untouched.
#[test]
fn sanitize_trims_the_subtitle_and_keeps_the_chrome_flags() {
    let _ = ferris_core::ui_extensions::register_host_widgets();
    let widgets = vec![
        WidgetInstance {
            id: "a".to_string(),
            widget: "host.stat".to_string(),
            subtitle: Some("  En vivo  ".to_string()),
            show_title: Some(false),
            show_subtitle: Some(true),
            ..Default::default()
        },
        WidgetInstance {
            id: "b".to_string(),
            widget: "host.stat".to_string(),
            subtitle: Some("   ".to_string()),
            ..Default::default()
        },
    ];

    let sanitized = sanitize_widgets("host", &widgets);

    assert_eq!(sanitized.len(), 2);
    assert_eq!(sanitized[0].subtitle.as_deref(), Some("En vivo"));
    assert_eq!(sanitized[0].show_title, Some(false));
    assert_eq!(sanitized[0].show_subtitle, Some(true));
    assert!(sanitized[1].subtitle.is_none());
}

#[test]
fn registry_seed_mirrors_the_registry_contract() {
    // The registry conversion is a plain field shim; prove it round-trips
    // the instance shape a seed carries.
    ferris_core::ui_extensions::register_app_ui(
        "example",
        ferris_core::ui_extensions::AppUiSpec {
            dashboards: vec![ferris_core::ui_extensions::DashboardSpec {
                id: "board".to_string(),
                route: "/example/board".to_string(),
                widgets: vec![ferris_core::ui_extensions::WidgetInstanceSpec {
                    id: "w1".to_string(),
                    widget: "example.kpi".to_string(),
                    label: None,
                    params: vec![("doctype".to_string(), "ToDo".to_string())],
                    span: Some(4),
                    col: Some(8),
                    row: Some(2),
                    rows: Some(3),
                    style: None,
                }],
                ..Default::default()
            }],
            widgets: vec![ferris_core::ui_extensions::WidgetDefSpec {
                name: "kpi".to_string(),
                component: "example-kpi".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    let mut seed = registry_seed("example", "board").expect("the app declares it");
    normalize_widgets("example", &mut seed);
    assert_eq!(seed.len(), 1);
    assert_eq!(seed[0].id, "w1");
    assert_eq!(
        seed[0].params[0],
        ("doctype".to_string(), "ToDo".to_string())
    );
    assert_eq!(seed[0].span, Some(4));
    // The declared cell survives normalization, and the height the
    // instance did not declare is filled from the definition's kind.
    assert_eq!((seed[0].col, seed[0].row), (Some(8), Some(2)));
    assert_eq!(seed[0].rows, Some(3));

    // Unknown owner/dashboard has no seed at all — distinct from "empty",
    // which is what lets a zero-widget plugin dashboard keep its own name.
    assert!(registry_seed("other", "board").is_none());
    assert!(registry_seed("example", "nope").is_none());
}

#[test]
fn doc_name_is_the_shared_primary_key() {
    // The row is shared by every user on purpose (plan C4): the name keys
    // the `(owner, dashboard)` pair, so two users editing the same
    // dashboard address the *same* row rather than two colliding ones.
    assert_eq!(doc_name("solmecbox", "planta"), "solmecbox:planta");
    // Distinct dashboards keep distinct names; the app owner is part of the
    // key, so two apps may each declare a dashboard called `planta`.
    assert_ne!(
        doc_name("solmecbox", "planta"),
        doc_name("solmecbox", "almacen")
    );
    assert_ne!(
        doc_name("solmecbox", "planta"),
        doc_name("otra-app", "planta")
    );
    let _ = contract::DashboardLayout::default();
}

/// A saved config is clamped on the way in, exactly like the geometry, and
/// an absent field survives as absent ("inherit") rather than becoming a
/// zero the Desk would draw.
///
/// There are two guards, deliberately: the fields are `u8`, so a value the
/// type cannot hold is rejected when the body parses (a loud 422), and
/// [`DashboardConfig::clamped`] caps the values the type *can* hold at the
/// semantic ceiling the grid can actually draw.
#[test]
fn a_saved_config_clamps_numbers_and_keeps_absent_fields_absent() {
    let raw: DashboardConfig = serde_json::from_value(serde_json::json!({
        "style": {
            "gap_px": 200,
            "padding_px": 200,
            "border_radius": 200,
            "background": "muted",
        },
    }))
    .expect("the config parses");
    let clamped = raw.clamped();
    assert_eq!(clamped.style.gap_px, Some(contract::MAX_GAP));
    assert_eq!(clamped.style.padding_px, Some(contract::MAX_PADDING));
    assert_eq!(clamped.style.border_radius, Some(contract::MAX_RADIUS));
    // The fields the override did not declare stay absent — "inherit",
    // not "zero".
    assert_eq!(clamped.style.title_full_screen, None);
    assert_eq!(clamped.style.background.as_deref(), Some("muted"));
}

/// The seed carries the style the instance declares, so the layout answer
/// is self-contained: an instance with a declared style keeps it, and one
/// without stays without (the Desk layers the definition's default on top).
#[test]
fn the_seed_carries_declared_instance_styles() {
    ferris_core::ui_extensions::register_app_ui(
        "stylish",
        ferris_core::ui_extensions::AppUiSpec {
            dashboards: vec![ferris_core::ui_extensions::DashboardSpec {
                id: "board".to_string(),
                route: "/stylish/board".to_string(),
                widgets: vec![
                    ferris_core::ui_extensions::WidgetInstanceSpec {
                        id: "styled".to_string(),
                        widget: "stylish.kpi".to_string(),
                        style: Some(ferris_core::ui_extensions::StyleSpec {
                            background: Some("accent".to_string()),
                            padding: Some(20),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ferris_core::ui_extensions::WidgetInstanceSpec {
                        id: "plain".to_string(),
                        widget: "stylish.kpi".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            widgets: vec![ferris_core::ui_extensions::WidgetDefSpec {
                name: "kpi".to_string(),
                component: "stylish-kpi".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    let seed = registry_seed("stylish", "board").expect("the app declares it");
    let styled = seed
        .iter()
        .find(|w| w.id == "styled")
        .expect("the styled instance");
    let style = styled.style.as_ref().expect("its style is copied");
    assert_eq!(style.background.as_deref(), Some("accent"));
    assert_eq!(style.padding, Some(20));
    assert!(
        style.background_custom.is_none(),
        "absent fields stay absent"
    );

    let plain = seed
        .iter()
        .find(|w| w.id == "plain")
        .expect("the plain instance");
    assert!(
        plain.style.is_none(),
        "an instance with no style keeps none"
    );
}
