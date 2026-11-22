//! Dashboards an app declares and a user's saved layout of one.

use serde::{Deserialize, Serialize};

use super::widget::WidgetInstance;
use crate::contract::style::{DashboardConfig, DashboardStyle};

/// One dashboard an app declares: a layout of widget instances. Required to
/// carry a route — apps expose single widgets by publishing their
/// [`WidgetDef`], not by declaring route-less dashboards.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dashboard {
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// Desk route the dashboard is mounted at (`/sales`). Required: the
    /// server validates it against the desk's reserved prefixes.
    pub route: String,
    /// The dashboard shown at `/` when true (first registered wins).
    #[serde(default)]
    pub default: bool,
    /// The dashboard's default appearance and board geometry, as declared by
    /// the app (or the `Dashboard` document). A user's per-dashboard override
    /// layers over it (see [`DashboardConfig`]); absent means the Desk's global
    /// default alone. Optional so an app that declares no style is unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<DashboardStyle>,
    /// Widget instances, in layout order.
    #[serde(default)]
    pub widgets: Vec<WidgetInstance>,
}

/// A user's saved arrangement of one app dashboard: `GET|PUT|DELETE
/// /api/dashboard-layout/{owner}/{dashboard}`.
///
/// `customized` tells the Desk whether the user has a saved override (their
/// `widgets` are authoritative) or whether the app's declared seed is still
/// showing (the field then carries that seed, so the client renders the same
/// list either way). `editable` is the server's answer to "may this user save
/// an override": the `Dashboard Layout` doctype's `create` gate, evaluated for
/// the authenticated caller — the client shows the editor only when it is true.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardLayout {
    /// Whether the authenticated user may save an override for this dashboard.
    pub editable: bool,
    /// Whether a saved override exists (vs the app's declared seed).
    pub customized: bool,
    /// The widget instances to render, in layout order: the user's override
    /// when `customized`, else the dashboard's declared seed.
    pub widgets: Vec<WidgetInstance>,
    /// The dashboard's saved configuration (style and board geometry), shared
    /// by every user. Absent means "the dashboard declares none"; the effective
    /// style is this layered over the Desk's global default.
    #[serde(default, skip_serializing_if = "DashboardConfig::is_empty")]
    pub config: DashboardConfig,
}

/// The body of `PUT /api/dashboard-layout/{owner}/{dashboard}`: the whole
/// layout and the configuration, saved together.
///
/// One payload rather than two endpoints because a save is always the whole
/// board — the widget list *and* its style are the state of the same document —
/// and splitting them would let the two halves drift. The old bare-array body
/// is retired with this type: the server is the only writer and it is versioned
/// with the Desk.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardLayoutPayload {
    /// The widget instances to store, in layout order.
    #[serde(default)]
    pub widgets: Vec<WidgetInstance>,
    /// The dashboard's configuration override. Absent (or empty) stores none.
    #[serde(default, skip_serializing_if = "DashboardConfig::is_empty")]
    pub config: DashboardConfig,
}
