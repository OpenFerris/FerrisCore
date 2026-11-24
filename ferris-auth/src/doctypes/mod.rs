//! DocTypes declared by `ferris-auth` (the Users section).
//!
//! These are the identity, access-control and audit DocTypes the Users
//! workspace links to. The registry lives in
//! `ferris-schema`; this module only declares the metas and registers them
//! (§14.1).
//!
//! Decision recorded in the plan (§16.2, D3 "completo", 2026-10-10): `User` and
//! `Role` are declared here as metadata DocTypes. The document engine is the
//! source of truth for them: `UserStore`, `DocPermStore`, `ApiKeyManager` and
//! `SessionManager` all read and write through it (`store_support.rs`), so the
//! declarations are the live storage, not a target shape. Roles are still
//! embedded in the `User.roles` JSON field — the `Has Role` child table is
//! declared, but the login path reads roles from that field. The rest of the
//! section has no live equivalent and is pure navigation metadata.
//!
//! Field sets are deliberately Frappe-shaped but trimmed to what a declaration
//! needs; a DocType grows fields when something reads them, not before.
//!
//! One file per DocType keeps each declaration reviewable on its own; the
//! registration order lives in [`metas`].
#![allow(clippy::doc_markdown)]

mod access_log;
mod activity_log;
mod api_key;
mod common;
mod docperm;
mod has_role;
mod metas;
mod module_profile;
mod role;
mod role_profile;
mod session;
mod user;
mod user_group;
mod user_invitation;
mod user_permission;
mod user_type;

#[cfg(test)]
mod tests;

pub use access_log::access_log_meta;
pub use activity_log::activity_log_meta;
pub use api_key::api_key_meta;
pub use common::MODULE;
pub use docperm::docperm_meta;
pub use has_role::has_role_meta;
pub use metas::{doctype_metas, register_doctypes};
pub use module_profile::module_profile_meta;
pub use role::role_meta;
pub use role_profile::role_profile_meta;
pub use session::session_meta;
pub use user::user_meta;
pub use user_group::user_group_meta;
pub use user_invitation::user_invitation_meta;
pub use user_permission::user_permission_meta;
pub use user_type::user_type_meta;
