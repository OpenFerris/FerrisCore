//! Ferris Auth - Re-exports the permission engine from `ferris-core` and adds
//! the auth-facing `ResourceAccess` view.
//!
//! The engine, the identity predicate [`check_permission`] and the whole-DocType
//! [`read_scope`] now live in [`ferris_core::permissions`]: `ferris-document`
//! authorises every read and write against them and cannot depend on this crate.
//! They are re-exported here so `ferris_auth::permissions::check_permission`
//! (and `ferris_auth::check_permission`) keep working.

pub use ferris_core::permissions::{
    ALL_USER_ROLE, Decision, DocTypeCaps, DocTypePolicy, Effect, OWNER_CONDITION, Permission,
    PermissionEngine, PermissionRule, Policy, PolicyStatement, Principal, PrincipalType, ReadScope,
    Resource, SkippedRule, Untranslated, check_permission, doc_type_policy, global_engine_ref,
    read_scope, set_global_engine,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_engine_is_deny_by_default() {
        let engine = PermissionEngine::default();
        assert_eq!(
            engine.check(
                &Principal::role("System Manager"),
                Permission::Read,
                &Resource::new("doctype", "ToDo"),
                &std::collections::HashMap::new()
            ),
            Decision::Allow
        );
        assert_eq!(
            engine.check(
                &Principal::role("Guest"),
                Permission::Read,
                &Resource::new("doctype", "ToDo"),
                &std::collections::HashMap::new()
            ),
            Decision::NotApplicable
        );
    }
}
