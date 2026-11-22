//! Shared module constant and the permission every declared DocType grants.

/// Desk module/section every DocType here belongs to.
pub const MODULE: &str = "Users";

/// Full CRUD for the administrator role, from the one declaration shared by the
/// DocType catalogs (`ferris_schema::builders::grants`).
pub(super) use ferris_schema::builders::system_manager;
