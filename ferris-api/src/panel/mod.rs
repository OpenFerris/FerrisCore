//! The desk shell's per-user server surface.
//!
//! Serves the notification feed (`GET /api/notifications`), mark-all read, the
//! events feed (`GET /api/events`), the per-user notification settings
//! (`GET|PUT /api/notification-settings`), the `push_notification` producer the
//! session layer calls at login, and the per-user desk grid
//! (`GET|PUT /api/desk-layout`). The backing DocTypes are framework-owned (see
//! [`crate::core_doctypes`]).
//!
//! These read and write the documents directly instead of going through the
//! generic `/api/resource/{doctype}` routes because the engine does not scope
//! a list by owner: an unscoped `DocStore::list` would return every user's
//! notifications. Every read here filters on the user resolved from the
//! session, and every write derives its target from that same id, so a
//! request body can never widen the scope.

mod context;
mod events;
mod layout;
mod notifications;
mod settings;
#[cfg(test)]
mod tests;

pub(crate) use context::panel_context;
pub use events::events;
pub use layout::{get_desk_layout, put_desk_layout, stored_layout};
pub use notifications::{
    dismiss_notification, mark_notification_read, mark_notifications_read, notifications,
    notify_on_login, push_notification,
};
pub use settings::{get_notification_settings, put_notification_settings};
