//! Operator documents: deny overrides, malformed files and the optional path.

use ferris_auth::permissions::{Permission, global_engine_ref};

use super::support::*;
use crate::authz::*;

#[test]
fn an_operator_document_can_deny_what_a_doctype_grants() {
    with_engine(|| {
        install_after(
            "Authz Denied",
            vec![rule("Authz Sales User", true, false, false)],
        );
        let report = install_policies(&PolicySource {
            policy_json: Some(
                r#"[{"version":"1","statements":[{"effect":"deny",
                    "principals":[{"principal_type":"role","identifier":"Authz Sales User"}],
                    "actions":["read"],
                    "resources":[{"resource_type":"Authz Denied","name":"*","owner":null}],
                    "conditions":{}}]}]"#
                    .to_string(),
            ),
            exclude: Vec::new(),
        });
        assert_eq!(report.json_policies, 1);

        let role = user("sam@example.com", &["Authz Sales User"]);
        assert!(
            !ferris_auth::permissions::check_permission(
                &role,
                &doc("Authz Denied", "D-1", "ada@example.com"),
                Permission::Read
            ),
            "deny must win over the DocType's allow"
        );
    });
}

#[test]
fn a_malformed_operator_document_does_not_silently_succeed() {
    with_engine(|| {
        install_after(
            "Authz Bad",
            vec![rule("Authz Sales User", true, false, false)],
        );
        let report = install_policies(&PolicySource {
            policy_json: Some("{ not json".to_string()),
            exclude: Vec::new(),
        });
        assert_eq!(
            report.json_policies, 0,
            "a rejected document is not counted as loaded"
        );
        // The DocType statements survive: a broken operator file denies by
        // omission, it does not take the declared rules with it.
        let role = user("sam@example.com", &["Authz Sales User"]);
        assert!(ferris_auth::permissions::check_permission(
            &role,
            &doc("Authz Bad", "B-1", "ada@example.com"),
            Permission::Read
        ));
    });
}

#[test]
fn excluded_doctypes_are_not_translated() {
    with_engine(|| {
        install_after(
            "Authz Excluded",
            vec![rule("Authz Sales User", true, false, false)],
        );
        install_policies(&PolicySource {
            policy_json: None,
            exclude: vec!["Authz Excluded".into()],
        });
        let role = user("sam@example.com", &["Authz Sales User"]);
        assert!(!ferris_auth::permissions::check_permission(
            &role,
            &doc("Authz Excluded", "E-1", "ada@example.com"),
            Permission::Read
        ));
    });
}

#[test]
fn install_is_idempotent() {
    with_engine(|| {
        install_after(
            "Authz Twice",
            vec![rule("Authz Twice Reader", true, false, false)],
        );
        let policies = || {
            global_engine_ref()
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .policies()
                .len()
        };
        let after_first = policies();
        install_policies(&PolicySource::default());
        assert_eq!(
            after_first,
            policies(),
            "a rebuild must replace the engine, not append to it"
        );
    });
}

#[test]
fn explain_reports_the_installed_decision() {
    with_engine(|| {
        install_after(
            "Authz Explained",
            vec![rule("Authz Explained Reader", true, false, false)],
        );
        let yes = explain(
            "Authz Explained",
            &["Authz Explained Reader"],
            "sam@example.com",
            Some("ada@example.com"),
            Permission::Read,
        );
        assert!(yes.contains("allow"), "{yes}");
        let no = explain(
            "Authz Explained",
            &["Authz Explained Reader"],
            "sam@example.com",
            Some("ada@example.com"),
            Permission::Delete,
        );
        assert!(no.contains("no matching statement"), "{no}");
    });
}

/// The operator-facing entry point, `ServerSettings::policy_path`.
///
/// Three cases, and the middle one is the point: an absent path is not a
/// failure, so an install with no policy document behaves exactly like one
/// with an empty `PolicySource`; a path that exists and parses adds its
/// statements; a path that exists and does not parse is reported and the
/// DocType-declared rules still stand.
#[test]
fn the_policy_path_is_optional_but_a_bad_one_is_not_silent() {
    let unique = format!("ferris-authz-{}-{}", std::process::id(), line!());
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();

    with_engine(|| {
        install_after("Authz FromFile", vec![]);

        // Absent.
        let missing = dir.join("does-not-exist.json");
        let report = install_policies_from_path(Some(missing.as_path()));
        assert_eq!(report.json_policies, 0);
        let reader = user("sam@example.com", &["Authz FromFile Reader"]);
        assert!(!ferris_auth::permissions::check_permission(
            &reader,
            &doc("Authz FromFile", "F-1", "ada@example.com"),
            Permission::Read
        ));

        // Present and parseable.
        let good = dir.join("good.json");
        std::fs::write(
            &good,
            r#"[{"version":"1","statements":[{"effect":"allow",
                "principals":[{"principal_type":"role","identifier":"Authz FromFile Reader"}],
                "actions":["read"],
                "resources":[{"resource_type":"Authz FromFile","name":"*","owner":null}],
                "conditions":{}}]}]"#,
        )
        .unwrap();
        let report = install_policies_from_path(Some(good.as_path()));
        assert_eq!(report.json_policies, 1);
        assert!(ferris_auth::permissions::check_permission(
            &reader,
            &doc("Authz FromFile", "F-1", "ada@example.com"),
            Permission::Read
        ));

        // Present and broken.
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        let report = install_policies_from_path(Some(bad.as_path()));
        assert_eq!(
            report.json_policies, 0,
            "a rejected document is not counted as loaded"
        );
        assert!(
            !ferris_auth::permissions::check_permission(
                &reader,
                &doc("Authz FromFile", "F-1", "ada@example.com"),
                Permission::Read
            ),
            "a broken operator document denies by omission rather than keeping the last good one"
        );
    });

    let _ = std::fs::remove_dir_all(&dir);
}
