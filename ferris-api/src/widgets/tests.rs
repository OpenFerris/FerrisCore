use ferris_core::Value;
use ferris_core::ui_extensions::{GridSizeSpec, StyleSpec, WidgetDefSpec, WidgetParamSpec};

use super::model::{stored_from_value, to_contract, to_value};
use super::seed_hash;

fn def() -> WidgetDefSpec {
    WidgetDefSpec {
        name: "sales-chart".into(),
        label: "Ventas".into(),
        component: "sales-chart".into(),
        kind: "chart".into(),
        params: vec![WidgetParamSpec {
            name: "doctype".into(),
            label: Some("DocType".into()),
            value_type: "doctype".into(),
            default: None,
            options: vec![],
            required: true,
            description: None,
            group: Some("Datos".into()),
            placeholder: None,
            depends_on: None,
            read_only: false,
            hidden: false,
        }],
        description: Some("Ventas por mes".into()),
        requires: None,
        events: vec!["refresh".into()],
        icon: Some("📈".into()),
        category: Some("Charts".into()),
        preview: None,
        size: Some(GridSizeSpec { span: 6, rows: 4 }),
        min: Some(GridSizeSpec { span: 3, rows: 3 }),
        style: Some(StyleSpec {
            background: Some("card".into()),
            padding: Some(12),
            ..StyleSpec::default()
        }),
        renderer: None,
        version: Some("1".into()),
        code_path: Some("ui/widgets/charts/sales.js".into()),
        script: Some("export const x = 1;".into()),
        stylesheet: Some(".x { color: var(--foreground); }".into()),
        ssr: None,
    }
}

#[test]
fn a_document_round_trips_every_field() {
    let def = def();
    let value = to_value("solmecbox", &def, "hash", true);
    let stored = stored_from_value(&value).expect("the row is complete");
    assert_eq!(stored.app, "solmecbox");
    assert!(stored.enabled);
    assert_eq!(stored.seed_hash, "hash");
    assert_eq!(stored.def, def);
}

#[test]
fn an_update_preserves_the_admin_enabled_flag() {
    let value = to_value("app", &def(), "hash", false);
    assert!(
        value.get("enabled").is_none(),
        "an update must not reset a row the admin disabled"
    );
    let create = to_value("app", &def(), "hash", true);
    assert_eq!(create.get("enabled").and_then(Value::as_bool), Some(true));
}

#[test]
fn a_changed_definition_changes_the_seed_hash() {
    let first = def();
    let mut second = first.clone();
    second.script = Some("export const x = 2;".into());
    assert_ne!(seed_hash(&first), seed_hash(&second));
    assert_eq!(seed_hash(&first), seed_hash(&def()));
}

#[test]
fn the_contract_urls_follow_the_stored_path() {
    let stored = stored_from_value(&to_value("app", &def(), "hash", true)).unwrap();
    let wire = to_contract(&stored);
    let code_url = wire.code_url.as_deref().expect("a script widget has code");
    assert!(
        code_url.starts_with("/plugins/app/asset/ui/widgets/charts/sales.js?v="),
        "the stored module is served at its original path, versioned by content: {code_url}"
    );
    let style_url = wire.style_url.as_deref().expect("a styled widget has CSS");
    assert!(
        style_url.starts_with("/api/widgets/app/sales-chart/style.css?v="),
        "the stored stylesheet is versioned by content: {style_url}"
    );

    // A source-only edit changes the URL (and therefore the feed revision),
    // even when every metadata field stays identical.
    let mut edited = def();
    edited.script = Some("export const x = 2;".into());
    let edited = stored_from_value(&to_value("app", &edited, "hash", true)).unwrap();
    assert_ne!(to_contract(&edited).code_url, wire.code_url);

    let mut bare = def();
    bare.code_path = None;
    bare.stylesheet = None;
    let stored = stored_from_value(&to_value("app", &bare, "hash", true)).unwrap();
    let wire = to_contract(&stored);
    let code_url = wire.code_url.as_deref().expect("the fallback script URL");
    assert!(
        code_url.starts_with("/api/widgets/app/sales-chart/script.js?v="),
        "the self-contained fallback is versioned too: {code_url}"
    );
    assert!(wire.style_url.is_none());
}

#[test]
fn a_native_widget_has_no_code_url() {
    let mut native = def();
    native.renderer = Some("stat".into());
    native.script = None;
    native.code_path = None;
    let stored = stored_from_value(&to_value("host", &native, "hash", true)).unwrap();
    assert!(to_contract(&stored).code_url.is_none());
}

#[test]
fn a_disabled_row_is_not_published() {
    let mut value = to_value("app", &def(), "hash", true);
    value.set("enabled".to_string(), Value::bool(false));
    let stored = stored_from_value(&value).unwrap();
    assert!(!stored.enabled);
}

#[test]
fn an_empty_footprint_stays_absent() {
    let mut value = to_value("app", &def(), "hash", true);
    value.set("span".to_string(), Value::number(0));
    value.set("rows".to_string(), Value::number(0));
    let stored = stored_from_value(&value).unwrap();
    assert!(stored.def.size.is_none());
}
