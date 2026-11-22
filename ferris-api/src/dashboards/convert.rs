//! Document → wire-contract conversion for `Dashboard` rows.
//!
//! The `Dashboard` `DocType` stores a definition the way the engine stores any
//! document: the parent's `data` blob holds scalars, and the `widgets` child
//! table lives in `data.widgets` as an array of row objects (`idx`, `parent`,
//! `parenttype`, `parentfield` plus the row's own fields) — the same shape the
//! `DocField` child table uses.
//!
//! Every function here is pure and total: a malformed row yields a dashboard
//! with fewer widgets rather than an error, because a hand-written document must
//! not be able to break the feed. The one exception is a row with no `name`,
//! which [`to_dashboard`] rejects — a dashboard the layout endpoint cannot
//! address is not a dashboard.

use ferris_core::Value;
use ferris_schema::contract;

/// One `Dashboard` document as a feed dashboard, or `None` when it has no name.
pub(super) fn to_dashboard(value: &Value) -> Option<contract::Dashboard> {
    let id = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())?
        .to_string();
    // The label is the human title; a document that left it blank still needs a
    // header, so it falls back to the id.
    let label = value
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map_or_else(|| id.clone(), str::to_string);
    let route = value
        .get("route")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    Some(contract::Dashboard {
        id,
        label,
        route,
        default: truthy(value.get("is_default")),
        // The board style the document declares; a user's override lives in the
        // shared `Dashboard Layout` row and layers over it.
        style: style_of(value.get("style")),
        widgets: instances(value.get("widgets")),
    })
}

/// The `widgets` child rows of a dashboard as feed instances.
///
/// A row without a `widget` reference is dropped: an instance that names no
/// definition can never render. The instance id falls back to the row's `idx`
/// (`row-{idx}`) so a hand-written child row does not need a `widget_id`.
pub(super) fn instances(raw: Option<&Value>) -> Vec<contract::WidgetInstance> {
    let Some(rows) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(rows.len());
    for (position, row) in rows.iter().enumerate() {
        let reference = row
            .get("widget")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|reference| !reference.is_empty());
        let Some(reference) = reference else {
            continue;
        };
        let idx = row
            .get("idx")
            .and_then(Value::as_i64)
            .and_then(|idx| u64::try_from(idx).ok())
            .unwrap_or(position as u64 + 1);
        let id = row
            .get("widget_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map_or_else(|| format!("row-{idx}"), str::to_string);
        out.push(contract::WidgetInstance {
            id,
            widget: reference.to_string(),
            label: row
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_string),
            // The `Dashboard Widget` child table has no chrome columns (unlike
            // the Desk's layout row): the host defaults apply to a document
            // dashboard, and the layout editor overlays its own values on the
            // saved row.
            show_title: None,
            subtitle: None,
            show_subtitle: None,
            params: params(row.get("params")),
            span: dimension(row.get("span"), 1, 12),
            col: dimension(row.get("col"), 0, 11),
            row: row
                .get("row")
                .and_then(Value::as_i64)
                .and_then(|row| u16::try_from(row).ok()),
            rows: dimension(row.get("rows"), 1, u8::MAX),
            // The instance's own style; absent means it inherits the
            // definition's declared default.
            style: style_of(row.get("style")),
        });
    }
    out
}

/// A JSON-typed style field as the contract's shape.
///
/// The field holds either the object a `JSON` field stores or, in a
/// hand-authored row, the serialized string of that object; both are accepted.
/// A malformed value reads as "no declared style" rather than as an error,
/// exactly like the other forgiving readers in this module — the definition's
/// default and the Desk's theme then apply, which is a perfectly good board to
/// show over a row nobody can parse.
fn style_of<T: serde::de::DeserializeOwned>(raw: Option<&Value>) -> Option<T> {
    let raw = raw?;
    raw.as_str().map_or_else(
        || serde_json::from_value(raw.to_json()).ok(),
        |text| serde_json::from_str(text).ok(),
    )
}

/// A `params` value as `(name, value)` pairs.
///
/// Two hand-authorable shapes are accepted: the JSON array of pairs a document
/// stores (`[["doctype", "ToDo"], …]`) and an array of `{ "name": …, "value": …
/// }` objects. A pair with a blank name is dropped, and a non-scalar value
/// becomes the empty string — a parameter is always a name and one scalar.
fn params(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(pair) = item.as_array() {
                let name = pair.first()?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                return Some((
                    name.to_string(),
                    pair.get(1).map_or_else(String::new, scalar_string),
                ));
            }
            let name = item.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            Some((
                name.to_string(),
                item.get("value").map_or_else(String::new, scalar_string),
            ))
        })
        .collect()
}

/// A parameter value as the string the contract carries; non-scalars are empty.
fn scalar_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    }
}

/// A small grid dimension clamped into `min..=max`; `None` when absent or not an
/// integer, which the shared layout rule ([`contract::layout`]) then fills.
fn dimension(value: Option<&Value>, min: u8, max: u8) -> Option<u8> {
    value
        .and_then(Value::as_i64)
        .and_then(|number| u8::try_from(number).ok())
        .map(|number| number.clamp(min, max))
}

/// A `Check` field read as a bool.
///
/// The engine stores a `Check` default as a real bool and validates writes as a
/// bool, but a document written by a raw JSON client may carry `"1"`/`"true"`;
/// both are accepted so a legacy row keeps working.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => matches!(text.trim(), "1" | "true" | "True" | "yes" | "Yes"),
        Some(Value::Number(number)) => number.as_i64().is_some_and(|number| number != 0),
        Some(Value::Null | Value::Array(_) | Value::Object(_)) | None => false,
    }
}
