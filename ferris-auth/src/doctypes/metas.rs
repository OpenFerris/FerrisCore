//! Registration order of every DocType this module declares.
//!
//! The order is load-bearing: it is what the registry and the Users workspace
//! see first, so the sidebar lists direct DocTypes, then profiles, permissions,
//! logs and reports.

use ferris_schema::DoctypeMeta;

use super::{
    access_log_meta, activity_log_meta, api_key_meta, docperm_meta, has_role_meta,
    module_profile_meta, role_meta, role_profile_meta, session_meta, user_group_meta,
    user_invitation_meta, user_meta, user_permission_meta, user_type_meta,
};

/// Every DocType this crate declares, in registration order.
#[must_use]
pub fn doctype_metas() -> Vec<DoctypeMeta> {
    vec![
        // Direct
        user_meta(),
        role_meta(),
        has_role_meta(),
        docperm_meta(),
        session_meta(),
        api_key_meta(),
        user_group_meta(),
        user_invitation_meta(),
        // Profiles
        role_profile_meta(),
        module_profile_meta(),
        user_type_meta(),
        // Permissions
        user_permission_meta(),
        // Logs
        activity_log_meta(),
        access_log_meta(),
        // Reports
    ]
}

/// Register this crate's DocTypes in the global schema registry.
///
/// Idempotent: re-registering overwrites the same keys, so it is safe on every
/// boot and after a hot reload. Tables themselves are created lazily by the
/// document engine the first time a DocType is touched.
pub fn register_doctypes() {
    // The document engine hashes `Password` fields through the hook in
    // `ferris_core::password`. Install Argon2 as the process-wide hasher here,
    // so any DocType with a `Password` field stores a hash and never the
    // plaintext. Idempotent, like the registration below.
    ferris_core::password::set_password_hasher(crate::password::hash_password_default);
    for meta in doctype_metas() {
        ferris_schema::register_doctype(meta);
    }
}
