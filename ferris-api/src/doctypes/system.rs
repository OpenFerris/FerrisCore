//! The framework DocTypes every install has: the notification and events
//! feeds, the per-user notification settings and desk grid, and the
//! desired-state document edge agents pull.
//!
//! All of them are `hidden` — they are reached through the desk panels and
//! their own machine APIs, never as workspace links — which is why they live
//! apart from the visible Data and Desk-feed families.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::constants::{
    DESIRED_STATE, DESK_LAYOUT, DESK_MODULE, EVENT, NOTIFICATION, NOTIFICATION_SETTINGS,
};
use super::support::{all_users, field, required, system_manager, with_default, with_label};

/// A notification addressed to one user.
///
/// `user` is the recipient; the desk filters the feed on it, so it is the
/// field that makes a notification personal. `owner` is stamped to the same
/// value on create so the recipient can mark their own notifications read
/// through the owner rule.
#[must_use]
pub fn notification_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: NOTIFICATION.to_string(),
        module: "Desk".to_string(),
        description: Some("A notification addressed to a single user.".to_string()),
        // Empty series: names are opaque UUIDs, so no naming-series state is
        // needed for a feed nobody addresses by name.
        naming_series: String::new(),
        fields: vec![
            FieldMeta::section_break("section_details").label("Details"),
            required(with_label(field("user", "Data"), "User")),
            with_label(
                with_default(field("notification_type", "Data"), "General"),
                "Notification Type",
            ),
            required(with_label(field("title", "Data"), "Title")),
            with_label(field("description", "Text"), "Description"),
            with_label(field("route", "Data"), "Route"),
            with_label(with_default(field("is_read", "Check"), "0"), "Read"),
        ],
        // A feed, not a document: keep it out of the workspace, search and
        // exports. It is reached only through the panel's own endpoints.
        hidden: true,
        allow_delete: true,
        // No `permissions` on purpose: the recipient owns their notifications, so
        // the owner rule grants them and a role grant would widen the feed to
        // whoever else could be given a role.
        permissions: Vec::new(),
        sort_field: Some("creation".to_string()),
        sort_order: Some("desc".to_string()),
        ..Default::default()
    }
}

/// A dated entry on the desk's "what's happening" feed.
///
/// `user` scopes an event to a person; an event with no `user` is shared and
/// shown to everyone.
#[must_use]
pub fn event_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: EVENT.to_string(),
        module: "Desk".to_string(),
        description: Some("A dated entry on the desk events feed.".to_string()),
        naming_series: String::new(),
        fields: vec![
            FieldMeta::section_break("section_details").label("Details"),
            required(with_label(field("title", "Data"), "Title")),
            required(with_label(field("starts_on", "Datetime"), "Starts On")),
            with_label(field("ends_on", "Datetime"), "Ends On"),
            with_label(field("location", "Data"), "Location"),
            with_label(field("description", "Text"), "Description"),
            with_label(field("user", "Data"), "User"),
        ],
        hidden: true,
        allow_delete: true,
        // A shared event — one with no `user` — belongs to nobody but its
        // creator, so the owner rule hides it from the rest of the install. The
        // panel shows shared events to everyone, so `All` needs read here.
        // Write and delete stay with the creator: the panel has no shared-event
        // edit path, and a grant nobody uses is a grant nobody audits.
        permissions: vec![all_users(true)],
        in_calendar: true,
        date_field: Some("starts_on".to_string()),
        calendar_end_field: Some("ends_on".to_string()),
        calendar_title_field: Some("title".to_string()),
        sort_field: Some("starts_on".to_string()),
        sort_order: Some("asc".to_string()),
        ..Default::default()
    }
}

/// One row per user; the document name is the user's id.
#[must_use]
pub fn notification_settings_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: NOTIFICATION_SETTINGS.to_string(),
        module: "Desk".to_string(),
        description: Some("Per-user notification preferences.".to_string()),
        naming_series: String::new(),
        fields: vec![
            required(with_label(field("user", "Data"), "User")),
            with_default(
                with_label(field("desktop", "Check"), "Desktop Notifications"),
                "1",
            ),
            with_default(
                with_label(field("notify_on_login", "Check"), "Notify On Login"),
                "1",
            ),
        ],
        hidden: true,
        allow_delete: true,
        // The document is named after the user id and created with that user as
        // `owner` (`panel.rs:537`), so the owner rule is the whole policy. See
        // `all_users`.
        permissions: Vec::new(),
        ..Default::default()
    }
}

/// The user's own arrangement of the desk grid; the document name is the user's
/// id, so there is exactly one row per person and the engine's owner rule
/// already scopes every read and write to them.
///
/// `layout` is the stored JSON array of `DeskIconPlacement`. It is a Code field
/// rather than child tables because an arrangement is one ordered list that is
/// always read and written whole: splitting it into rows would buy nothing over
/// a document edit and would make reordering a multi-document operation.
#[must_use]
pub fn desk_layout_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: DESK_LAYOUT.to_string(),
        module: "Desk".to_string(),
        description: Some(
            "The user's own arrangement of the desk grid: which workspace icons sit \
             at the root, which folders group them, and in what order."
                .to_string(),
        ),
        naming_series: String::new(),
        fields: vec![
            required(with_label(field("user", "Data"), "User")),
            with_label(field("layout", "Code"), "Layout"),
        ],
        hidden: true,
        allow_delete: true,
        // Same as the notification settings: one row per user, owned by them.
        // See `all_users`.
        permissions: Vec::new(),
        ..Default::default()
    }
}

/// Persistent desired state of one externally managed entity (R-FW-1b).
///
/// The document's **name is the entity id**, so the pull route
/// (`GET /api/desired-state/{entity}`) is a primary-key read rather than a
/// filtered scan. `payload` holds the JSON document an edge agent should
/// converge on, stored as text so any JSON value survives a round trip;
/// `version` is bumped on every publish so an agent can poll with a known
/// version and be told `304 Not Modified` when nothing changed.
///
/// Hidden like the other framework DocTypes: it is reached through its own
/// machine API, never as a workspace link. `All` gets **read** because the whole
/// point is a pull any signed-in agent may perform; writes stay with `System
/// Manager`. An operator who considers desired state sensitive can withhold the
/// read again through the policy document.
#[must_use]
pub fn desired_state_meta() -> DoctypeMeta {
    DoctypeMeta::new(DESIRED_STATE, DESK_MODULE)
        .description(
            "The desired configuration of one externally managed entity, as published \
             by the control plane and pulled by edge agents.",
        )
        .sort("modified", "desc")
        .allow_delete()
        .hidden()
        .field(FieldMeta::code("payload").label("Desired State"))
        .field(
            FieldMeta::int("version")
                .label("Version")
                .default_value("1")
                .in_list_view(),
        )
        // Read is for the agents that pull; the write half is for the control
        // plane, which reaches the store through the `/api/desired-state` route.
        .permission(all_users(true))
        .permission(system_manager())
}
