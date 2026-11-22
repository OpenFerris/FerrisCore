//! The permission grants the declared `DocType` catalogs repeat.
//!
//! `System Manager` full CRUD and the read/write policy of a Single recur across
//! the Build, Users and Data sections, in `ferris-schema`, `ferris-auth` and
//! `ferris-api`. They are defined here once so a second declaration in another
//! crate cannot drift into a different policy.

use crate::builders::RolePermission;

/// Full CRUD for the administrator role.
#[must_use]
pub fn system_manager() -> RolePermission {
    RolePermission::new("System Manager")
        .read()
        .write()
        .create()
        .delete()
}

/// Read + write with neither create nor delete: the policy of a Single.
///
/// A Single's one row is created lazily by `get_single`, which deliberately
/// skips `Permission::Create` (§13), so a `create` grant here would be a right
/// no code path ever checks.
#[must_use]
pub fn system_manager_single() -> RolePermission {
    RolePermission::new("System Manager").read().write()
}
