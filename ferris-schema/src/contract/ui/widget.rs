//! The widget catalog: definitions, instances and their footprint.

use serde::{Deserialize, Serialize};

use crate::contract::style::StyleSpec;

/// A widget's footprint in the `grid-12` layout: columns × rows.
///
/// Mirrors the definition-side pair (`widget-def.size` / `widget-def.min`):
/// `size` is the footprint a new instance starts with and `min` the one the
/// builder refuses to shrink below.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GridSize {
    /// Columns (1..=12), the same axis as [`WidgetInstance::span`].
    pub span: u8,
    /// Rows (1..=64) of [`crate::contract::layout::CELL_HEIGHT_PX`] each.
    pub rows: u8,
}

/// Schema of one parameter a widget accepts: what an instance may configure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WidgetParam {
    /// Parameter name, kebab-case (`doctype`, `page-size`).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Open value type: `"string"`, `"number"`, `"bool"`, `"doctype"`,
    /// `"json"` — the host only transports strings; the widget coerces.
    #[serde(rename = "value-type")]
    pub value_type: String,
    /// Fallback when an instance omits the parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Allowed values (for selects); empty means free-form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Form section this parameter belongs to in the properties dialog
    /// (`"Datos"`, `"Presentación"`, …); absent means the dialog's default
    /// group. Purely presentational: values are always keyed by `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Input placeholder shown while the parameter is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Expression that decides whether the settings panel shows this
    /// parameter, evaluated with the form renderer's own small language
    /// (field names, literals, comparisons, `&&`/`||`/`!`, `eval:`/`doc.`
    /// prefixes). Absent means always visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    /// Render the control disabled while still storing its value.
    #[serde(default)]
    pub read_only: bool,
    /// Keep the parameter out of the settings panel (its stored value is
    /// preserved untouched).
    #[serde(default)]
    pub hidden: bool,
}

/// One reusable widget definition in the global catalog: any app's dashboard
/// can instance it by reference `owner.name`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WidgetDef {
    /// Name unique within the publishing app, kebab-case.
    pub name: String,
    /// Human label shown above the widget when an instance does not override it.
    #[serde(default)]
    pub label: String,
    /// Custom element the publishing app defines (`"task-chart"` →
    /// `<ferris-task-chart>`).
    pub component: String,
    /// Widget kind: `"chart"`, `"stat"`, `"table"` — how the Desk frames the
    /// widget and what grid span it gets by default.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub params: Vec<WidgetParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// Short glyph (emoji or one or two characters) drawn on the palette card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Free-form palette category (`"Cards"`, `"Charts"`, …). The builder
    /// groups the catalog by it and an app may invent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Preview image (svg/png/webp) drawn on the catalog's cards: an URL the
    /// client can `<img src>` (same-origin `/static/…` for built-ins, the
    /// publishing app's own asset for plugins). Absent means the Desk paints
    /// its built-in placeholder for the widget's `kind` (WIT 9.0.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// URL of the `JavaScript` module the Desk must load before mounting
    /// `component`, served by the host from the `Widget` `DocType`.
    ///
    /// The host keeps the module at the app's original asset path
    /// (`/plugins/{app}/asset/{code-path}`) when the definition declares one,
    /// so relative `import`s still resolve; otherwise it falls back to
    /// `/api/widgets/{app}/{name}/script.js`. Absent means the definition is
    /// rendered natively by the Desk (host widgets) or has no code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_url: Option<String>,
    /// URL of the widget's own CSS module, when its `Widget` row stores one.
    /// App-wide stylesheets still arrive through the app's asset manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_url: Option<String>,
    /// Footprint a new instance starts with; absent means the `kind` default
    /// (`stat`→3, `table`→12, anything else→6, always [`DEFAULT_ROWS`]).
    ///
    /// [`DEFAULT_ROWS`]: crate::contract::layout::DEFAULT_ROWS
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<GridSize>,
    /// Smallest footprint the builder lets the user shrink the widget to;
    /// absent means one cell in both axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<GridSize>,
    /// Default appearance this definition gives every instance: background,
    /// border, radius, padding, shadow. Absent means the Desk's defaults; an
    /// instance may override it field by field (see
    /// [`WidgetInstance::style`]). Optional on purpose — an app that declares
    /// no style keeps rendering exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleSpec>,
}

/// One widget *instance* inside a dashboard: a reference to a global
/// [`WidgetDef`] plus this placement's configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WidgetInstance {
    /// Instance id, unique within the dashboard.
    pub id: String,
    /// Reference to the definition: `owner.name` (`host.stat`,
    /// `otherapp.kpi`), or bare `name` for the dashboard's own app.
    #[serde(default)]
    pub widget: String,
    /// Overrides the definition's label for this instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether the host paints this instance's title.
    ///
    /// Absent means "yes": the heading is [`WidgetInstance::label`], else the
    /// definition's label, else the reference. Hiding it keeps the text in the
    /// stored layout, so turning it back on restores the same heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_title: Option<bool>,
    /// Subtitle the host paints under the title.
    ///
    /// Absent or empty falls back to the definition's `description`, which is
    /// how a widget documents itself without painting anything; the instance
    /// override wins. The host only draws it when
    /// [`WidgetInstance::show_subtitle`] says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Whether the host paints the subtitle.
    ///
    /// Absent means "no": subtitles are opt-in. This is deliberately
    /// independent from [`WidgetInstance::subtitle`] so an editor can keep a
    /// text but hide it, and toggle it back without retyping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_subtitle: Option<bool>,
    /// (name, value) pairs for this instance; coerced per `value-type`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<(String, String)>,
    /// Grid columns (1..12) in a `grid-12` layout; absent means the
    /// definition's `kind` default (`stat`→3, `chart`→6, `table`→12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<u8>,
    /// 0-based grid column (x) of the widget's top-left cell; absent means
    /// "place me at the first free cell", which
    /// [`crate::contract::layout::normalize`] resolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u8>,
    /// 0-based grid row (y) of the widget's top-left cell; absent behaves like
    /// an absent [`WidgetInstance::col`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u16>,
    /// Grid rows the widget is tall; absent means the definition's declared
    /// `size.rows`, else [`DEFAULT_ROWS`].
    ///
    /// [`DEFAULT_ROWS`]: crate::contract::layout::DEFAULT_ROWS
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u8>,
    /// This instance's appearance, layered over its definition's
    /// [`WidgetDef::style`] field by field. Absent means "inherit the
    /// definition's style, which itself inherits the Desk's defaults".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleSpec>,
}

/// The widget instances a JSON slot carries, from one shared parsing policy.
///
/// This is the reader for a document's bound `widgets` field under the
/// `document` dashboard binding ([`crate::contract::DashboardView`]): the
/// Desk's seed and the layout endpoint's answer decode the payload the same
/// way instead of two.
///
/// Two shapes are legitimate: a `Code` field stores the list as a
/// JSON-encoded string, a JSON field stores the array itself, and a document
/// nobody has edited yet holds neither. Anything else — malformed JSON, a
/// non-list value, one broken element — yields fewer instances rather than an
/// error: a dashboard must still render (and be re-saved into a valid shape)
/// when part of the payload went bad, the same tolerance the internal
/// `Dashboard` conversion applies to its child rows.
#[must_use]
pub fn widget_instances(value: Option<&serde_json::Value>) -> Vec<WidgetInstance> {
    let Some(value) = value else {
        return Vec::new();
    };
    let decoded;
    let value = match value {
        serde_json::Value::Array(_) => value,
        serde_json::Value::String(text) => {
            decoded =
                serde_json::from_str::<serde_json::Value>(text).unwrap_or(serde_json::Value::Null);
            &decoded
        }
        _ => return Vec::new(),
    };
    value.as_array().map_or_else(Vec::new, |items| {
        items
            .iter()
            .filter_map(|item| serde_json::from_value::<WidgetInstance>(item.clone()).ok())
            .collect()
    })
}
