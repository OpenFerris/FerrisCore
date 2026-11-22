//! Role Permissions Builder merge (C3(a)): persisted `DocPerm` rows join the
//! same rebuild as the registry and the operator document.

use ferris_auth::permissions::Permission;

use super::support::*;
use crate::authz::*;

/// §C3(a): the builder path. A persisted row joins the same rebuild as the
/// registry and grants a role that only the row names. `Notification`
/// declares no DocType rules, so `Audit-Lens` reads it *only* through the
/// store — if the merge dropped the row, `check_permission` would deny.
#[tokio::test]
async fn a_stored_role_permission_row_reaches_the_engine() {
    let _scope = engine_scope();
    {
        crate::core_doctypes::register_core_doctypes();
        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::docperm::DocPermStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(&ferris_auth::docperm::DocPermRow {
                doctype: crate::core_doctypes::NOTIFICATION.to_string(),
                role: "Audit-Lens".to_string(),
                read: true,
                ..Default::default()
            })
            .await
            .unwrap();

        let report = install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();
        assert!(report.statements > 0, "{report:?}");

        let grants = doc(crate::core_doctypes::NOTIFICATION, "N-1", "ada@example.com");
        let role = user("sam@example.com", &["Audit-Lens"]);
        assert!(
            ferris_auth::permissions::check_permission(&role, &grants, Permission::Read),
            "a stored row must grant the named role"
        );
        assert!(
            !ferris_auth::permissions::check_permission(&role, &grants, Permission::Delete),
            "a read-only row must not grant anything else"
        );

        let other = user("sam@example.com", &["Some Other Role"]);
        assert!(
            !ferris_auth::permissions::check_permission(&other, &grants, Permission::Read),
            "a row grants only its named role"
        );
    }
}

/// §C3(a): a `permlevel > 0` row is skipped *and reported* — the framework
/// has no field-level checks, so translating a permission-level grant into
/// an unconditioned statement would be a lie the engine then enforces.
#[tokio::test]
async fn a_non_zero_permlevel_row_is_skipped_and_reported() {
    let _scope = engine_scope();
    {
        crate::core_doctypes::register_core_doctypes();
        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::docperm::DocPermStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(&ferris_auth::docperm::DocPermRow {
                doctype: crate::core_doctypes::NOTIFICATION.to_string(),
                role: "Finance Manager".to_string(),
                read: true,
                permlevel: 2,
                ..Default::default()
            })
            .await
            .unwrap();

        let report = install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();
        assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
        assert!(
            report.skipped[0].reason.as_str().contains("permlevel"),
            "{:?}",
            report.skipped
        );

        let role = user("sam@example.com", &["Finance Manager"]);
        assert!(
            !ferris_auth::permissions::check_permission(
                &role,
                &doc(crate::core_doctypes::NOTIFICATION, "N-1", "ada@example.com"),
                Permission::Read
            ),
            "a skipped non-zero permlevel row must not grant an unconditioned right"
        );
    }
}

/// §C3(a): edits are live — deleting a row stops granting on the next
/// reinstall, which is exactly what the DELETE endpoint does behind the
/// scenes.
#[tokio::test]
async fn a_deleted_row_stops_granting_after_reinstall() {
    let _scope = engine_scope();
    {
        crate::core_doctypes::register_core_doctypes();
        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::docperm::DocPermStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(&ferris_auth::docperm::DocPermRow {
                doctype: crate::core_doctypes::NOTIFICATION.to_string(),
                role: "Audit-Lens".to_string(),
                read: true,
                ..Default::default()
            })
            .await
            .unwrap();
        install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();

        let role = user("sam@example.com", &["Audit-Lens"]);
        let doc_row = doc(crate::core_doctypes::NOTIFICATION, "N-1", "ada@example.com");
        assert!(ferris_auth::permissions::check_permission(
            &role,
            &doc_row,
            Permission::Read
        ));

        store
            .delete(crate::core_doctypes::NOTIFICATION, "Audit-Lens")
            .await
            .unwrap();
        install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();
        assert!(
            !ferris_auth::permissions::check_permission(&role, &doc_row, Permission::Read),
            "the grant must disappear with the row"
        );
    }
}

/// §C3(a)+C3(e) together: a stored row and an operator document merge on
/// one rebuild, not two. If the role-permissions install skipped the file,
/// `Audit-Lens` would lose the read its row grants; if it skipped the row,
/// the delete would linger.
#[tokio::test]
async fn a_stored_row_and_an_operator_document_merge_on_one_install() {
    let _scope = engine_scope();
    {
        crate::core_doctypes::register_core_doctypes();
        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::docperm::DocPermStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(&ferris_auth::docperm::DocPermRow {
                doctype: crate::core_doctypes::NOTIFICATION.to_string(),
                role: "Audit-Lens".to_string(),
                read: true,
                ..Default::default()
            })
            .await
            .unwrap();

        let unique = format!("ferris-merge-{}-{}", std::process::id(), line!());
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("policies.json");
        std::fs::write(
            &file,
            r#"[{"version":"1","statements":[{"effect":"deny",
                "principals":[{"principal_type":"role","identifier":"Audit-Lens"}],
                "actions":["read"],
                "resources":[{"resource_type":"Notification","name":"*","owner":null}],
                "conditions":{}}]}]"#,
        )
        .unwrap();

        let report = install_policies_with_role_permissions(&pool, Some(file.as_path()))
            .await
            .unwrap();
        assert_eq!(report.json_policies, 1);
        assert!(report.statements > 0, "{report:?}");

        let role = user("sam@example.com", &["Audit-Lens"]);
        assert!(
            !ferris_auth::permissions::check_permission(
                &role,
                &doc(crate::core_doctypes::NOTIFICATION, "N-1", "ada@example.com"),
                Permission::Read
            ),
            "the operator's deny must win over the stored row's allow"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// §C3(a): a row may name a DocType the schema registry has never seen.
/// The merge falls back to the default caps (not submittable, no import)
/// and still produces a working statement, so a `DocPerm` row cannot take
/// an unregistered DocType down.
#[tokio::test]
async fn a_row_for_an_unregistered_doctype_still_translates() {
    let _scope = engine_scope();
    {
        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::docperm::DocPermStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(&ferris_auth::docperm::DocPermRow {
                doctype: "Authz Not In Registry".to_string(),
                role: "Audit-Lens".to_string(),
                read: true,
                ..Default::default()
            })
            .await
            .unwrap();

        let report = install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();
        assert!(report.statements > 0, "{report:?}");

        let role = user("sam@example.com", &["Audit-Lens"]);
        assert!(
            ferris_auth::permissions::check_permission(
                &role,
                &doc("Authz Not In Registry", "X-1", "ada@example.com"),
                Permission::Read
            ),
            "a row for an unregistered DocType must still translate"
        );
    }
}
