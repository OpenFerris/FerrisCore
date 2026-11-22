//! Point of entry for authorization: build the process-wide permission engine
//! from what the schema registry actually declares, and install it.
//!
//! # Why this module exists
//!
//! `PermissionEngine` was fully built and fully unreachable. `set_global_engine`
//! — the only way to put a policy in front of a decision — had **zero callers**
//! outside its own test module, and `DoctypeMeta::permissions`, the field that
//! carries a DocType's declared rights, was empty on every registered DocType.
//! The engine therefore answered every request from a single hardcoded baseline
//! ("System Manager may do anything") plus an owner comparison in
//! `ferris_auth::permissions::check_permission`. The role model the framework
//! documents — roles, deny-wins, wildcards, conditions — existed and was never
//! consulted with a policy in it.
//!
//! # What it installs, and when
//!
//! Only DocTypes *already registered* are translated. That is a real ordering
//! constraint, not a detail: the `#[doctype]` macro registers lazily, from the
//! first call to its metadata accessor (`ferris-macro/src/doctype.rs:281`), so a
//! boot-time install sees nothing but whatever has been registered eagerly. The
//! core DocTypes are registered eagerly by
//! [`register_core_doctypes`](crate::core_doctypes::register_core_doctypes),
//! and a WASM plugin's DocTypes arrive when `PluginStore::register_doctypes`
//! runs. Callers therefore install *after* registration, and must call again
//! after adding DocTypes — this function is a full rebuild, not an append, so
//! calling it twice is safe but calling it once too early is not.
//!
//! [`PolicySource::policy_json`] is re-read on every call for the same reason:
//! [`set_global_engine`](ferris_auth::permissions::set_global_engine) replaces
//! the engine wholesale, so a second install that omitted the operator's JSON
//! would silently drop it.

mod custom;
mod explain;
mod install;
mod source;
#[cfg(test)]
mod tests;

pub use custom::{has_custom_permission, has_custom_permission_on};
pub use explain::{explain, known_doctype};
pub use install::{
    install_policies, install_policies_from_path, install_policies_with_role_permissions,
};
pub use source::{PolicyInstall, PolicySource};
