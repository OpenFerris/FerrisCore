//! Read-only view of what the installed engine will decide.

use ferris_auth::permissions::global_engine_ref;
use ferris_schema::get_doctype;

/// What the installed engine will decide right now, for one caller.
///
/// Not an authorization path — nothing routes a request through it. It exists so
/// an operator can see the effect of a policy change without reading logs, and
/// so a test can assert on the *installed* engine rather than a hand-built one.
///
/// `owner` is the document's stored owner, or `None` for a DocType-level or
/// create-level question. Pass `Some` to see an `if_owner` rule resolve.
pub fn explain(
    doctype: &str,
    roles: &[&str],
    user_id: &str,
    owner: Option<&str>,
    action: ferris_auth::permissions::Permission,
) -> String {
    let stored_owner: &str = owner.unwrap_or("");
    let resource = ferris_auth::permissions::Resource::new(doctype, "*").with_owner(stored_owner);
    let is_owner = !stored_owner.is_empty() && stored_owner == user_id;
    let context = std::collections::HashMap::from([(
        ferris_auth::permissions::OWNER_CONDITION.to_string(),
        ferris_core::Value::Bool(is_owner),
    )]);
    let decision = {
        let engine = global_engine_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        engine.check_user(
            &roles.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
            action,
            &resource,
            &context,
        )
    };
    let role_of = match &decision {
        ferris_auth::permissions::Decision::Allow => "allow",
        ferris_auth::permissions::Decision::Deny => "deny",
        ferris_auth::permissions::Decision::NotApplicable => "no matching statement",
    };
    format!(
        "{doctype} {} for roles {roles:?} (user {user_id}, owner {}): {role_of}",
        action.as_str(),
        owner.unwrap_or("-")
    )
}

/// Whether a DocType is known to the registry, for the CLI's diagnostics.
pub fn known_doctype(name: &str) -> bool {
    get_doctype(name).is_some()
}
