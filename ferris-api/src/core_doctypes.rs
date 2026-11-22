//! Compatibility alias for [`crate::doctypes`].
//!
//! The core DocTypes (Notification, Event, Notification Settings, Desk Layout)
//! moved to [`crate::doctypes`] as part of the per-crate DocType declaration
//! pattern. This module re-exports the old names so
//! existing callers — `register_core_doctypes()`, the constants, the `*_meta`
//! builders — keep working unchanged.
//!
//! New code should use [`crate::doctypes`] directly.
#![allow(clippy::doc_markdown)]

pub use crate::doctypes::register_doctypes;
pub use crate::doctypes::register_doctypes as register_core_doctypes;
pub use crate::doctypes::{
    DASHBOARD_LAYOUT, DESK_LAYOUT, EVENT, NOTIFICATION, NOTIFICATION_SETTINGS,
    dashboard_layout_meta, desk_layout_meta, doctype_metas, event_meta, notification_meta,
    notification_settings_meta,
};
