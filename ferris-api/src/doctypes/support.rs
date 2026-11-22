//! The permission and field builders every family module shares.
//!
//! The grants themselves have one declaration, in
//! `ferris_schema::builders::grants`; this module re-exports them so the family
//! modules keep importing from their sibling and two families cannot mint
//! "System Manager may do everything" rules that disagree.

use ferris_schema::{FieldMeta, PermissionRule};

/// Full CRUD for the administrator role.
///
/// Shared with [`crate::tags`] so the tag `DocType`s do not mint a second,
/// independently-editable copy of the same grant.
pub(crate) use ferris_schema::builders::system_manager;

/// Read + write with neither create nor delete: the policy of a Single.
pub(super) use ferris_schema::builders::system_manager_single;

/// A `DocPerm` row for "every signed-in user", Frappe's `ALL_USER_ROLE`.
///
/// `doc_type_policy` turns this into a `Principal::everyone()` statement, which
/// `check_principals` appends to every evaluation, so the grant does not depend
/// on the caller holding any role record.
///
/// Only the rights named are granted. The call sites below weigh each one:
/// - `Notification` gets **none**: the recipient owns their notifications, so a
///   blanket `All` read on `Notification` would publish every user's feed.
/// - `Event` grants **read** to `All`, because a shared event has no `user` and
///   no owner but its creator, and the events panel shows shared events to
///   everyone (`panel.rs:301`). Without this the shared half of the feed is
///   invisible to everybody except whoever created each row.
#[must_use]
pub(super) fn all_users(read: bool) -> PermissionRule {
    PermissionRule {
        role: ferris_core::permissions::ALL_USER_ROLE.to_string(),
        read,
        ..Default::default()
    }
}

/// A list-visible `Data`-shaped field.
#[must_use]
pub(super) fn field(fieldname: &str, fieldtype: &str) -> FieldMeta {
    FieldMeta {
        fieldname: fieldname.to_string(),
        fieldtype: fieldtype.to_string(),
        in_list_view: true,
        ..Default::default()
    }
}

#[must_use]
pub(super) const fn required(mut f: FieldMeta) -> FieldMeta {
    f.reqd = true;
    f
}

#[must_use]
pub(super) fn with_default(mut f: FieldMeta, default: &str) -> FieldMeta {
    f.default = Some(default.to_string());
    f
}

#[must_use]
pub(super) fn with_label(mut f: FieldMeta, label: &str) -> FieldMeta {
    f.label = Some(label.to_string());
    f
}
