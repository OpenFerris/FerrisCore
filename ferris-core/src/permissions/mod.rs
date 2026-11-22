//! Permission engine with Cedar-like policy language.

mod engine;
mod permission;
mod policy;
mod principal;
mod resource;
#[cfg(test)]
mod scope_tests;
#[cfg(test)]
mod tests;
mod user_permission;

pub use engine::{Decision, PermissionEngine, ReadScope};
pub use permission::{
    ALL_USER_ROLE, OWNER_CONDITION, ParsePermissionError, Permission, PermissionRule,
    STANDARD_ROLES, SYSTEM_MANAGER_ROLE, caller_principals, check_permission, global_engine_ref,
    has_permlevel_access, is_standard_role, principals_for, read_scope, set_global_engine,
};
pub use policy::{
    DocTypeCaps, DocTypePolicy, Effect, Policy, PolicyStatement, SkippedRule, Untranslated,
    doc_type_policy,
};
pub use principal::{Principal, PrincipalType};
pub use resource::Resource;
pub use user_permission::{
    UserPermissionRule, UserPermissionRules, UserPermissionSource, clear_user_permission_source,
    has_user_permissions_for, set_user_permission_source, user_permissions_for,
    value_denied_by_user_permissions,
};
