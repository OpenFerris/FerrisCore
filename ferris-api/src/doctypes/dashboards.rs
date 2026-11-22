//! The shared dashboard DocTypes: one saved layout per dashboard and the
//! install-wide board style.
//!
//! Both are the storage side of the dashboard service
//! ([`crate::dashboard_layout`] and [`crate::dashboard_settings`]); keeping them
//! together makes the shared edit gate visible in one place.

use ferris_schema::{DoctypeMeta, FieldMeta};

use super::constants::{DASHBOARD_LAYOUT, DASHBOARD_SETTINGS, DESK_MODULE};
use super::support::{all_users, field, system_manager, with_label};

/// A saved arrangement of one app dashboard, shared by every user.
///
/// The document name is `"{owner}:{dashboard}"`, so there is exactly one row
/// per dashboard — the deliberate product decision (C4): editing a dashboard edits it for
/// everyone. The row is system-owned (read and written through the service
/// principal), so no user owns it and the second editor updates rather than
/// collides. `widgets` holds the full ordered JSON list of `WidgetInstance`,
/// always read and written whole for the same reason the desk's placements are.
///
/// The permission policy is the *edit gate*: a user edits a dashboard by
/// writing this shared row, and `Permission::Create` is deliberately not
/// covered by the owner shortcut (`engine.rs`) — and the row has no user owner
/// to shortcut in the first place — so the only way a save succeeds is through
/// this role rule. It defaults to the administrator role; granting any other
/// role `create` through the authz role-permissions UI opens
/// dashboard editing to it. `Delete` is gated the same way and by the same
/// policy: the REST `DELETE` checks `Permission::Delete` explicitly.
#[must_use]
pub fn dashboard_layout_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: DASHBOARD_LAYOUT.to_string(),
        module: "Desk".to_string(),
        description: Some(
            "The shared arrangement of one dashboard: which widgets, in what order, \
             with each one's grid span and parameters, plus the board's style \
             override. One row per dashboard, edited by whoever may edit the Desk."
                .to_string(),
        ),
        naming_series: String::new(),
        fields: vec![
            // The app that declared the dashboard. Named `app_owner`, not
            // `owner`: the engine reserves `owner` for the row's own owner (the
            // service principal, since the row is shared), so a client-supplied
            // `owner` is a 400.
            with_label(field("app_owner", "Data"), "App Owner"),
            with_label(field("dashboard", "Data"), "Dashboard"),
            with_label(field("widgets", "Code"), "Widgets"),
            // The board override (`DashboardConfig` JSON: background, gap,
            // padding, chrome flags). Absent or `{}` means the dashboard
            // declares no override and the Desk's global default alone applies.
            with_label(field("config", "Code"), "Config"),
        ],
        hidden: true,
        allow_delete: true,
        // The `create` grant is the "may this user edit dashboards" flag; the
        // shared row is read and written through the service principal, so only
        // this role rule gates it. See the module docs above.
        permissions: vec![system_manager().into()],
        ..Default::default()
    }
}

/// The Desk's global board style (the dashboard config, phase C8).
///
/// One row, `name = "global"`, shared by the whole install: it is the base
/// every board layers its declared style and saved override over, so editing it
/// repaints every dashboard. `config` holds the same `DashboardConfig` JSON a
/// per-dashboard `Dashboard Layout` row stores, which lets one precedence rule
/// read both.
///
/// Read is granted to every signed-in user — every board needs the value to
/// render — while writes are the `create` right on this doctype, defaulting to
/// System Manager. The endpoint gates `PUT` with exactly that right and reads
/// through the service principal, the same shape as [`dashboard_layout_meta`].
#[must_use]
pub fn dashboard_settings_meta() -> DoctypeMeta {
    DoctypeMeta::new(DASHBOARD_SETTINGS, DESK_MODULE)
        .description(
            "The Desk's global board style: the base every dashboard's declared \
             style and saved override layer over.",
        )
        .hidden()
        .field(FieldMeta::code("config").label("Config"))
        .permission(all_users(true))
        .permission(system_manager())
}
