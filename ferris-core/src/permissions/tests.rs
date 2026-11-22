use super::policy::DOC_RIGHTS;
use super::*;
use crate::Value;
use std::collections::HashMap;

fn user_with_roles(roles: &[&str]) -> crate::user::User {
    let mut user = crate::user::User::system_user("u@local".into(), "U".into());
    user.roles = roles.iter().map(|r| r.to_string()).collect();
    user
}

/// `permlevel` 0 is the document right the engine already checked, so it is
/// always visible; a higher level needs a matching rule, and System Manager
/// bypasses. Mirrors Frappe's `get_permlevel_access`.
#[test]
fn permlevel_access_follows_the_field_rules() {
    let rules = vec![
        PermissionRule {
            role: "Reader".into(),
            read: true,
            permlevel: 0,
            ..Default::default()
        },
        PermissionRule {
            role: "Auditor".into(),
            read: true,
            permlevel: 1,
            ..Default::default()
        },
    ];
    let reader = user_with_roles(&["Reader"]);
    let auditor = user_with_roles(&["Auditor"]);
    let admin = user_with_roles(&[SYSTEM_MANAGER_ROLE]);

    // Level 0 is the row itself: any admitted caller reaches it.
    assert!(has_permlevel_access(&rules, &reader, Permission::Read, 0));
    // Level 1 is granted only by the matching rule.
    assert!(!has_permlevel_access(&rules, &reader, Permission::Read, 1));
    assert!(has_permlevel_access(&rules, &auditor, Permission::Read, 1));
    // The action has to match: the Auditor has no `write` at level 1.
    assert!(!has_permlevel_access(
        &rules,
        &auditor,
        Permission::Write,
        1
    ));
    // System Manager sees every level.
    assert!(has_permlevel_access(&rules, &admin, Permission::Read, 7));
}

/// `All` is Frappe's "every signed-in user", so its field rule reaches a
/// caller who does not literally hold the role.
#[test]
fn the_all_user_role_reaches_everyone() {
    let rules = vec![PermissionRule {
        role: ALL_USER_ROLE.into(),
        read: true,
        permlevel: 1,
        ..Default::default()
    }];
    let any = user_with_roles(&["Some Role"]);
    assert!(has_permlevel_access(&rules, &any, Permission::Read, 1));
}

#[test]
fn test_permission_engine() {
    let mut engine = PermissionEngine::new();

    engine.add_policy(Policy::new(vec![PolicyStatement::allow(
        vec![Principal::role("User")],
        vec![Permission::Read, Permission::Write],
        vec![Resource::new("doctype", "Sales Order")],
    )]));

    let decision = engine.check(
        &Principal::role("User"),
        Permission::Read,
        &Resource::new("doctype", "Sales Order"),
        &HashMap::new(),
    );

    assert_eq!(decision, Decision::Allow);
}

#[test]
fn test_deny_overrides_allow() {
    let mut engine = PermissionEngine::new();
    engine.add_policy(Policy::new(vec![
        PolicyStatement::allow(
            vec![Principal::role("User")],
            vec![Permission::Read],
            vec![Resource::new("doctype", "*")],
        ),
        PolicyStatement::deny(
            vec![Principal::role("User")],
            vec![Permission::Read],
            vec![Resource::new("doctype", "Secret")],
        ),
    ]));

    assert_eq!(
        engine.check(
            &Principal::role("User"),
            Permission::Read,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new()
        ),
        Decision::Allow
    );
    assert_eq!(
        engine.check(
            &Principal::role("User"),
            Permission::Read,
            &Resource::new("doctype", "Secret"),
            &HashMap::new()
        ),
        Decision::Deny
    );
}

#[test]
fn test_role_hierarchy_grants_parent_policy() {
    let mut engine = PermissionEngine::new();
    let mut hierarchy = HashMap::new();
    hierarchy.insert("Manager".to_string(), vec!["System Manager".to_string()]);
    engine.set_role_hierarchy(hierarchy);
    engine.add_policy(Policy::new(vec![PolicyStatement::allow(
        vec![Principal::role("System Manager")],
        vec![Permission::Manage],
        vec![Resource::new("*", "*")],
    )]));

    // A Manager inherits the System Manager grant.
    assert_eq!(
        engine.check(
            &Principal::role("Manager"),
            Permission::Manage,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new()
        ),
        Decision::Allow
    );
    assert_eq!(
        engine.check(
            &Principal::role("Guest"),
            Permission::Manage,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new()
        ),
        Decision::NotApplicable
    );
}

#[test]
fn test_conditions_and_check_user() {
    let mut engine = PermissionEngine::new();
    engine.add_policy(Policy::new(vec![
        PolicyStatement::allow(
            vec![Principal::role("User")],
            vec![Permission::Submit],
            vec![Resource::new("doctype", "Invoice")],
        )
        .with_condition("branch", Value::from("HQ")),
    ]));

    let mut hq = HashMap::new();
    hq.insert("branch".to_string(), Value::from("HQ"));
    assert_eq!(
        engine.check_user(
            &["User".to_string()],
            Permission::Submit,
            &Resource::new("doctype", "Invoice"),
            &hq
        ),
        Decision::Allow
    );

    let mut remote = HashMap::new();
    remote.insert("branch".to_string(), Value::from("Remote"));
    assert_eq!(
        engine.check_user(
            &["User".to_string()],
            Permission::Submit,
            &Resource::new("doctype", "Invoice"),
            &remote
        ),
        Decision::NotApplicable
    );
}

#[test]
fn test_default_engine_is_deny_by_default() {
    let engine = PermissionEngine::default();
    assert_eq!(
        engine.check(
            &Principal::role("System Manager"),
            Permission::Read,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new()
        ),
        Decision::Allow
    );
    assert_eq!(
        engine.check(
            &Principal::role("Guest"),
            Permission::Read,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new()
        ),
        Decision::NotApplicable
    );
}

#[test]
fn test_policy_json_roundtrip_and_load() {
    let policy = Policy::new(vec![PolicyStatement::allow(
        vec![Principal::role("User")],
        vec![Permission::Read],
        vec![Resource::new("doctype", "ToDo")],
    )]);
    let json = policy.to_json_pretty().unwrap();
    assert_eq!(Policy::from_json(&json).unwrap().statements.len(), 1);

    let mut engine = PermissionEngine::new();
    engine.load_policy_json(&json).unwrap();
    assert_eq!(engine.policies().len(), 1);

    // Array form loads multiple policies.
    let mut engine = PermissionEngine::new();
    engine.load_policy_json(&format!("[{json}]")).unwrap();
    assert_eq!(engine.policies().len(), 1);

    assert!(Policy::from_json("not json").is_err());
}

#[test]
fn test_resource_matching() {
    let resource = Resource::new("doctype", "Sales Order").with_owner("user1");
    let pattern = Resource::new("doctype", "Sales Order").with_owner("user1");
    let pattern2 = Resource::new("doctype", "*").with_owner("user1");
    let pattern3 = Resource::new("*", "Sales Order");

    let engine = PermissionEngine::default();

    assert!(engine.resource_matches(&pattern, &resource));
    assert!(engine.resource_matches(&pattern2, &resource));
    assert!(engine.resource_matches(&pattern3, &resource));
}

#[test]
fn test_manage_is_explicit_shorthand_not_hidden_wildcard() {
    // A statement listing only `Read` must NOT match other actions — the
    // old `contains(&Manage)` equality quirk is gone.
    let mut read_only = PermissionEngine::new();
    read_only.add_policy(Policy::new(vec![PolicyStatement::allow(
        vec![Principal::role("User")],
        vec![Permission::Read],
        vec![Resource::new("doctype", "*")],
    )]));
    assert_eq!(
        read_only.check(
            &Principal::role("User"),
            Permission::Read,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new(),
        ),
        Decision::Allow
    );
    assert_eq!(
        read_only.check(
            &Principal::role("User"),
            Permission::Delete,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new(),
        ),
        Decision::NotApplicable,
        "a Read grant must not imply Delete"
    );

    // `Manage` remains the named, deliberate shorthand for every action.
    let mut manage = PermissionEngine::new();
    manage.add_policy(Policy::new(vec![PolicyStatement::allow(
        vec![Principal::role("Admin")],
        vec![Permission::Manage],
        vec![Resource::new("doctype", "*")],
    )]));
    for action in Permission::all() {
        assert_eq!(
            manage.check(
                &Principal::role("Admin"),
                action,
                &Resource::new("doctype", "ToDo"),
                &HashMap::new(),
            ),
            Decision::Allow,
            "Manage shorthand must cover {action:?}"
        );
    }

    // A Manage *deny* short-circuits every action requested by that principal.
    let mut deny_manage = PermissionEngine::new();
    deny_manage.add_policy(Policy::new(vec![
        PolicyStatement::allow(
            vec![Principal::role("Admin")],
            vec![Permission::Read],
            vec![Resource::new("doctype", "*")],
        ),
        PolicyStatement::deny(
            vec![Principal::role("Admin")],
            vec![Permission::Manage],
            vec![Resource::new("doctype", "*")],
        ),
    ]));
    assert_eq!(
        deny_manage.check(
            &Principal::role("Admin"),
            Permission::Read,
            &Resource::new("doctype", "ToDo"),
            &HashMap::new(),
        ),
        Decision::Deny,
        "a Manage deny must deny even otherwise-allowed actions"
    );
}

/// §C3(b): the seeded role catalogue is the scaffold surface for the Role
/// Permissions Builder — a stable name for each framework role. Free-form
/// roles stay legal (nothing here rejects them); the catalogue only has to
/// recognise its own names and leave everything else alone.
#[test]
fn the_standard_role_catalogue_recognises_framework_roles_only() {
    assert!(is_standard_role(SYSTEM_MANAGER_ROLE));
    assert!(is_standard_role("Administrator"));
    assert!(is_standard_role(ALL_USER_ROLE));
    assert!(is_standard_role("Guest"));
    assert!(
        !is_standard_role("Audit-Lens"),
        "app roles are not framework roles"
    );
    assert!(
        !is_standard_role(""),
        "an empty role is not the framework's"
    );
    assert_eq!(
        STANDARD_ROLES.len(),
        6,
        "a change to the catalogue is a visible decision"
    );
    assert!(STANDARD_ROLES.contains(&SYSTEM_MANAGER_ROLE));
}

/// `Manage` means read+write+delete on a `DocPerm` row but *everything* in a
/// policy. The two definitions are deliberate, so this pins both directions
/// and the translation that keeps them apart.
#[test]
fn manage_is_rwdelete_on_a_row_and_everything_in_a_policy() {
    let rwdelete = PermissionRule {
        role: "Manager".into(),
        read: true,
        write: true,
        delete: true,
        ..Default::default()
    };
    assert!(rwdelete.check(Permission::Manage));
    let rw = PermissionRule {
        role: "Writer".into(),
        read: true,
        write: true,
        ..Default::default()
    };
    assert!(!rw.check(Permission::Manage), "read+write is not manage");

    // Policy side: a granted `Manage` covers every action…
    assert!(Permission::Manage.grants(Permission::Email));
    assert!(Permission::Manage.grants(Permission::Import));
    // …while a granted `Read` covers only itself.
    assert!(!Permission::Read.grants(Permission::Write));

    // …but translating the row must not hand that expansion out: the
    // level-0 rights list omits `Manage`, so a three-right row yields
    // exactly three rights and never thirteen.
    let rights: Vec<Permission> = DOC_RIGHTS
        .iter()
        .copied()
        .filter(|right| rwdelete.check(*right))
        .collect();
    assert_eq!(
        rights,
        vec![Permission::Read, Permission::Write, Permission::Delete]
    );
    assert!(!DOC_RIGHTS.contains(&Permission::Manage));
}

/// The principal list is the invariant `check_permission` and `read_scope`
/// share, so it has one construction site now.
#[test]
fn caller_principals_is_the_roles_plus_the_user_itself() {
    let mut user = user_with_roles(&["Desk User", "Auditor"]);
    user.id = "alice@example.com".into();
    assert_eq!(
        caller_principals(&user),
        vec![
            Principal::role("Desk User"),
            Principal::role("Auditor"),
            Principal::user("alice@example.com"),
        ]
    );
}

#[test]
fn permission_parses_through_from_str_and_displays_its_wire_name() {
    use std::str::FromStr;
    assert_eq!("WRITE".parse::<Permission>().unwrap(), Permission::Write);
    assert_eq!(Permission::from_str(" amend ").unwrap(), Permission::Amend);
    let err = "nope".parse::<Permission>().unwrap_err();
    assert_eq!(err, ParsePermissionError("nope".into()));
    assert_eq!(err.to_string(), "unknown permission: nope");
    for permission in Permission::all() {
        assert_eq!(permission.to_string(), permission.as_str());
        assert_eq!(
            permission.as_str().parse::<Permission>().unwrap(),
            permission
        );
    }
}

#[test]
fn resource_display_is_type_name_and_optional_owner() {
    assert_eq!(Resource::new("doctype", "ToDo").to_string(), "doctype:ToDo");
    assert_eq!(
        Resource::new("doctype", "ToDo")
            .with_owner("alice")
            .to_string(),
        "doctype:ToDo:alice"
    );
}
