//! Conversion between the engine's stored `Widget` row, the registry spec and
//! the wire contract. The row is the source of truth; `WidgetDefSpec` is the
//! shape the registry and the layout helpers understand, and
//! `contract::WidgetDef` is what `/api/ui-extensions` publishes.

use ferris_core::Value;
use ferris_core::ui_extensions::{GridSizeSpec, StyleSpec, WidgetDefSpec, WidgetParamSpec};
use ferris_schema::contract;
use serde_json::Value as JsonValue;

use super::{API_PREFIX, Stored, document_name};

/// One widget definition as the document body the engine stores.
///
/// `create` is false on an update: the seed does not touch `enabled`, so an
/// administrator who disabled a row does not have it re-enabled by the next
/// boot, and the document `name` is already the update path.
pub(super) fn to_value(app: &str, def: &WidgetDefSpec, hash: &str, create: bool) -> Value {
    let mut body = serde_json::Map::new();
    body.insert(
        "name".to_string(),
        JsonValue::String(document_name(app, &def.name)),
    );
    body.insert("app".to_string(), JsonValue::String(app.trim().to_string()));
    body.insert(
        "widget_name".to_string(),
        JsonValue::String(def.name.trim().to_string()),
    );
    body.insert("label".to_string(), JsonValue::String(def.label.clone()));
    body.insert(
        "component".to_string(),
        JsonValue::String(def.component.clone()),
    );
    body.insert("kind".to_string(), JsonValue::String(def.kind.clone()));
    body.insert("renderer".to_string(), opt_string(def.renderer.as_deref()));
    body.insert(
        "description".to_string(),
        opt_string(def.description.as_deref()),
    );
    body.insert("requires".to_string(), opt_string(def.requires.as_deref()));
    body.insert("events".to_string(), json_value(&def.events));
    body.insert("icon".to_string(), opt_string(def.icon.as_deref()));
    body.insert("category".to_string(), opt_string(def.category.as_deref()));
    body.insert("preview".to_string(), opt_string(def.preview.as_deref()));
    body.insert("style".to_string(), json_value(&def.style));
    body.insert("span".to_string(), size_span(def.size));
    body.insert("rows".to_string(), size_rows(def.size));
    body.insert("min_span".to_string(), size_span(def.min));
    body.insert("min_rows".to_string(), size_rows(def.min));
    body.insert(
        "params".to_string(),
        JsonValue::Array(
            def.params
                .iter()
                .enumerate()
                .map(|(index, param)| param_value(index, param))
                .collect(),
        ),
    );
    if create {
        body.insert("enabled".to_string(), JsonValue::Bool(true));
    }
    body.insert("version".to_string(), opt_string(def.version.as_deref()));
    body.insert(
        "code_path".to_string(),
        opt_string(def.code_path.as_deref()),
    );
    body.insert("script".to_string(), opt_string(def.script.as_deref()));
    body.insert(
        "stylesheet".to_string(),
        opt_string(def.stylesheet.as_deref()),
    );
    body.insert("ssr".to_string(), opt_string(def.ssr.as_deref()));
    body.insert("seed_hash".to_string(), JsonValue::String(hash.to_string()));
    Value::from_json(JsonValue::Object(body))
}

/// One `Widget Param` child row. `idx` is 1-based, as the engine's child rows
/// are; `param_name` avoids `name`, which a child row may use for its own id.
fn param_value(index: usize, param: &WidgetParamSpec) -> JsonValue {
    let mut row = serde_json::Map::new();
    row.insert("idx".to_string(), JsonValue::from(index + 1));
    row.insert(
        "param_name".to_string(),
        JsonValue::String(param.name.clone()),
    );
    row.insert("label".to_string(), opt_string(param.label.as_deref()));
    row.insert(
        "value_type".to_string(),
        JsonValue::String(param.value_type.clone()),
    );
    row.insert("default".to_string(), opt_string(param.default.as_deref()));
    row.insert("options".to_string(), json_value(&param.options));
    row.insert("required".to_string(), JsonValue::Bool(param.required));
    row.insert(
        "description".to_string(),
        opt_string(param.description.as_deref()),
    );
    row.insert("group".to_string(), opt_string(param.group.as_deref()));
    row.insert(
        "placeholder".to_string(),
        opt_string(param.placeholder.as_deref()),
    );
    row.insert(
        "depends_on".to_string(),
        opt_string(param.depends_on.as_deref()),
    );
    row.insert("read_only".to_string(), JsonValue::Bool(param.read_only));
    row.insert("hidden".to_string(), JsonValue::Bool(param.hidden));
    JsonValue::Object(row)
}

/// A stored row as its spec shape, or `None` when it has no usable identity.
pub(super) fn stored_from_value(row: &Value) -> Option<Stored> {
    let document = row.get("name").and_then(Value::as_str).unwrap_or_default();
    let app = row
        .get("app")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|app| !app.is_empty())
        .map(str::to_string)
        .or_else(|| document.rsplit_once('.').map(|(app, _)| app.to_string()))?;
    let name = row
        .get("widget_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| document.split_once('.').map(|(_, name)| name.to_string()))?;

    let size = grid_size(int_field(row, "span"), int_field(row, "rows"));
    let min = grid_size(int_field(row, "min_span"), int_field(row, "min_rows"));
    let def = WidgetDefSpec {
        name,
        label: string_field(row, "label"),
        component: string_field(row, "component"),
        kind: string_field(row, "kind"),
        params: row
            .get("params")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(stored_param).collect())
            .unwrap_or_default(),
        description: opt_field(row, "description"),
        requires: opt_field(row, "requires"),
        events: string_array(row, "events"),
        icon: opt_field(row, "icon"),
        category: opt_field(row, "category"),
        preview: opt_field(row, "preview"),
        size,
        min,
        style: style_field(row, "style"),
        renderer: opt_field(row, "renderer"),
        version: opt_field(row, "version"),
        code_path: opt_field(row, "code_path"),
        script: opt_field(row, "script"),
        stylesheet: opt_field(row, "stylesheet"),
        ssr: opt_field(row, "ssr"),
    };
    Some(Stored {
        app,
        def,
        enabled: bool_field(row, "enabled", true),
        seed_hash: string_field(row, "seed_hash"),
    })
}

/// One stored `Widget Param` row as its spec shape.
fn stored_param(row: &Value) -> Option<WidgetParamSpec> {
    let name = row
        .get("param_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())?
        .to_string();
    Some(WidgetParamSpec {
        name,
        label: opt_field(row, "label"),
        value_type: opt_field(row, "value_type").unwrap_or_else(|| "string".to_string()),
        default: opt_field(row, "default"),
        options: string_array(row, "options"),
        required: bool_field(row, "required", false),
        description: opt_field(row, "description"),
        group: opt_field(row, "group"),
        placeholder: opt_field(row, "placeholder"),
        depends_on: opt_field(row, "depends_on"),
        read_only: bool_field(row, "read_only", false),
        hidden: bool_field(row, "hidden", false),
    })
}

/// A stored definition as the wire shape the Desk consumes.
pub(super) fn to_contract(stored: &Stored) -> contract::WidgetDef {
    let app = stored.app.as_str();
    let name = stored.def.name.as_str();
    let renderer = stored.def.renderer.as_deref().unwrap_or_default();
    let script_version = stored.def.script.as_deref().and_then(source_version);
    let code_url = if renderer.trim().is_empty() {
        match stored.def.code_path.as_deref() {
            Some(path) if !path.trim().is_empty() => Some(with_version(
                format!("/plugins/{app}/asset/{}", path.trim_start_matches('/')),
                script_version.as_deref(),
            )),
            _ if script_version.is_some() => Some(with_version(
                format!("{API_PREFIX}/{app}/{name}/script.js"),
                script_version.as_deref(),
            )),
            _ => None,
        }
    } else {
        // Native renderer: the Desk draws it; there is no module to load.
        None
    };
    let style_url = stored
        .def
        .stylesheet
        .as_deref()
        .and_then(source_version)
        .map(|version| format!("{API_PREFIX}/{app}/{name}/style.css?v={version}"));

    contract::WidgetDef {
        name: stored.def.name.clone(),
        label: stored.def.label.clone(),
        component: stored.def.component.clone(),
        kind: stored.def.kind.clone(),
        params: stored
            .def
            .params
            .iter()
            .map(|param| contract::WidgetParam {
                name: param.name.clone(),
                label: param.label.clone(),
                value_type: param.value_type.clone(),
                default: param.default.clone(),
                options: param.options.clone(),
                required: param.required,
                description: param.description.clone(),
                group: param.group.clone(),
                placeholder: param.placeholder.clone(),
                depends_on: param.depends_on.clone(),
                read_only: param.read_only,
                hidden: param.hidden,
            })
            .collect(),
        description: stored.def.description.clone(),
        requires: stored.def.requires.clone(),
        events: stored.def.events.clone(),
        icon: stored.def.icon.clone(),
        category: stored.def.category.clone(),
        preview: stored.def.preview.clone(),
        code_url,
        style_url,
        size: stored.def.size.map(contract_size),
        min: stored.def.min.map(contract_size),
        style: stored.def.style.clone().map(contract_style),
    }
}

/// The content fingerprint of a widget source.
///
/// It travels in the URL (`?v=…`) so editing a script or stylesheet in the Desk
/// changes both the module URL and the feed revision. Without it the wire
/// payload would be identical after a source-only edit, the Desk would skip the
/// refresh, and the browser's module cache would keep the old module.
fn source_version(source: &str) -> Option<String> {
    (!source.trim().is_empty()).then(|| contract::revision_of(source.as_bytes()))
}

/// Append the optional content version to a code URL.
fn with_version(mut url: String, version: Option<&str>) -> String {
    if let Some(version) = version {
        url.push_str("?v=");
        url.push_str(version);
    }
    url
}

/// The spec-side footprint as the wire shape.
const fn contract_size(size: GridSizeSpec) -> contract::GridSize {
    contract::GridSize {
        span: size.span,
        rows: size.rows,
    }
}

/// The spec-side widget appearance as the wire shape. Mirrors
/// `ferris_api::ui::style_spec`, which is private to that module.
fn contract_style(style: StyleSpec) -> contract::StyleSpec {
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

/// A non-empty string field, else `""`.
fn string_field(row: &Value, key: &str) -> String {
    opt_field(row, key).unwrap_or_default()
}

/// A non-empty string field, else `None`.
fn opt_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// An integer field; a JSON number or a numeric string both count.
fn int_field(row: &Value, key: &str) -> Option<u8> {
    let value = row.get(key)?;
    let number = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|raw| raw.trim().parse().ok()))?;
    u8::try_from(number).ok()
}

/// A boolean field accepting the engine's stored bool or a `0`/`1`.
fn bool_field(row: &Value, key: &str, fallback: bool) -> bool {
    match row.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_i64().is_some_and(|value| value != 0),
        Some(Value::String(value)) => matches!(value.trim(), "1" | "true" | "yes"),
        _ => fallback,
    }
}

/// A JSON array of strings; a serialized JSON string is accepted too, because a
/// hand-edited `Data`/`Code` field may hold one.
fn string_array(row: &Value, key: &str) -> Vec<String> {
    match row.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A JSON object/array field, tolerating a serialized copy.
fn style_field(row: &Value, key: &str) -> Option<StyleSpec> {
    match row.get(key)? {
        Value::Object(_) => serde_json::from_value(row.get(key)?.to_json()).ok(),
        Value::String(raw) => serde_json::from_str(raw).ok(),
        _ => None,
    }
}

/// A footprint from two optional axes: absent when either is missing/zero, so a
/// hand-written row does not accidentally claim a `0×0` cell.
fn grid_size(span: Option<u8>, rows: Option<u8>) -> Option<GridSizeSpec> {
    let span = span.filter(|value| *value > 0)?;
    let rows = rows.filter(|value| *value > 0)?;
    Some(GridSizeSpec { span, rows })
}

/// An optional string as a JSON value.
fn opt_string(value: Option<&str>) -> JsonValue {
    value.map_or(JsonValue::Null, |value| {
        JsonValue::String(value.to_string())
    })
}

/// A serde value as JSON; a serialization failure becomes `Null` rather than a
/// panic on a request path.
fn json_value<T: serde::Serialize>(value: &T) -> JsonValue {
    serde_json::to_value(value).unwrap_or(JsonValue::Null)
}

/// The `span` axis of a footprint, `0` when absent.
fn size_span(size: Option<GridSizeSpec>) -> JsonValue {
    JsonValue::from(size.map_or(0, |size| size.span))
}

/// The `rows` axis of a footprint, `0` when absent.
fn size_rows(size: Option<GridSizeSpec>) -> JsonValue {
    JsonValue::from(size.map_or(0, |size| size.rows))
}
