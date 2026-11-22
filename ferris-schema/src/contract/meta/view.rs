//! Which view a doctype's documents open, and the dashboard binding behind
//! [`DocView::Dashboard`].

use serde::{Deserialize, Serialize};

/// A **collection** view: one of the many-records renderings of a doctype,
/// as opposed to the document views of [`DocView`].
///
/// The Desk's collection routes are
/// `/desk/:module/:doctype/<kind>` with `kind` in this enum; the bare index
/// (`/desk/:module/:doctype`) is the list. A doctype declares which ones it
/// supports with the `in_*` flags and, optionally, which one opens by default
/// (`DoctypeMeta::default_view`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionView {
    /// The tabular list (the route index).
    List,
    /// Records grouped in columns by a `Select`/`Link` field.
    Kanban,
    /// Records placed on a calendar by their date field(s).
    Calendar,
    /// Records nested by a `Link` to the same doctype.
    Tree,
    /// Records drawn as bars between a start and an end date.
    Gantt,
    /// The read-only report of the doctype's data.
    Report,
}

impl CollectionView {
    /// The URL segment that names this view, exactly as [`Self`] serializes.
    #[must_use]
    pub const fn segment(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Kanban => "kanban",
            Self::Calendar => "calendar",
            Self::Tree => "tree",
            Self::Gantt => "gantt",
            Self::Report => "report",
        }
    }

    /// Every collection view, in the Desk's reading order.
    pub const ALL: [Self; 6] = [
        Self::List,
        Self::Kanban,
        Self::Calendar,
        Self::Tree,
        Self::Gantt,
        Self::Report,
    ];

    /// Parse a route segment. Only the lowercase spelling is accepted: the
    /// value is a URL keyword, not user text.
    #[must_use]
    pub fn from_segment(segment: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.segment() == segment)
    }
}

/// Which view the Desk opens for a doctype's documents.
///
/// A **declaration, not a route**: the URL stays `/desk/:module/:doctype/:name`
/// whatever this says, so a bookmark keeps resolving and the engine keeps
/// serving the same document while the Desk only changes how it is presented.
/// The list at `/desk/:module/:doctype` is unaffected — a dashboard doctype is
/// still listed, and opening one of its documents shows the dashboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocView {
    /// The built-in metadata form (the default).
    #[default]
    Form,
    /// A dashboard, rendered by the Desk's dashboard engine — the same
    /// `DashboardGrid` a plugin's route or the home page mounts.
    Dashboard,
    /// An app's custom element (`form.view` from the UI-extension feed).
    ///
    /// A **declaration that refuses the built-in form**: the feed already
    /// mounts an app's element whenever it declares one for the doctype, so
    /// what this adds is the other half — when no app declares one, the Desk
    /// says so instead of quietly drawing the form the declaration rejected.
    Custom,
}

/// Which dashboard a [`DocView::Dashboard`] document opens, as the Desk
/// resolves it once it has the document in hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardBinding {
    /// Open this dashboard id directly. `native` distinguishes the two ways an
    /// id can be missing from the UI feed: a document name (the `Dashboard`
    /// doctype, or a wrapper's link target) still opens — its widgets come
    /// from `GET /api/dashboard-layout/db/{id}` — while a declared id that the
    /// feed does not carry is a broken declaration.
    Direct { id: String, native: bool },
    /// Read `field` on the document first: a doctype that wraps a dashboard
    /// through a `Link` opens whatever that field points at.
    Link { field: String },
    /// Open the dashboard this very document carries: `format` names the
    /// doctype's own fields holding the widgets, the layout and the label, so
    /// there is no second record — the document *is* the dashboard, and the
    /// layout endpoint reads and writes the same fields
    /// (`GET|PUT /api/dashboard-layout/doc/{doctype}/{name}`).
    Document { format: DocumentFormat },
}

/// Which fields of *this* doctype carry the dashboard format, when the
/// document itself is the dashboard (the `document` binding of
/// [`DashboardView`]).
///
/// Every name is optional: an absent (or empty) one means the canonical
/// field, so [`DashboardView::document()`] declares "my documents are
/// dashboards" with no boilerplate, and each field can still be renamed
/// independently — `layout` in `panel_layout` while `widgets` keeps reading
/// `widgets`. Fields beyond this set are the doctype's own business: the
/// binding reads what it names and touches nothing else.
///
/// `route` is deliberately **not** part of the binding: the format's minimum
/// is widgets + layout + label, and a document has no route of its own to
/// publish — its URL is always `/desk/:module/:doctype/:name`. (The internal
/// `Dashboard` doctype keeps its `route`, which only feeds standalone
/// registrations in the UI feed.)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFormat {
    /// Field carrying the widget list. Canonical: `widgets`.
    #[serde(default)]
    pub widgets: Option<String>,
    /// Field carrying the grid layout strategy. Canonical: `layout`.
    #[serde(default)]
    pub layout: Option<String>,
    /// Field carrying the dashboard label. Canonical: `label`.
    #[serde(default)]
    pub label: Option<String>,
    /// Field carrying the shared board configuration (`DashboardConfig` JSON:
    /// the style/geometry override). Canonical: `config`. Optional and read
    /// only by the layout endpoint — a document that never declares the field
    /// simply has no override, so the board falls back to the Desk's global
    /// default.
    #[serde(default)]
    pub config: Option<String>,
}

impl DocumentFormat {
    /// `name` when it holds one, else the canonical field it stands for —
    /// one rule for "unset" and "declared empty", so a guest that maps an
    /// empty string cannot end up reading a field named "".
    fn effective<'a>(name: Option<&'a str>, canonical: &'static str) -> &'a str {
        name.map_or(
            canonical,
            |name| {
                if name.is_empty() { canonical } else { name }
            },
        )
    }

    /// The field carrying the widget list (`widgets` when unbound).
    #[must_use]
    pub fn widgets(&self) -> &str {
        Self::effective(self.widgets.as_deref(), "widgets")
    }

    /// The field carrying the grid layout strategy (`layout` when unbound).
    #[must_use]
    pub fn layout(&self) -> &str {
        Self::effective(self.layout.as_deref(), "layout")
    }

    /// The field carrying the dashboard label (`label` when unbound).
    #[must_use]
    pub fn label(&self) -> &str {
        Self::effective(self.label.as_deref(), "label")
    }

    /// The field carrying the shared board configuration (`config` when
    /// unbound).
    #[must_use]
    pub fn config(&self) -> &str {
        Self::effective(self.config.as_deref(), "config")
    }

    /// The binding with the widget list in a custom field.
    #[must_use]
    pub fn with_widgets(mut self, field: &str) -> Self {
        self.widgets = Some(field.to_string());
        self
    }

    /// The binding with the layout strategy in a custom field.
    #[must_use]
    pub fn with_layout(mut self, field: &str) -> Self {
        self.layout = Some(field.to_string());
        self
    }

    /// The binding with the label in a custom field.
    #[must_use]
    pub fn with_label(mut self, field: &str) -> Self {
        self.label = Some(field.to_string());
        self
    }

    /// The binding with the board configuration in a custom field.
    #[must_use]
    pub fn with_config(mut self, field: &str) -> Self {
        self.config = Some(field.to_string());
        self
    }

    /// The widget instances `doc`'s bound [`Self::widgets`] field carries —
    /// the one reader both the Desk's seed and the layout endpoint use.
    #[must_use]
    pub fn widgets_in(&self, doc: &serde_json::Value) -> Vec<crate::contract::ui::WidgetInstance> {
        crate::contract::ui::widget_instances(doc.get(self.widgets()))
    }

    /// The board configuration `doc`'s bound [`Self::config`] field carries,
    /// or `None` when the document never declared one (or the field holds
    /// anything but a `DashboardConfig` object) — the layout endpoint then
    /// answers with the Desk's global default alone.
    #[must_use]
    pub fn config_in(
        &self,
        doc: &serde_json::Value,
    ) -> Option<crate::contract::style::DashboardConfig> {
        let raw = doc.get(self.config())?;
        // The field may arrive as the JSON value the engine stored (a `Code`
        // field holds a string, a `JSON` field an object); accept both so a
        // hand-authored document is read the same way either declaration
        // stores it.
        raw.as_str().map_or_else(
            || serde_json::from_value(raw.clone()).ok(),
            |text| serde_json::from_str(text).ok(),
        )
    }
}

/// Which dashboard a [`DocView::Dashboard`] doctype's documents open.
///
/// The four bindings, all of them declared alongside [`DocView::Dashboard`]:
///
/// 1. **native** — nothing set: the document *is* a dashboard record (the
///    `Dashboard` doctype), so its own name is the dashboard id and the
///    Desk renders it directly. The native `Dashboard` doctype declares
///    this, which is what makes opening any of its records a dashboard.
/// 2. **document** — `document` names the doctype's *own* fields that carry
///    the dashboard format (widgets, layout, label), so each document **is**
///    its own dashboard: one row, one panel, no second record. The bound
///    fields keep their canonical names unless the declaration renames them
///    ([`DocumentFormat`]); extra fields on the doctype are untouched.
/// 3. **fixed** — `dashboard` names one dashboard every document of the
///    doctype opens, resolved against the UI feed.
/// 4. **wrapper** — `link_field` names a field (normally a `Link`) whose value
///    is the dashboard document, so a doctype that wraps another through a
///    link opens the linked one.
///
/// Specificity decides a declaration that carries more than one: a
/// `link_field` wins over everything (the per-document answer is the one the
/// user chose when they filled the field in), and `document` wins over
/// `dashboard` (a document declared to *be* a dashboard is one, whatever a
/// fixed id would have said).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardView {
    /// Field on the doctype whose value names the dashboard document.
    #[serde(default)]
    pub link_field: Option<String>,
    /// A fixed dashboard id, shown for every document of this doctype.
    #[serde(default)]
    pub dashboard: Option<String>,
    /// The document-format binding: the doctype's own fields carry the
    /// dashboard, under the names this value names (canonical when unbound).
    #[serde(default)]
    pub document: Option<DocumentFormat>,
}

impl DashboardView {
    /// The wrapper binding: open the dashboard the document's `field` links to.
    #[must_use]
    pub fn link_field(field: &str) -> Self {
        Self {
            link_field: Some(field.to_string()),
            ..Self::default()
        }
    }

    /// The fixed binding: every document of the doctype opens `dashboard`.
    #[must_use]
    pub fn dashboard(dashboard: &str) -> Self {
        Self {
            dashboard: Some(dashboard.to_string()),
            ..Self::default()
        }
    }

    /// The document binding with the canonical field names: the document is
    /// its own dashboard, reading `widgets`, `layout` and `label`.
    #[must_use]
    pub fn document() -> Self {
        Self {
            document: Some(DocumentFormat::default()),
            ..Self::default()
        }
    }

    /// The document binding with per-field names (`fields` renames only the
    /// fields it sets — see [`DocumentFormat`]).
    #[must_use]
    pub fn document_fields(fields: DocumentFormat) -> Self {
        Self {
            document: Some(fields),
            ..Self::default()
        }
    }

    /// The binding for the document named `docname` — everything except the
    /// wrapper's field *value*, which only the document itself can supply.
    #[must_use]
    pub fn binding_for(&self, docname: &str) -> DashboardBinding {
        match (&self.link_field, &self.document, &self.dashboard) {
            // The wrapper: the target is the field's value, not the name.
            // It wins over every other reading — the user chose this field.
            (Some(field), _, _) => DashboardBinding::Link {
                field: field.clone(),
            },
            // The document binding: this document *is* the dashboard.
            (None, Some(format), _) => DashboardBinding::Document {
                format: format.clone(),
            },
            // Fixed: the declaration names the dashboard.
            (None, None, Some(id)) => DashboardBinding::Direct {
                id: id.clone(),
                native: false,
            },
            // Native: the document *is* the dashboard record, so its name is
            // its own dashboard id.
            (None, None, None) => DashboardBinding::Direct {
                id: docname.to_string(),
                native: true,
            },
        }
    }
}
