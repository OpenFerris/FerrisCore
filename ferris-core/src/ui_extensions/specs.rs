//! UI-extension spec types: slots, widgets, dashboards, website.
//!
//! Pure data plus serde shapes, split out of the old `ui_extensions.rs`;
//! the registry and the validation helpers live in sibling files.

use serde::{Deserialize, Serialize};

/// Desk routes no dashboard may shadow: the host's own views and endpoints.
/// A `dashboard.route` inside any of these prefixes (or equal to one) is
/// rejected at registration with a warning.
pub(super) const RESERVED_ROUTE_PREFIXES: &[&str] = &[
    "/api",
    "/plugins",
    "/pkg",
    "/static",
    "/socket.io",
    "/events",
    "/desk",
    "/app",
    "/report",
    "/dashboard",
    "/kanban",
    "/calendar",
    "/settings",
    "/login",
];

/// One declared UI extension: a component the app mounts into a host slot.
///
/// `component` is the custom element the app defines and this declaration
/// mounts — the host renders `<ferris-{component}>` (or an iframe for
/// `component: "iframe"`) into the named slot. `doctype` scopes the extension
/// to one document type for the doctype-aware slots (`form.*`, `list.*`); a
/// slot that is global (`shell.*`) ignores it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UiExtensionSpec {
    /// The host slot this extension binds to, e.g. `form.toolbar.right`,
    /// `list.before_table`, `shell.sidebar.bottom`.
    pub slot: String,
    /// DocType this extension applies to; `None` for the shell-wide slots.
    #[serde(default)]
    pub doctype: Option<String>,
    /// Custom element name the app defines (`"invoice-actions"` → the host
    /// mounts `<ferris-invoice-actions>`); `"iframe"` opts into the iframe
    /// escape hatch instead.
    pub component: String,
    /// Human label for tooltips, menus and empty placeholder text.
    #[serde(default)]
    pub label: Option<String>,
    /// (name, value) pairs the host hands to the mounted element as its
    /// initial props — the slot-side twin of widget `params`.
    #[serde(default)]
    pub props: Vec<(String, String)>,
    /// Permission/doctype-action required to display this extension.
    #[serde(default)]
    pub requires: Option<String>,
}

/// A widget definition's footprint in the `grid-12` layout: columns × rows.
///
/// The spec-side twin of the contract's `GridSize`. These are hints — the
/// layout module clamps whatever they resolve to — so the registry stores them
/// as declared.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GridSizeSpec {
    /// Columns (1..=12).
    pub span: u8,
    /// Rows (1..=64) of 72 px each.
    pub rows: u8,
}

/// Appearance of one widget — a definition's default or an instance's override.
///
/// The spec-side twin of the contract's `StyleSpec`. The duplication is
/// structural, exactly like [`crate::themes::ThemeSpec`]'s: this crate sits
/// *below* `ferris-schema` (which depends on it under the `native` feature), so
/// it cannot name the contract type. The two are kept field-for-field compatible
/// by the round-trip test in `ferris-api`, which projects one into the other.
///
/// Every field is `Option`: an absent field inherits from the level below, and a
/// widget that declares no style renders exactly as it did before this type
/// existed. A **colour field is a semantic token name**, never a literal (the
/// Desk turns it into a class); the `*_custom` twin carries a free-form colour
/// the Desk injects as a CSS variable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StyleSpec {
    /// Semantic token for the background (`"card"`, `"muted"`, `"accent"`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Free-form background colour, used only when `background` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_custom: Option<String>,
    /// Semantic token for the border colour; `"none"` removes the border.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_color: Option<String>,
    /// Border width, in pixels (`0` = no border).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_width: Option<u8>,
    /// Corner radius, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<u8>,
    /// Inner padding, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u8>,
    /// Semantic elevation token: `"none"`, `"xs"`, `"sm"`, `"md"`, `"lg"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<String>,
}

impl StyleSpec {
    /// Whether this style declares nothing at all — the common case.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.background_custom.is_none()
            && self.border_color.is_none()
            && self.border_width.is_none()
            && self.border_radius.is_none()
            && self.padding.is_none()
            && self.shadow.is_none()
    }

    /// `self` (the more specific level) wins over `base`, field by field. The
    /// spec-side twin of the contract's rule, so the registry resolves the same
    /// precedence the Desk will.
    #[must_use]
    pub fn overlay(&self, base: &Self) -> Self {
        Self {
            background: self.background.clone().or_else(|| base.background.clone()),
            background_custom: self
                .background_custom
                .clone()
                .or_else(|| base.background_custom.clone()),
            border_color: self
                .border_color
                .clone()
                .or_else(|| base.border_color.clone()),
            border_width: self.border_width.or(base.border_width),
            border_radius: self.border_radius.or(base.border_radius),
            padding: self.padding.or(base.padding),
            shadow: self.shadow.clone().or_else(|| base.shadow.clone()),
        }
    }

    /// Every numeric field bounded so a declared value cannot break the layout.
    /// Idempotent.
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            background: self.background.clone(),
            background_custom: self.background_custom.clone(),
            border_color: self.border_color.clone(),
            border_width: self.border_width.map(|n| n.min(MAX_BORDER_WIDTH_SPEC)),
            border_radius: self.border_radius.map(|n| n.min(MAX_RADIUS_SPEC)),
            padding: self.padding.map(|n| n.min(MAX_PADDING_SPEC)),
            shadow: self.shadow.clone(),
        }
    }
}

/// Border-width ceiling, mirroring the contract's `MAX_BORDER_WIDTH`.
const MAX_BORDER_WIDTH_SPEC: u8 = 8;
/// Radius ceiling, mirroring the contract's `MAX_RADIUS`.
const MAX_RADIUS_SPEC: u8 = 32;
/// Padding ceiling, mirroring the contract's `MAX_PADDING`.
const MAX_PADDING_SPEC: u8 = 64;

/// Appearance and geometry of a whole board — the Desk's global default, an
/// app's declaration, or a user's per-dashboard override.
///
/// The spec-side twin of the contract's `DashboardStyle` (same structural
/// duplication as [`StyleSpec`]). Carries the board's own style plus the chrome
/// layout flags, so one form configures "how my board looks".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardStyle {
    /// Semantic token for the board's background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Free-form board background colour, used only when `background` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_custom: Option<String>,
    /// Separation between two tiles, in pixels; absent means the Desk's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_px: Option<u8>,
    /// The board's own inner padding, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_px: Option<u8>,
    /// The board's corner radius, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<u8>,
    /// Whether the board draws its title in full-screen mode. Absent means yes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_full_screen: Option<bool>,
}

impl DashboardStyle {
    /// Whether this board style declares nothing at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.background_custom.is_none()
            && self.gap_px.is_none()
            && self.padding_px.is_none()
            && self.border_radius.is_none()
            && self.title_full_screen.is_none()
    }

    /// `self` (the user's override) layered over `base` (the app's declaration
    /// or the Desk's global default), field by field.
    #[must_use]
    pub fn overlay(&self, base: &Self) -> Self {
        Self {
            background: self.background.clone().or_else(|| base.background.clone()),
            background_custom: self
                .background_custom
                .clone()
                .or_else(|| base.background_custom.clone()),
            gap_px: self.gap_px.or(base.gap_px),
            padding_px: self.padding_px.or(base.padding_px),
            border_radius: self.border_radius.or(base.border_radius),
            title_full_screen: self.title_full_screen.or(base.title_full_screen),
        }
    }

    /// Every numeric field bounded. Idempotent.
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            background: self.background.clone(),
            background_custom: self.background_custom.clone(),
            gap_px: self.gap_px.map(|n| n.min(MAX_GAP_SPEC)),
            padding_px: self.padding_px.map(|n| n.min(MAX_PADDING_SPEC)),
            border_radius: self.border_radius.map(|n| n.min(MAX_RADIUS_SPEC)),
            title_full_screen: self.title_full_screen,
        }
    }
}

/// Gap ceiling, mirroring the contract's `MAX_GAP`.
const MAX_GAP_SPEC: u8 = 64;

/// Schema of one parameter a widget accepts: what an instance may configure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WidgetParamSpec {
    /// Parameter name, kebab-case (`doctype`, `page-size`).
    pub name: String,
    /// Human label for a configuration UI.
    #[serde(default)]
    pub label: Option<String>,
    /// Value type: `"string"`, `"number"`, `"bool"`, `"doctype"`, `"json"` —
    /// an open string; the host only transports values as strings and the
    /// widget coerces them. Defaults to `"string"` when absent.
    #[serde(default = "default_value_type", rename = "value-type")]
    pub value_type: String,
    /// Fallback when an instance omits the parameter.
    #[serde(default)]
    pub default: Option<String>,
    /// Allowed values (for selects); empty means free-form.
    #[serde(default)]
    pub options: Vec<String>,
    /// Whether instances must provide the parameter.
    #[serde(default)]
    pub required: bool,
    /// Doc hint for the configuration UI.
    #[serde(default)]
    pub description: Option<String>,
    /// Form section this parameter belongs to in the properties dialog
    /// (`"Datos"`, `"Presentación"`, …); absent means the default group.
    #[serde(default)]
    pub group: Option<String>,
    /// Input placeholder shown while the parameter is empty.
    #[serde(default)]
    pub placeholder: Option<String>,
    /// Expression deciding whether the settings panel shows this parameter
    /// (the form renderer's `depends_on` language). `None` = always visible.
    #[serde(default)]
    pub depends_on: Option<String>,
    /// Render the control disabled while still storing its value.
    #[serde(default)]
    pub read_only: bool,
    /// Keep the parameter out of the settings panel, preserving its stored
    /// value untouched.
    #[serde(default)]
    pub hidden: bool,
}

fn default_value_type() -> String {
    "string".to_string()
}

/// One reusable widget definition published to the global catalog.
///
/// N instances of one definition mount N copies of the same `component` with
/// different parameter values — that is what makes a widget reusable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WidgetDefSpec {
    /// Name unique within the publishing app, kebab-case; the global
    /// reference is `owner.name`.
    pub name: String,
    /// Human label shown above the widget when an instance does not override it.
    #[serde(default)]
    pub label: String,
    /// Custom element the publishing app defines (`"task-chart"` → the host
    /// mounts `<ferris-task-chart>`). Host-owned widgets are rendered natively
    /// by the Desk and this is a synthetic name (`"host-stat"`).
    pub component: String,
    /// Widget kind: `"stat"`, `"chart"`, `"table"` — how the desk sizes and
    /// frames the widget (and its default grid span). Kept as an open string
    /// so a kind an app invents degrades to a generic framed body instead of
    /// failing to render.
    #[serde(rename = "type")]
    pub kind: String,
    /// The parameters instances may configure.
    #[serde(default)]
    pub params: Vec<WidgetParamSpec>,
    /// Longer description for catalogs/docs.
    #[serde(default)]
    pub description: Option<String>,
    /// Permission/doctype-action required to mount/use this widget.
    #[serde(default)]
    pub requires: Option<String>,
    /// Realtime events the widget subscribes to (server event names)
    /// that should trigger a refresh/update on this widget instance.
    #[serde(default)]
    pub events: Vec<String>,
    /// Short glyph (emoji or one or two characters) for the builder palette.
    #[serde(default)]
    pub icon: Option<String>,
    /// Free-form palette category; the builder groups the catalog by it.
    #[serde(default)]
    pub category: Option<String>,
    /// Preview image (svg/png/webp) the Desk draws on the catalog's cards
    /// (`/static/previews/…` for the host's built-ins, the app's own asset
    /// URL for a plugin). Absent → the Desk's built-in placeholder per `kind`.
    #[serde(default)]
    pub preview: Option<String>,
    /// Footprint a new instance starts with; absent means the `kind` default.
    #[serde(default)]
    pub size: Option<GridSizeSpec>,
    /// Smallest footprint the builder lets the user shrink the widget to.
    #[serde(default)]
    pub min: Option<GridSizeSpec>,
    /// Default appearance this definition gives every instance (background,
    /// border, radius, padding, shadow). Absent means the Desk's defaults; an
    /// instance overrides it field by field.
    ///
    /// The **spec-side twin** of the contract's field, and the same type: the
    /// style vocabulary is generic (a widget or a board wears it), so there is
    /// nothing for a `*Spec` mirror to translate and a second copy would only
    /// be a chance for the two to drift. `None` is the normal case — most apps
    /// declare no style, and the Desk then resolves its own default. Skipped on
    /// the wire so a style-less spec serializes exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleSpec>,
    /// Native renderer id for a host-owned widget (`stat`, `trend`, `table`).
    /// `None` means the widget is code-backed and mounts its `component`.
    #[serde(default)]
    pub renderer: Option<String>,
    /// Seed version the publishing app last migrated, stored with the row and
    /// included in the seed hash, so bumping it re-applies the seed.
    #[serde(default)]
    pub version: Option<String>,
    /// Original asset path of the widget module (`ui/widgets/charts/bar.js`).
    ///
    /// The host serves the DB-stored [`Self::script`] at this path, so relative
    /// `import`s inside the module keep resolving against the app's assets. A
    /// widget without a path is served from `/api/widgets/{app}/{name}/script.js`
    /// and must be self-contained.
    #[serde(default)]
    pub code_path: Option<String>,
    /// Full JavaScript module source of the widget, stored in the `Widget`
    /// DocType and editable from the Desk.
    #[serde(default)]
    pub script: Option<String>,
    /// Full CSS source of the widget, stored beside the script.
    #[serde(default)]
    pub stylesheet: Option<String>,
    /// Optional server-rendered markup kept with the widget definition.
    #[serde(default)]
    pub ssr: Option<String>,
}

/// One widget *instance* inside an app-declared dashboard: a reference to a
/// global widget definition plus the concrete configuration for this placement.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WidgetInstanceSpec {
    /// Instance id, unique within the dashboard, e.g. `"monthly-sales"`.
    pub id: String,
    /// Reference to the definition: `owner.name` (`host.stat`,
    /// `otherapp.kpi`), or bare `name` for a widget of the dashboard's own app.
    #[serde(default)]
    pub widget: String,
    /// Overrides the definition's label for this instance.
    #[serde(default)]
    pub label: Option<String>,
    /// Parameter values for this instance, as (name, value) pairs; the value
    /// is coerced per the definition's `value-type`.
    #[serde(default)]
    pub params: Vec<(String, String)>,
    /// Grid columns (1..12) in a `grid-12` layout; `None` means the
    /// definition's `kind` default (`stat`→3, `chart`→6, `table`→12).
    #[serde(default)]
    pub span: Option<u8>,
    /// 0-based grid column (x) of the widget's top-left cell; `None` means
    /// "place me at the first free cell".
    #[serde(default)]
    pub col: Option<u8>,
    /// 0-based grid row (y) of the widget's top-left cell.
    #[serde(default)]
    pub row: Option<u16>,
    /// Grid rows the widget is tall; `None` means the definition's declared
    /// `size.rows`, else three.
    #[serde(default)]
    pub rows: Option<u8>,
    /// This placement's own appearance, layered over the definition's
    /// [`WidgetDefSpec::style`] field by field. Absent means "inherit the
    /// definition's style, which itself inherits the Desk's defaults".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleSpec>,
}

/// A dashboard an app declares: a route with an ordered layout of widgets.
///
/// Dashboards always have a route; an app wanting to expose a single widget
/// publishes its `WidgetDefSpec` instead and lets any dashboard instance it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardSpec {
    /// Unique id, e.g. `"sales-overview"`.
    pub id: String,
    /// Human label for the dashboard switcher/title.
    #[serde(default)]
    pub label: String,
    /// Desk route the dashboard is mounted at (`/ops`, `/sales`). Required:
    /// widget-only dashboards do not exist anymore — publish a widget
    /// definition instead. Validated against [`RESERVED_ROUTE_PREFIXES`] at
    /// registration.
    pub route: String,
    /// The dashboard shown at `/` when `true`. At most one per registry
    /// (first registered wins; later flags are cleared with a warning).
    #[serde(default)]
    pub default: bool,
    /// Board appearance and geometry the app declares (background, gap,
    /// padding, chrome flags). A user's per-dashboard override layers over it,
    /// and that over the Desk's global default; absent means neither the app
    /// nor the user configured the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<DashboardStyle>,
    /// Widget instances, in layout order.
    #[serde(default)]
    pub widgets: Vec<WidgetInstanceSpec>,
}

/// One website route an app publishes (Frappe-style web page).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WebsiteRouteSpec {
    /// Route path, e.g. `"/blog"` or `"/blog/:slug"`.
    pub path: String,
    /// Custom element for the page body.
    pub component: String,
    /// Route kind: `"page"` (the default) or `"bare"`.
    ///
    /// A `"bare"` route owns the whole viewport: the Desk renders it with no
    /// sidebar and no top bar, the declaration apps use for a full-screen
    /// canvas, editor or kiosk surface. Kept an open string so further kinds
    /// (`"feed"`, …) need no wire change.
    #[serde(rename = "type")]
    pub kind: String,
}

/// Website publishing declaration of an app.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WebsiteSpec {
    /// Routes, in declaration order.
    #[serde(default)]
    pub routes: Vec<WebsiteRouteSpec>,
}

/// One app's declared UI extension set, as it arrived from its `describe()`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppUi {
    /// Owner tag: the plugin/app id that declared this set.
    pub owner: String,
    /// Slot extensions, in declaration order.
    #[serde(default)]
    pub extensions: Vec<UiExtensionSpec>,
    /// Dashboards, in declaration order.
    #[serde(default)]
    pub dashboards: Vec<DashboardSpec>,
    /// Widget definitions this app publishes to the global catalog.
    #[serde(default)]
    pub widgets: Vec<WidgetDefSpec>,
    /// Website publishing, if the app publishes public pages.
    #[serde(default)]
    pub website: Option<WebsiteSpec>,
    /// UI asset paths (scripts/stylesheets) the Desk loads to define the
    /// declared custom elements. Served through the plugin's `ui-asset`
    /// export at `/plugins/{owner}/asset/{path}`.
    #[serde(default)]
    pub assets: Vec<String>,
}

/// The declaration side of [`register_app_ui`]: everything one app's
/// `describe()` contributed, minus the owner tag the caller supplies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppUiSpec {
    /// Slot extensions, in declaration order.
    pub extensions: Vec<UiExtensionSpec>,
    /// Dashboards, in declaration order.
    pub dashboards: Vec<DashboardSpec>,
    /// Widget definitions published to the global catalog.
    pub widgets: Vec<WidgetDefSpec>,
    /// Website publishing, if the app publishes public pages.
    pub website: Option<WebsiteSpec>,
    /// UI asset paths the Desk loads for this app.
    pub assets: Vec<String>,
}

impl AppUiSpec {
    /// Whether the spec declares nothing at all.
    pub(super) const fn is_empty(&self) -> bool {
        self.extensions.is_empty()
            && self.dashboards.is_empty()
            && self.widgets.is_empty()
            && self.website.is_none()
            && self.assets.is_empty()
    }
}
