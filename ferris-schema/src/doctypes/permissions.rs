//! Shared role-permission helpers for the declared DocTypes.
//!
//! The two recurring grants are declared once in
//! [`crate::builders::grants`]; this module re-exports them so every DocType
//! family keeps importing from its sibling, and adds the read-only grant only
//! the form metadata needs.

pub(super) use crate::builders::{system_manager, system_manager_single};

use crate::builders::RolePermission;

/// Read-only for every signed-in user.
///
/// Used by metadata a form needs to render for everyone (`Form Tour`): the
/// tour is copy, not site configuration, and hiding it behind System Manager
/// would make the feature unusable for the users it is written for.
#[must_use]
pub(super) fn all_users_read() -> RolePermission {
    RolePermission::new(ferris_core::permissions::ALL_USER_ROLE).read()
}
