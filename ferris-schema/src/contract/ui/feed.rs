//! The `GET /api/ui-extensions` envelope and one app's declared UI set.

use serde::{Deserialize, Serialize};

use super::dashboard::Dashboard;
use super::website::Website;
use super::widget::WidgetDef;

/// The `GET /api/ui-extensions` envelope.
///
/// Every app's UI set plus a `revision` of the whole payload so the Desk can
/// skip refetching when nothing changed (and notice a hot-reloaded plugin when
/// something did).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UiFeed {
    /// Stable fingerprint (FNV-1a hex) of `apps` — changes iff the payload
    /// changes.
    pub revision: String,
    /// Every registered UI set, in registration order.
    pub apps: Vec<AppUi>,
}

/// One declared UI extension: a component the app mounts into a host slot.
///
/// `component` becomes the custom element the Desk mounts
/// (`"invoice-actions"` → `<ferris-invoice-actions>`); `"iframe"` opts into
/// the sandboxed iframe escape hatch. `doctype` scopes the extension to one
/// document type for the doctype-aware slots (`form.*`, `list.*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiExtension {
    /// The host slot this extension binds to, e.g. `form.toolbar.right`,
    /// `list.before_table`, `shell.sidebar.bottom`.
    pub slot: String,
    /// DocType this extension applies to; omitted for shell-wide slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doctype: Option<String>,
    /// Custom element name the app defines.
    pub component: String,
    /// Human label for tooltips, menus and empty placeholder text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// (name, value) pairs the host hands to the mounted element as its
    /// initial props — the slot-side twin of widget `params`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
}

/// The owner tag the native dashboards — the `Dashboard` documents — ride
/// under in the UI feed and in `GET /api/dashboard-layout/{owner}/{dashboard}`.
///
/// [`AppUi::owner`] names the plugin that declared a set, and these documents
/// have no plugin behind them, so the server publishes them under one
/// synthetic app with this tag. The Desk needs it too: a doctype that declares
/// [`DocView::Dashboard`] opens its document by *name*, which is enough to
/// reach the dashboard even when the feed does not carry it (its route was
/// unusable, or the feed has not loaded yet) — the widgets then come from the
/// layout endpoint.
///
/// [`DocView::Dashboard`]: crate::contract::DocView::Dashboard
pub const NATIVE_DASHBOARD_OWNER: &str = "db";

/// One app's registered UI extension set, as `GET /api/ui-extensions` hands it
/// to the Desk. `owner` is the plugin id that declared the set — and, for the
/// host's own catalog, the literal `"host"`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppUi {
    /// Owner tag: the plugin/app id that declared this set.
    pub owner: String,
    #[serde(default)]
    pub extensions: Vec<UiExtension>,
    #[serde(default)]
    pub dashboards: Vec<Dashboard>,
    /// Widget definitions this app publishes to the global catalog; other
    /// apps' dashboards (and the host's) instance them by `owner.name`.
    #[serde(default)]
    pub widgets: Vec<WidgetDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<Website>,
    /// UI asset paths (scripts/stylesheets) the Desk loads to define the
    /// declared custom elements, served at `/plugins/{owner}/asset/{path}`.
    #[serde(default)]
    pub assets: Vec<String>,
}

/// FNV-1a (64-bit) over `bytes`, as lowercase hex — a dependency-free
/// fingerprint the Desk compares to decide whether the feed changed.
#[must_use]
pub fn revision_of(bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}
