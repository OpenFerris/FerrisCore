//! DocType-declaration translation and the shape of the statements it emits.

use ferris_auth::permissions::{DocTypeCaps, Permission, PrincipalType, doc_type_policy};

use super::support::*;
use crate::authz::*;

#[test]
fn install_translates_declared_rules_into_allow_statements() {
    with_engine(|| {
        let report = install_after(
            "Authz ToDo",
            vec![rule("Authz Sales User", true, true, false)],
        );
        assert_eq!(report.statements, 1);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);

        let role = user("sam@example.com", &["Authz Sales User"]);
        let foreign = doc("Authz ToDo", "TODO-0001", "ada@example.com");
        assert!(
            ferris_auth::permissions::check_permission(&role, &foreign, Permission::Read),
            "a role granted read on the DocType must read any of its documents"
        );
        assert!(
            ferris_auth::permissions::check_permission(&role, &foreign, Permission::Write),
            "…and write, which the rule granted"
        );
        assert!(
            !ferris_auth::permissions::check_permission(&role, &foreign, Permission::Delete),
            "…but not delete, which it did not"
        );
    });
}

#[test]
fn a_doctype_that_declares_nothing_stays_unreadable() {
    with_engine(|| {
        let report = install_after("Authz Silent", vec![]);
        assert_eq!(report.statements, 0);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);

        let role = user("sam@example.com", &["Authz Sales User"]);
        assert!(!ferris_auth::permissions::check_permission(
            &role,
            &doc("Authz Silent", "S-1", "ada@example.com"),
            Permission::Read
        ));
    });
}

#[test]
fn all_role_reaches_every_authenticated_caller() {
    with_engine(|| {
        install_after("Authz Public", vec![rule("All", true, false, false)]);

        // No roles at all. Frappe's `get_roles` hands `ALL_USER_ROLE` to every
        // signed-in user, so "no roles" is not a reason to be refused.
        let nobody = user("sam@example.com", &[]);
        assert!(
            ferris_auth::permissions::check_permission(
                &nobody,
                &doc("Authz Public", "P-1", "ada@example.com"),
                Permission::Read
            ),
            "an `All` grant must not require the caller to hold a named role"
        );
    });
}

#[test]
fn a_statement_naming_one_user_reaches_that_user() {
    with_engine(|| {
        ferris_schema::register_doctype(meta("Authz ByUser", vec![]));
        // No DocType rules at all: the grant comes from the operator document.
        install_policies(&PolicySource {
            policy_json: Some(
                r#"[{"version":"1","statements":[{"effect":"allow",
                    "principals":[{"principal_type":"user","identifier":"ada@example.com"}],
                    "actions":["read"],
                    "resources":[{"resource_type":"Authz ByUser","name":"*","owner":null}],
                    "conditions":{}}]}]"#
                    .to_string(),
            ),
            exclude: Vec::new(),
        });

        let ada = user("ada@example.com", &[]);
        let grace = user("grace@example.com", &[]);
        let d = doc("Authz ByUser", "U-1", "sam@example.com");
        assert!(ferris_auth::permissions::check_permission(
            &ada,
            &d,
            Permission::Read
        ));
        assert!(
            !ferris_auth::permissions::check_permission(&grace, &d, Permission::Read),
            "a per-user grant must not leak to anyone else"
        );
    });
}

#[test]
fn if_owner_grants_the_owner_and_only_the_owner() {
    with_engine(|| {
        let report = install_after(
            "Authz Owned",
            vec![ferris_schema::PermissionRule {
                role: "Authz Desk User".into(),
                read: true,
                delete: true,
                if_owner: true,
                ..Default::default()
            }],
        );
        assert_eq!(report.statements, 1);

        let member = user("ada@example.com", &["Authz Desk User"]);
        let mine = doc("Authz Owned", "O-1", "ada@example.com");
        let theirs = doc("Authz Owned", "O-2", "grace@example.com");

        assert!(ferris_auth::permissions::check_permission(
            &member,
            &mine,
            Permission::Delete
        ));
        assert!(ferris_auth::permissions::check_permission(
            &member,
            &mine,
            Permission::Read
        ));
        assert!(
            !ferris_auth::permissions::check_permission(&member, &theirs, Permission::Delete),
            "if_owner must withhold delete from a non-owner, even though the rule grants the role delete"
        );
        assert!(
            !ferris_auth::permissions::check_permission(&member, &theirs, Permission::Read),
            "and withhold read from a non-owner too: the statement carries an is_owner condition, \
             so Frappe's doctype-level read=1 escape does not reach the document"
        );
    });
}

#[test]
fn if_owner_never_grants_create() {
    with_engine(|| {
        let report = install_after(
            "Authz OwnedCreate",
            vec![ferris_schema::PermissionRule {
                role: "Authz Desk User".into(),
                create: true,
                if_owner: true,
                ..Default::default()
            }],
        );
        assert_eq!(report.statements, 0, "create is not an ownable right");
        assert_eq!(report.skipped.len(), 1);

        let member = user("ada@example.com", &["Authz Desk User"]);
        assert!(!ferris_auth::permissions::check_permission(
            &member,
            &doc("Authz OwnedCreate", "NEW", ""),
            Permission::Create
        ));
    });
}

#[test]
fn permlevel_rules_are_reported_not_translated() {
    with_engine(|| {
        let report = install_after(
            "Authz Levels",
            vec![ferris_schema::PermissionRule {
                role: "Authz Manager".into(),
                read: true,
                permlevel: 3,
                ..Default::default()
            }],
        );
        assert_eq!(report.statements, 0, "permlevel is a field-level concept");
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(
            report.skipped[0].reason.as_str(),
            "permlevel is not 0 (field-level permission)"
        );
    });
}

#[test]
fn submit_and_import_are_withheld_when_the_doctype_forbids_them() {
    with_engine(|| {
        install_after(
            "Authz Plain",
            vec![ferris_schema::PermissionRule {
                role: "Authz Manager".into(),
                read: true,
                submit: true,
                import: true,
                ..Default::default()
            }],
        );

        let manager = user("sam@example.com", &["Authz Manager"]);
        let d = doc("Authz Plain", "PL-1", "ada@example.com");
        assert!(ferris_auth::permissions::check_permission(
            &manager,
            &d,
            Permission::Read
        ));
        assert!(
            !ferris_auth::permissions::check_permission(&manager, &d, Permission::Submit),
            "is_submittable is false, so submit must not survive the translation"
        );
        assert!(
            !ferris_auth::permissions::check_permission(&manager, &d, Permission::Import),
            "allow_import is false, so import must not survive the translation"
        );
    });
}

#[test]
fn apply_to_all_doctypes_widens_the_resource() {
    with_engine(|| {
        install_after(
            "Authz Wide",
            vec![ferris_schema::PermissionRule {
                role: "Authz Auditor".into(),
                read: true,
                apply_to_all_doctypes: true,
                ..Default::default()
            }],
        );

        let auditor = user("sam@example.com", &["Authz Auditor"]);
        assert!(ferris_auth::permissions::check_permission(
            &auditor,
            &doc("Some Other DocType", "X-1", "ada@example.com"),
            Permission::Read
        ));
    });
}

#[test]
fn statements_name_both_resource_shapes() {
    // The DocType-level shape the typed bridge and the realtime check ask
    // with, and the document-level shape the REST path asks with.
    let policy = doc_type_policy(
        "Authz Shapes",
        &[rule("Authz Reader", true, false, false)],
        DocTypeCaps::default(),
    );
    let resources = &policy.statements[0].resources;
    assert_eq!(resources.len(), 2, "{resources:?}");
    assert!(resources.iter().any(|r| r.resource_type == "Authz Shapes"));
    assert!(
        resources
            .iter()
            .any(|r| r.resource_type == "doctype" && r.name == "Authz Shapes")
    );

    let principals: Vec<_> = policy
        .statements
        .iter()
        .flat_map(|s| s.principals.iter())
        .collect();
    assert!(
        principals
            .iter()
            .all(|p| p.principal_type == PrincipalType::Role)
    );
}

#[test]
fn manage_is_never_emitted_as_an_action() {
    // `PermissionRule::check(Manage)` is true for any rule with
    // read+write+delete, and `Manage` expands to every action at decision
    // time. Emitting it would grant all thirteen rights from a three-right
    // rule.
    let policy = doc_type_policy(
        "Authz Manage",
        &[rule("Authz Manager", true, true, true)],
        DocTypeCaps {
            submittable: true,
            allow_import: true,
        },
    );
    assert_eq!(
        policy.statements[0].actions.len(),
        3,
        "{:?}",
        policy.statements[0].actions
    );
    assert!(
        !policy.statements[0]
            .actions
            .iter()
            .any(|a| *a == Permission::Manage)
    );
}

#[test]
fn a_blank_owner_is_never_ownership() {
    with_engine(|| {
        install_after(
            "Authz Blank",
            vec![ferris_schema::PermissionRule {
                role: "Authz Desk User".into(),
                delete: true,
                if_owner: true,
                ..Default::default()
            }],
        );
        let nobody = user("", &["Authz Desk User"]);
        assert!(
            !ferris_auth::permissions::check_permission(
                &nobody,
                &doc("Authz Blank", "B-1", ""),
                Permission::Delete
            ),
            "a document with no stored owner must not be owned by a user with a blank id"
        );
    });
}

/// `skipped_lines` is what an operator reads in the log, so its output has to
/// name the role and say why — a bare counter is the failure mode this module
/// exists to remove.
#[test]
fn a_skipped_rule_is_reported_with_its_role_and_reason() {
    let policy = doc_type_policy(
        "Authz Report",
        &[ferris_core::PermissionRule {
            role: "Finance Manager".into(),
            read: true,
            permlevel: 2,
            ..Default::default()
        }],
        DocTypeCaps::default(),
    );
    let install = PolicyInstall {
        doctypes: 1,
        doctypes_with_rules: 1,
        statements: policy.statements.len(),
        json_policies: 0,
        skipped: policy.skipped,
    };
    let lines = install.skipped_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("Finance Manager"), "{}", lines[0]);
    assert!(lines[0].contains("permlevel"), "{}", lines[0]);
}
