use super::*;
use crate::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------
// §C2 — `read_scope`, the whole-DocType answer
//
// These pin the *classification*, and the classification is what lets the
// list query put `LIMIT` in the SQL. A wrong `All` leaks documents; a wrong
// `Owner` hides them; a wrong `None` hides the whole DocType. So every
// shape has a test that says what a probe on the rows would have answered,
// next to what the analysis answered.
// ---------------------------------------------------------------------

fn scope(statements: Vec<PolicyStatement>, role: &str, user: &str, owner: &str) -> ReadScope {
    let mut engine = PermissionEngine::new();
    engine.add_policy(Policy::new(statements));
    engine.read_scope(
        &[Principal::role(role), Principal::user(user)],
        "ToDo",
        owner,
    )
}

/// A grant over every row of `ToDo`, in the two-resource form
/// [`doc_type_policy`] emits.
fn doctype_wide(role: &str) -> PolicyStatement {
    PolicyStatement::allow(
        vec![Principal::role(role)],
        vec![Permission::Read],
        vec![Resource::new("ToDo", "*"), Resource::new("doctype", "ToDo")],
    )
}

/// A grant naming the DocType with a wildcard name covers every row, so the
/// list needs no per-row check at all.
#[test]
fn a_doctype_wide_grant_reads_every_row() {
    assert_eq!(
        scope(
            vec![doctype_wide("Reader")],
            "Reader",
            "ada@local",
            "bob@local"
        ),
        ReadScope::All
    );
}

/// `Manage` is the explicit shorthand for every action, so it covers `Read`
/// and the analysis has to see that rather than special-case `Read`.
#[test]
fn a_manage_grant_reads_every_row() {
    let statement = PolicyStatement::allow(
        vec![Principal::role("Admin")],
        vec![Permission::Manage],
        vec![Resource::new("ToDo", "*")],
    );
    assert_eq!(
        scope(vec![statement], "Admin", "ada@local", "bob@local"),
        ReadScope::All
    );
}

/// Frappe's `if_owner` is a condition on `is_owner`, and it is exactly
/// `owner = ?`. That is the whole reason `ReadScope::Owner` exists.
#[test]
fn an_if_owner_grant_reduces_to_an_owner_where_term() {
    let statement = doctype_wide("Owner").with_condition(OWNER_CONDITION, Value::Bool(true));
    assert_eq!(
        scope(vec![statement], "Owner", "ada@local", "ada@local"),
        ReadScope::Owner
    );
}

/// With nothing granting `Read`, the caller still reads what they own —
/// `check_permission`'s owner shortcut. So the answer is `Owner`, not `None`.
#[test]
fn no_grant_at_all_still_reads_the_callers_own_rows() {
    assert_eq!(
        scope(Vec::new(), "Nobody", "ada@local", "ada@local"),
        ReadScope::Owner
    );
}

/// A statement pinned to a *concrete* row is row-dependent, and the analysis
/// must not mistake the presence of an allow for a universal one: doing so
/// would hand the caller every row of the DocType because one was named.
#[test]
fn a_statement_naming_one_row_is_row_dependent() {
    let statement = PolicyStatement::allow(
        vec![Principal::role("Reader")],
        vec![Permission::Read],
        vec![Resource::new("ToDo", "TODO-0001")],
    );
    assert_eq!(
        scope(vec![statement], "Reader", "ada@local", "bob@local"),
        ReadScope::PerRow
    );
}

/// A condition the caller cannot be indexed by is row-dependent for the same
/// reason. `is_owner = false` in particular grants the rows the caller does
/// *not* own, which is the complement of `Owner` and must not be mistaken
/// for it.
#[test]
fn a_condition_other_than_is_owner_true_is_row_dependent() {
    let statement = doctype_wide("Reader").with_condition("is_department_head", Value::Bool(true));
    assert_eq!(
        scope(vec![statement], "Reader", "ada@local", "bob@local"),
        ReadScope::PerRow
    );

    let inverted = doctype_wide("Anti").with_condition(OWNER_CONDITION, Value::Bool(false));
    assert_eq!(
        scope(vec![inverted], "Anti", "ada@local", "bob@local"),
        ReadScope::PerRow
    );
}

/// A deny that covers every row beats every allow, so the list is empty and
/// no query has to run. This is the case that a naive "is there an allow?"
/// check would get wrong.
#[test]
fn a_doctype_wide_deny_overrides_a_doctype_wide_allow() {
    let deny = PolicyStatement::deny(
        vec![Principal::role("Reader")],
        vec![Permission::Read],
        vec![Resource::new("ToDo", "*")],
    );
    assert_eq!(
        scope(
            vec![doctype_wide("Reader"), deny],
            "Reader",
            "ada@local",
            "bob@local"
        ),
        ReadScope::None
    );
}

/// A deny scoped to one row cannot be answered without looking at rows, so
/// the scope falls back to per-row — where `check` applies it correctly.
#[test]
fn a_partial_deny_forces_the_per_row_path() {
    let deny = PolicyStatement::deny(
        vec![Principal::role("Reader")],
        vec![Permission::Read],
        vec![Resource::new("ToDo", "TODO-0001")],
    );
    assert_eq!(
        scope(
            vec![doctype_wide("Reader"), deny],
            "Reader",
            "ada@local",
            "bob@local"
        ),
        ReadScope::PerRow
    );
}

/// A grant about a different DocType must not leak into this one, and a
/// DocType-*level* grant — `Resource::new("doctype", doctype)`, which
/// authorizes the schema rather than a document — must not either. Both would
/// otherwise widen the list to rows the checker never granted.
#[test]
fn only_document_grants_for_this_doctype_are_considered() {
    let elsewhere = PolicyStatement::allow(
        vec![Principal::role("Reader")],
        vec![Permission::Read],
        vec![Resource::new("Sales Order", "*")],
    );
    let schema_level = PolicyStatement::allow(
        vec![Principal::role("Reader")],
        vec![Permission::Read],
        vec![Resource::new("doctype", "ToDo")],
    );
    assert_eq!(
        scope(
            vec![
                doctype_wide("Reader"),
                elsewhere.clone(),
                schema_level.clone()
            ],
            "Reader",
            "ada@local",
            "bob@local"
        ),
        ReadScope::All,
        "the document grant decides, and the other two are simply not counted"
    );

    for statements in [vec![elsewhere.clone()], vec![schema_level.clone()]] {
        assert_eq!(
            scope(statements, "Reader", "ada@local", "bob@local"),
            ReadScope::Owner,
            "neither grant reaches a document, so only the owner shortcut does"
        );
    }

    // And the checker agrees, which is the reason they are excluded: a
    // DocType-level grant never matches a document resource, so it decides
    // nothing about rows.
    let mut engine = PermissionEngine::new();
    engine.add_policy(Policy::new(vec![schema_level]));
    assert_eq!(
        engine.check(
            &Principal::role("Reader"),
            Permission::Read,
            &Resource::new("ToDo", "TODO-0001"),
            &HashMap::new()
        ),
        Decision::NotApplicable
    );
}

/// A role the caller does not hold contributes nothing, and a role it holds
/// *through the hierarchy* contributes everything.
#[test]
fn role_hierarchy_applies_to_the_scope_as_it_does_to_check() {
    let mut engine = PermissionEngine::new();
    engine.add_policy(Policy::new(vec![doctype_wide("Senior")]));
    engine.set_role_hierarchy(HashMap::from([(
        "Junior".to_string(),
        vec!["Senior".to_string()],
    )]));
    let juniors = [Principal::role("Junior"), Principal::user("ada@local")];
    assert_eq!(
        engine.read_scope(&juniors, "ToDo", "bob@local"),
        ReadScope::All
    );

    let strangers = [Principal::role("Intern"), Principal::user("ada@local")];
    assert_eq!(
        engine.read_scope(&strangers, "ToDo", "bob@local"),
        ReadScope::Owner,
        "an unrelated role must not inherit the grant"
    );
}

/// The analysis and the per-row check must agree, or `list` would take one
/// path for the page and another for the total. This is the invariant that
/// makes the push-down safe, checked over the rows the scope claims to cover.
#[test]
fn the_scope_agrees_with_checking_each_row() {
    // A table of rows: two owned by the caller, two not.
    let rows = [
        ("TODO-0001", "ada@local"),
        ("TODO-0002", "bob@local"),
        ("TODO-0003", "ada@local"),
        ("TODO-0004", "carol@local"),
    ];
    let caller = "ada@local";

    for statements in [
        // Universal allow.
        vec![doctype_wide("Reader")],
        // Owner-only via the shortcut.
        Vec::new(),
        // Owner-only via an `if_owner` condition.
        vec![doctype_wide("Owner").with_condition(OWNER_CONDITION, Value::Bool(true))],
        // Row-dependent: one named row plus a universal allow.
        vec![
            doctype_wide("Reader"),
            PolicyStatement::allow(
                vec![Principal::role("Reader")],
                vec![Permission::Read],
                vec![Resource::new("ToDo", "TODO-0004")],
            ),
        ],
        // Universal deny.
        vec![PolicyStatement::deny(
            vec![Principal::role("Reader")],
            vec![Permission::Read],
            vec![Resource::new("doctype", "ToDo")],
        )],
    ] {
        let mut engine = PermissionEngine::new();
        engine.add_policy(Policy::new(statements));
        let principals = [Principal::role("Reader"), Principal::role("Owner")];
        let got = engine.read_scope(&principals, "ToDo", caller);

        // What a real call would answer, reproducing
        // `ferris_auth::permissions::check_permission` in full: engine
        // decision first, then the owner shortcut when nothing applied.
        // Modelling only the engine half is what made this test report a
        // disagreement that did not exist — `ReadScope::Owner` *is* the
        // shortcut, and the engine alone says `NotApplicable`.
        let decided = |name: &str, owner: &str| -> bool {
            let resource = Resource::new("ToDo", name).with_owner(owner);
            let decision = engine.check_principals(
                &principals,
                Permission::Read,
                &resource,
                &HashMap::from([(OWNER_CONDITION.to_string(), Value::Bool(owner == caller))]),
            );
            match decision {
                Decision::Deny => false,
                Decision::Allow => true,
                Decision::NotApplicable => owner == caller,
            }
        };

        // What the scope claims, as a filter over the same rows.
        let covered = |name: &str, owner: &str| -> bool {
            match got {
                ReadScope::All => true,
                ReadScope::None => false,
                ReadScope::Owner => owner == caller,
                ReadScope::PerRow => decided(name, owner),
            }
        };

        for (name, owner) in rows {
            let expected = decided(name, owner);
            assert_eq!(
                covered(name, owner),
                expected,
                "scope {got:?} disagrees with check for {name} owned by {owner}"
            );
        }
    }
}
