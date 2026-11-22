//! The two permission questions the layout endpoints ask.
//!
//! Both are the `Dashboard Layout` doctype's role policy evaluated for the
//! caller: the row is system-owned and shared, so the gate is role-shaped, not
//! ownership-shaped. `Create` and `Delete` are kept as separate questions
//! because the engine's owner shortcut never grants `Delete`, and reusing the
//! edit check here would let `Create` imply `Delete`.

use ferris_auth::Permission;
use ferris_auth::permissions::{Resource, check_permission};

use crate::core_doctypes::DASHBOARD_LAYOUT;

/// Whether `user` may edit this dashboard's shared row: the `Create` grant on
/// the `Dashboard Layout` doctype, evaluated for the caller. The row is
/// system-owned and shared, so the check is role-shaped, not ownership-shaped —
/// and it is *the* answer to "may this user edit dashboards" the `editable`
/// flag repeats.
pub(super) fn editable_for(user: &ferris_auth::User, name: &str) -> bool {
    let resource = Resource::new(DASHBOARD_LAYOUT, name);
    check_permission(user, &resource, Permission::Create)
}

/// Whether `user` may delete the saved row for this dashboard.
///
/// `Delete` is its own right (the engine's owner shortcut never grants it, and
/// there is no owner here to shortcut), and the delete runs through the service
/// context, so this explicit check is the only gate that runs. Reusing
/// [`editable_for`] here would let `Create` imply `Delete`.
pub(super) fn deletable_for(user: &ferris_auth::User, name: &str) -> bool {
    let resource = Resource::new(DASHBOARD_LAYOUT, name);
    check_permission(user, &resource, Permission::Delete)
}
