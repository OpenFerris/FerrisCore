//! DocTypes declared by `ferris-api` (the Data and Desk sections).
//!
//! Two groups live here:
//!
//! - The **framework** DocTypes every install has — the notification feed, the
//!   events feed, per-user notification settings and the per-user desk grid.
//!   They are `hidden` so they never show up as workspace links; the desk
//!   reaches them through the notification panel and the desk grid, not the
//!   sidebar.
//! - The **Data** section — import/export, document naming, import logs,
//!   prepared reports and the Desk-feed helpers (ToDo, Tag, Bulk Update,
//!   Notification Log).
//!
//! `File` is *not* declared here even though the Data workspace links to it:
//! the single declaration of that DocType belongs to `ferris-files` (plan
//! §16.6). A link is not a declaration.
//!
//! This is the canonical module the host's `register_doctype_catalog()` calls
//!. [`crate::core_doctypes`] is kept as a
//! compatibility alias for the older `register_core_doctypes` naming; new code
//! should use [`register_doctypes`] here.
//!
//! The declarations themselves are split by family — [`system`] for the hidden
//! framework feeds, [`dashboards`] for the shared dashboard rows, [`data`] for
//! the import/naming tooling, [`desk`] for the Desk-feed helpers — with the
//! registration order held in [`registry`]. Each builder keeps its original
//! public path through the re-exports below.
#![allow(clippy::doc_markdown)]

mod constants;
mod dashboards;
mod data;
mod desk;
mod registry;
mod support;
mod system;
#[cfg(test)]
mod tests;

pub use constants::{
    DASHBOARD_LAYOUT, DASHBOARD_SETTINGS, DATA_MODULE, DESIRED_STATE, DESK_LAYOUT, DESK_MODULE,
    EVENT, NOTIFICATION, NOTIFICATION_SETTINGS, TAG,
};
pub use dashboards::{dashboard_layout_meta, dashboard_settings_meta};
pub use data::{
    document_naming_rule_condition_meta, document_naming_rule_meta, document_naming_settings_meta,
};
pub use desk::{notification_log_meta, tag_meta, todo_meta};
pub use registry::{data_doctype_metas, doctype_metas, framework_doctype_metas, register_doctypes};
pub(crate) use support::system_manager;
pub use system::{
    desired_state_meta, desk_layout_meta, event_meta, notification_meta, notification_settings_meta,
};
