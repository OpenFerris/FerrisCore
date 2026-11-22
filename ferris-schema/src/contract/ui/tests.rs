use super::*;
use crate::contract::style::{DashboardConfig, DashboardStyle};

/// A slot extension needs `slot` and `component`; everything else is
/// optional. `doctype`/`label` absent must deserialize to `None`, not to a
/// hard failure or an empty string the Desk would render as a name.
#[test]
fn a_doctype_scoped_extension_roundtrips() {
    let ui: AppUi = serde_json::from_value(serde_json::json!({
        "owner": "app-a",
        "extensions": [{
            "slot": "form.toolbar.right",
            "doctype": "Invoice",
            "component": "invoice-actions",
            "label": "Acciones de factura",
            "props": [["mode", "compact"]],
        }],
        "assets": ["ui/invoice-actions.js", "ui/ui.css"],
    }))
    .expect("extension parses");

    assert_eq!(ui.owner, "app-a");
    let ext = &ui.extensions[0];
    assert_eq!(ext.slot, "form.toolbar.right");
    assert_eq!(ext.doctype.as_deref(), Some("Invoice"));
    assert_eq!(ext.component, "invoice-actions");
    assert_eq!(ext.label.as_deref(), Some("Acciones de factura"));
    assert_eq!(ext.props, vec![("mode".into(), "compact".into())]);
    assert!(ui.dashboards.is_empty());
    assert!(ui.website.is_none());
    // The asset manifest the Desk loads to define the custom elements.
    assert_eq!(ui.assets, vec!["ui/invoice-actions.js", "ui/ui.css"]);
}

/// A slot-wide extension omits `doctype`/`label`/`props` entirely: they
/// must come back as `None`/empty and the `AppUi` still parse (missing
/// keys are legal for every field with a default).
#[test]
fn a_global_extension_parses_without_scoping_keys() {
    let ui: AppUi = serde_json::from_value(serde_json::json!({
        "owner": "app-a",
        "extensions": [{
            "slot": "shell.topbar.right",
            "component": "health-indicator",
        }],
    }))
    .expect("extension parses");

    let ext = &ui.extensions[0];
    assert_eq!(ext.doctype, None);
    assert_eq!(ext.label, None);
    assert!(ext.props.is_empty());
}

/// An entirely absent `AppUi` (no extensions, no dashboards, no widgets)
/// is a legal response for a Desk with no plugins declaring UI — every
/// list field must default rather than fail the feed.
#[test]
fn an_empty_appui_is_legal() {
    let ui: AppUi = serde_json::from_value(serde_json::json!({ "owner": "app-a" }))
        .expect("empty AppUi parses");
    assert!(ui.extensions.is_empty());
    assert!(ui.dashboards.is_empty());
    assert!(ui.widgets.is_empty());
    assert!(ui.website.is_none());
}

/// The widget catalog + instances survive the feed round trip with the
/// field names the Desk reads (`type`, `value-type`, `span`, `params`).
#[test]
fn widgets_and_instances_roundtrip_with_wire_names() {
    let ui: AppUi = serde_json::from_value(serde_json::json!({
        "owner": "test",
        "widgets": [{
            "name": "task-chart",
            "label": "Tareas",
            "component": "task-chart",
            "type": "chart",
            "params": [{
                "name": "doctype",
                "label": "DocType",
                "value-type": "doctype",
                "required": true,
                "options": [],
            }],
        }],
        "dashboards": [{
            "id": "home",
            "label": "Home",
            "route": "/ops",
            "default": true,
            "widgets": [{
                "id": "c1",
                "widget": "host.stat",
                "label": "Total",
                "params": [["doctype", "Task"]],
                "span": 3,
            }],
        }],
    }))
    .expect("catalog parses");

    let def = &ui.widgets[0];
    assert_eq!(def.kind, "chart");
    assert_eq!(def.params[0].value_type, "doctype");
    assert!(def.params[0].required);

    let dash = &ui.dashboards[0];
    assert_eq!(dash.route, "/ops");
    assert!(dash.default);
    let inst = &dash.widgets[0];
    assert_eq!(inst.widget, "host.stat");
    assert_eq!(inst.params, vec![("doctype".into(), "Task".into())]);
    assert_eq!(inst.span, Some(3));

    // And the JSON the Desk would receive carries the same names back.
    let wire = serde_json::to_value(&ui).unwrap();
    assert_eq!(wire["widgets"][0]["type"], "chart");
    assert_eq!(wire["widgets"][0]["params"][0]["value-type"], "doctype");
    assert_eq!(wire["dashboards"][0]["widgets"][0]["span"], 3);
}

/// A dashboard without a `route` is the retired widget-only shape and
/// must not parse: publishing a widget definition replaced it.
#[test]
fn a_routeless_dashboard_is_rejected() {
    let parsed = serde_json::from_value::<AppUi>(serde_json::json!({
        "owner": "test",
        "dashboards": [{ "id": "d", "label": "D", "widgets": [] }],
    }));
    assert!(parsed.is_err(), "route is required by the contract");
}

/// The envelope's revision is a stable fingerprint of the payload:
/// equal payloads → equal revision, different payloads → different one.
#[test]
fn revision_fingerprints_the_payload() {
    let feed = UiFeed {
        revision: revision_of(b"same"),
        apps: Vec::new(),
    };
    assert_eq!(revision_of(b"same"), feed.revision, "stable");
    assert_ne!(revision_of(b"other"), feed.revision, "distinct");
    // And the envelope itself round trips.
    let json = serde_json::to_value(&feed).unwrap();
    let back: UiFeed = serde_json::from_value(json).unwrap();
    assert_eq!(back, feed);
}

/// `widget_instances` reads the two shapes a bound `widgets` field can
/// hold — a `Code` field's JSON-encoded string and an already-decoded
/// array — and treats an absent, malformed or non-list value as an empty
/// dashboard rather than an error, dropping only the broken elements.
#[test]
fn a_widgets_slot_parses_from_either_shape_and_never_fails() {
    let instance = r#"{"id":"w1","widget":"host.stat"}"#;
    let array = serde_json::json!([{"id": "w1", "widget": "host.stat"}]);
    let list = |slot: Option<&serde_json::Value>| widget_instances(slot);

    // A `Code` field: the JSON-encoded string.
    let encoded = serde_json::json!(format!("[{instance}]"));
    assert_eq!(list(Some(&encoded)).len(), 1, "string shape");
    // A JSON field: the array itself.
    assert_eq!(list(Some(&array)).len(), 1, "array shape");
    // Absent field, malformed string, and non-list values are empty.
    assert!(list(None).is_empty(), "absent slot");
    let broken = serde_json::json!("not json");
    assert!(list(Some(&broken)).is_empty(), "malformed string");
    let scalar = serde_json::json!(7);
    assert!(list(Some(&scalar)).is_empty(), "non-list value");
    // One broken element does not blank the good ones.
    let mixed = serde_json::json!([{"id": "w1", "widget": "host.stat"}, {"id": 42}]);
    let parsed = list(Some(&mixed));
    assert_eq!(parsed.len(), 1, "the broken element is dropped");
    assert_eq!(parsed[0].id, "w1");
}

/// A widget definition/instance with no `style` key is the case every app
/// that predates this feature produces: it must parse, and the field must
/// come back absent rather than fail the feed.
#[test]
fn style_is_optional_and_absent_by_default() {
    let ui: AppUi = serde_json::from_value(serde_json::json!({
        "owner": "test",
        "widgets": [{
            "name": "task-chart",
            "label": "Tareas",
            "component": "task-chart",
            "type": "chart",
        }],
        "dashboards": [{
            "id": "home",
            "label": "Home",
            "route": "/ops",
            "widgets": [{ "id": "c1", "widget": "host.stat" }],
        }],
    }))
    .expect("a style-less catalog parses");

    assert!(ui.widgets[0].style.is_none());
    assert!(ui.dashboards[0].widgets[0].style.is_none());
    assert!(ui.dashboards[0].style.is_none());
    // And nothing is written back for them: the feed stays byte-identical
    // to one produced before the field existed.
    let wire = serde_json::to_value(&ui).expect("serializes");
    assert!(wire["widgets"][0].get("style").is_none());
    assert!(wire["dashboards"][0]["widgets"][0].get("style").is_none());
    assert!(wire["dashboards"][0].get("style").is_none());
}

/// The style a definition declares and the one an instance overrides both
/// round-trip through the wire with the documented field names.
#[test]
fn widget_styles_roundtrip_on_definition_and_instance() {
    let ui: AppUi = serde_json::from_value(serde_json::json!({
        "owner": "test",
        "widgets": [{
            "name": "task-chart",
            "label": "Tareas",
            "component": "task-chart",
            "type": "chart",
            "style": { "background": "muted", "border_width": 1, "padding": 12 },
        }],
        "dashboards": [{
            "id": "home",
            "label": "Home",
            "route": "/ops",
            "style": { "gap_px": 20, "title_full_screen": true },
            "widgets": [{
                "id": "c1",
                "widget": "host.stat",
                "style": { "background_custom": "oklch(0.4 0.1 20)" },
            }],
        }],
    }))
    .expect("styled catalog parses");

    let def_style = ui.widgets[0].style.as_ref().expect("definition style");
    assert_eq!(def_style.background.as_deref(), Some("muted"));
    assert_eq!(def_style.border_width, Some(1));
    assert_eq!(def_style.padding, Some(12));

    let dash_style = ui.dashboards[0].style.as_ref().expect("dashboard style");
    assert_eq!(dash_style.gap_px, Some(20));
    assert_eq!(dash_style.title_full_screen, Some(true));

    let inst_style = ui.dashboards[0].widgets[0]
        .style
        .as_ref()
        .expect("instance style");
    assert_eq!(
        inst_style.background_custom.as_deref(),
        Some("oklch(0.4 0.1 20)")
    );

    // The wire names the Desk reads back.
    let wire = serde_json::to_value(&ui).expect("serializes");
    assert_eq!(wire["widgets"][0]["style"]["background"], "muted");
    assert_eq!(wire["dashboards"][0]["style"]["gap_px"], 20);
    assert_eq!(
        wire["dashboards"][0]["widgets"][0]["style"]["background_custom"],
        "oklch(0.4 0.1 20)"
    );
}

/// `DashboardLayout` carries the shared config, and a config that declares
/// nothing is skipped so an unconfigured dashboard's answer is unchanged.
#[test]
fn dashboard_layout_carries_an_optional_config() {
    let bare: DashboardLayout = serde_json::from_value(serde_json::json!({
        "editable": true,
        "customized": false,
        "widgets": [],
    }))
    .expect("a config-less layout parses");
    assert!(bare.config.is_empty());
    let wire = serde_json::to_value(&bare).expect("serializes");
    assert!(wire.get("config").is_none(), "empty config is skipped");

    let configured: DashboardLayout = serde_json::from_value(serde_json::json!({
        "editable": true,
        "customized": true,
        "widgets": [],
        "config": { "dashboard_id": "home", "style": { "gap_px": 24 } },
    }))
    .expect("a configured layout parses");
    assert_eq!(configured.config.dashboard_id.as_deref(), Some("home"));
    assert_eq!(configured.config.style.gap_px, Some(24));
}

/// The `PUT` body pairs the widget list and the config in one payload, and
/// an absent config is legal (a layout save with no style change).
#[test]
fn the_put_payload_pairs_widgets_and_config() {
    let payload: DashboardLayoutPayload = serde_json::from_value(serde_json::json!({
        "widgets": [{ "id": "c1", "widget": "host.stat", "span": 3 }],
    }))
    .expect("a widgets-only payload parses");
    assert_eq!(payload.widgets.len(), 1);
    assert!(payload.config.is_empty());

    let full = DashboardLayoutPayload {
        widgets: payload.widgets,
        config: DashboardConfig {
            dashboard_id: Some("home".to_string()),
            style: DashboardStyle {
                gap_px: Some(16),
                ..Default::default()
            },
        },
    };
    let wire = serde_json::to_value(&full).expect("serializes");
    assert_eq!(wire["widgets"][0]["span"], 3);
    assert_eq!(wire["config"]["style"]["gap_px"], 16);
    let back: DashboardLayoutPayload = serde_json::from_value(wire).expect("parses");
    assert_eq!(back, full);
}
