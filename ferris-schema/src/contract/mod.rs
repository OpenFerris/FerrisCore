//! Canonical wire contract between the Ferris engine and any Desk client.
//!
//! Every payload that crosses the process boundary — HTTP during hydration, the
//! in-process bridge during SSR — is one of these types. They are defined here,
//! once, so `ferris-api` (which serializes them) and `ferris-web` (which
//! deserializes them) cannot drift: a field rename becomes a compile error on
//! both sides instead of a runtime `null`.
//!
//! Two rules keep this module the only place a wire shape is defined:
//!
//! 1. **serde only.** No dependency on any other `ferris-*` crate, no database,
//!    no HTTP. `ferris-schema` is compiled with `default-features = false` for
//!    the browser and the wasm build resolves this module alone; pulling
//!    `sqlx`/`ferris-core` in here would put the whole backend back in the
//!    hydrate graph.
//! 2. **Pure data.** No rendering, no transport, no I/O. Behaviour lives with
//!    the crate that owns the effect; a `#[serde(default)]` here is a decision
//!    about what an absent field means, and it has to be the same decision the
//!    server made when it omitted it.
//!
//! The types mirror the engine's own structs rather than being those structs:
//! [`meta::DynMeta`] carries only the subset of `registry::DoctypeMeta` the
//! Desk renders, so adding a server-side metadata field does not widen the
//! public wire contract.

pub mod activity;
pub mod collaboration;
pub mod customization;
pub mod desk;
pub mod engagement;
pub mod identity;
pub mod layout;
pub mod list;
pub mod meta;
pub mod routes;
pub mod site;
pub mod style;
pub mod theme;
pub mod ui;

pub use activity::{ActivityEntry, ActivityKind};
pub use collaboration::{Assignment, Attachment, Share};
pub use customization::{CustomFieldSpec, Customization, ExportScope, PropertySetterSpec};
pub use desk::{
    DeskEvent, DeskIcon, DeskIconPlacement, DeskLayout, DeskModule, DeskSearchHit,
    DeskSidebarEntry, EventFeed, Notification, NotificationFeed, NotificationSettings,
};
pub use engagement::{EngagementPage, LikeState};
pub use identity::WebUser;
pub use layout::{CELL_HEIGHT_PX, COLUMN, DEFAULT_GAP_PX, DEFAULT_ROWS, Placement, Sizing};
pub use list::{
    ApiResponse, FILTER_OPERATORS, Filter, FilterOp, ListRequest, Neighbors, or_groups,
};
pub use meta::{
    ConnectionDocument, ConnectionGroup, Connections, CustomPermissionGrant, DashboardBinding,
    DashboardView, DocView, DocumentFormat, DynCustomPermission, DynDocAction, DynDocLink,
    DynField, DynIndex, DynMeta, DynPermissionRule, NamingType, PermissionMatrix,
};
pub use routes::{DeskRoute, slugify};
pub use site::SiteSettings;
pub use style::{
    DashboardConfig, DashboardSettings, DashboardSettingsPayload, DashboardStyle, MAX_BORDER_WIDTH,
    MAX_GAP, MAX_PADDING, MAX_RADIUS, StyleSpec,
};
pub use theme::{ThemeSpec, builtin_themes, dark_theme, light_theme};
pub use ui::{
    AppUi, Dashboard, DashboardLayout, DashboardLayoutPayload, GridSize, NATIVE_DASHBOARD_OWNER,
    UiExtension, UiFeed, Website, WebsiteRoute, WidgetDef, WidgetInstance, WidgetParam,
    revision_of,
};

/// Humanize an underscore/kebab identifier ("sales_order" -> "Sales Order").
///
/// Both `ferris-api` (module labels) and the Desk route slug use this wording,
/// so it is shared rather than duplicated per side of the wire.
pub fn pretty_identifier(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut cap = true;
    for ch in raw.chars() {
        if ch == '_' || ch == '-' {
            out.push(' ');
            cap = true;
        } else if cap {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_humanized() {
        assert_eq!(pretty_identifier("sales_order"), "Sales Order");
        assert_eq!(pretty_identifier("Accounting"), "Accounting");
        assert_eq!(pretty_identifier("acc_recv"), "Acc Recv");
    }
}
