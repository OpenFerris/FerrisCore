//! OpenUI extensions: `GET /api/ui-extensions`.
//!
//! The Desk mounts plugin components into host-defined slots; these are the
//! wire shapes for those declarations. The *registry* (which app declared
//! what) lives in `ferris_core::ui_extensions`; this is only the wire shape,
//! so the browser can deserialize the feed without the core crate.
//!
//! Deliberately both `Serialize` and `Deserialize` even though the Desk only
//! reads: that makes the types round-trippable so the server can serialize the
//! registry's `AppUi` through them and both sides are checked against one
//! definition.
//!
//! The shapes are grouped by concern: [`feed`] (the envelope), [`widget`] (the
//! catalog), [`dashboard`] (declared boards and saved layouts) and [`website`].

mod dashboard;
mod feed;
mod website;
mod widget;

pub use dashboard::{Dashboard, DashboardLayout, DashboardLayoutPayload};
pub use feed::{AppUi, NATIVE_DASHBOARD_OWNER, UiExtension, UiFeed, revision_of};
pub use website::{Website, WebsiteRoute};
pub use widget::{GridSize, WidgetDef, WidgetInstance, WidgetParam, widget_instances};

#[cfg(test)]
mod tests;
