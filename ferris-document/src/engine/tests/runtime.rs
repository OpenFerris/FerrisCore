use super::*;

#[tokio::test]
async fn a_registered_workflow_gates_a_transition() {
    use crate::workflow::{
        StateStyle, Workflow, WorkflowState, WorkflowTransition, global_workflow_engine,
    };

    // A doctype name no other test uses, so the process-global workflow registry
    // cannot leak into them (there is one registry per process).
    let doctype = "WorkflowGateDoc";
    global_workflow_engine()
        .register(Workflow {
            name: "Gate".to_string(),
            doctype: doctype.to_string(),
            initial_state: "Open".to_string(),
            states: vec![
                WorkflowState {
                    state: "Open".to_string(),
                    label: None,
                    style: StateStyle::Default,
                    doc_status: DocStatus::Draft,
                    allow_edit: vec![],
                    allowed_actions: vec![],
                    auto_assign: None,
                },
                WorkflowState {
                    state: "Approved".to_string(),
                    label: None,
                    style: StateStyle::Default,
                    doc_status: DocStatus::Submitted,
                    allow_edit: vec![],
                    allowed_actions: vec![],
                    auto_assign: None,
                },
            ],
            // Only `Submit` is declared; `Cancel` is not.
            transitions: vec![WorkflowTransition {
                name: None,
                action: "Submit".to_string(),
                from_state: "Open".to_string(),
                to_state: "Approved".to_string(),
                allowed_roles: vec![],
                condition: None,
                required_fields: vec![],
                notify: false,
            }],
        })
        .expect("register the gating workflow once");

    let engine = engine().await;
    let ctx = EngineContext::new(doctype, "WG-.#####").allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "gate me" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // The declared transition is allowed.
    let submitted = engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .expect("the workflow declares Open -> Approved on Submit");
    assert_eq!(
        submitted.get("docstatus").and_then(|v| v.as_str()),
        Some("submitted")
    );

    // `Submitted -> Cancelled` is a legal `DocStatus` move, but the workflow has
    // no transition for it, so the workflow refuses. Without DOC6 this call
    // would succeed on `can_transition_to` alone.
    let err = engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Workflow(_)),
        "the workflow must refuse a transition it does not declare, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// DOC7 — version history is recorded on the write path
// ---------------------------------------------------------------------------

/// DOC7: a write through the engine records a version, and
/// the version survives a round trip through `VersionManager::get_versions`.
///
/// Before the wiring, `tab_version` was cascaded on delete but nothing ever
/// inserted a row, so history was always empty.
#[tokio::test]
async fn the_engine_records_version_history() {
    use crate::versioning::{ChangeType, VersionManager};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new("Versioned", "V-.#####").allow_anonymous(true);

    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "first" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    let manager = VersionManager::with_db(db);
    let versions = manager.get_versions("Versioned", &name).await.unwrap();
    assert_eq!(versions.len(), 1, "an insert records exactly one version");
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[0].change_type, ChangeType::Create);
    assert_eq!(
        versions[0].data.get("title").and_then(|v| v.as_str()),
        Some("first"),
        "the recorded version is the document the engine stored"
    );

    // An update appends a second version, numbered after the first.
    engine
        .update(&ctx, &name, ferris_core::value!({ "title": "second" }))
        .await
        .unwrap();
    let versions = manager.get_versions("Versioned", &name).await.unwrap();
    assert_eq!(versions.len(), 2, "an update appends a version");
    assert_eq!(versions[1].version, 2);
    assert_eq!(versions[1].change_type, ChangeType::Update);
    assert_eq!(
        versions[1].data.get("title").and_then(|v| v.as_str()),
        Some("second")
    );
}

/// DOC7: submit, cancel, rename and delete are recorded by the engine too, so
/// the API layer no longer needs a second writer.
#[tokio::test]
async fn transitions_rename_and_delete_record_history() {
    use crate::versioning::{ChangeType, VersionManager};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let meta = DoctypeMeta {
        name: "History".to_string(),
        module: "Test".to_string(),
        allow_rename: true,
        ..Default::default()
    };
    let ctx = EngineContext::new("History", "H-.#####")
        .allow_anonymous(true)
        .with_meta(&meta);

    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "h" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    let manager = VersionManager::with_db(db.clone());

    // Rename is a draft operation now (DOC-15); it carries the history to the
    // new name instead of orphaning it.
    engine.rename(&ctx, &name, "H-RENAMED").await.unwrap();
    assert!(
        manager
            .get_versions("History", &name)
            .await
            .unwrap()
            .is_empty(),
        "the old name must not keep the history"
    );
    let renamed = "H-RENAMED";
    assert_eq!(
        manager
            .get_versions("History", renamed)
            .await
            .unwrap()
            .len(),
        1,
        "the Create version moved with the document"
    );

    engine
        .set_docstatus(&ctx, renamed, DocStatus::Submitted)
        .await
        .unwrap();
    engine
        .set_docstatus(&ctx, renamed, DocStatus::Cancelled)
        .await
        .unwrap();
    let versions = manager.get_versions("History", renamed).await.unwrap();
    assert_eq!(
        versions.iter().map(|v| v.change_type).collect::<Vec<_>>(),
        vec![ChangeType::Create, ChangeType::Submit, ChangeType::Cancel]
    );

    // SEC-07/DX-01: a cancelled document keeps its history and gains a trash
    // row, so cancelling and deleting cannot erase the evidence.
    engine.delete(&ctx, renamed).await.unwrap();
    let versions = manager.get_versions("History", "H-RENAMED").await.unwrap();
    assert_eq!(
        versions.iter().map(|v| v.change_type).collect::<Vec<_>>(),
        vec![
            ChangeType::Create,
            ChangeType::Submit,
            ChangeType::Cancel,
            ChangeType::Delete
        ]
    );
    assert!(
        versions.last().unwrap().data.is_empty(),
        "the delete marker carries no document body"
    );
    let trash = engine
        .deleted_documents("History", "H-RENAMED")
        .await
        .unwrap();
    assert_eq!(trash.len(), 1, "the cancelled document is in the trash");
    assert_eq!(
        trash[0].data.get("title").and_then(|v| v.as_str()),
        Some("h"),
        "the trash row holds the complete snapshot"
    );
}

// ---------------------------------------------------------------------------
// DOC8 — a share grants access where the role rules deny
// ---------------------------------------------------------------------------

/// DOC8: a stored share grants `Read` and `Write` to a user
/// the role rules refuse, and the share does not widen anyone else's access.
///
/// Before the wiring, `ShareManager` had no production caller: `tab_share` was
/// cascaded on delete but never written, so a shared document did not exist as a
/// concept.
#[tokio::test]
async fn a_share_grants_access_where_roles_deny() {
    use crate::sharing::{ShareManager, SharePermission};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let admin = sysadmin();
    let alice = plain_user();
    let admin_ctx = EngineContext::new("Shared", "S-.#####").with_user(&admin);
    let alice_ctx = EngineContext::new("Shared", "S-.#####").with_user(&alice);

    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "secret" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // Baseline: alice is refused by the role rules on the admin-owned document.
    let err = engine.get(&alice_ctx, &name).await.unwrap_err();
    assert!(
        matches!(
            err,
            Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
        ),
        "expected a denial before the share, got {err:?}"
    );

    // Grant read + write to alice.
    let shares = ShareManager::with_db(db.clone());
    shares.init().await.unwrap();
    shares
        .share(
            "Shared",
            &name,
            "alice@local",
            vec![SharePermission::Read, SharePermission::Write],
            "admin@local",
        )
        .await
        .unwrap();

    // Now alice can read and write the document she does not own.
    let read = engine
        .get(&alice_ctx, &name)
        .await
        .expect("the share grants Read");
    assert_eq!(read.get("title").and_then(|v| v.as_str()), Some("secret"));
    engine
        .update(
            &alice_ctx,
            &name,
            ferris_core::value!({ "title": "edited by alice" }),
        )
        .await
        .expect("the share grants Write");

    // A third party with no share is still refused — the grant is per user.
    let mut bob = ferris_core::User::system_user("bob@local".to_string(), "Bob".to_string());
    bob.id = "bob@local".to_string();
    let bob_ctx = EngineContext::new("Shared", "S-.#####").with_user(&bob);
    let err = engine.get(&bob_ctx, &name).await.unwrap_err();
    assert!(
        matches!(
            err,
            Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
        ),
        "a share must not widen access for an unrelated user, got {err:?}"
    );
}

/// DOC8: a shared document appears in the caller's `list`, even though it is not
/// in the caller's role scope.
#[tokio::test]
async fn a_shared_document_appears_in_the_list() {
    use crate::ListQuery;
    use crate::sharing::{ShareManager, SharePermission};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let admin = sysadmin();
    let alice = plain_user();
    let admin_ctx = EngineContext::new("ListShared", "L-.#####").with_user(&admin);
    let alice_ctx = EngineContext::new("ListShared", "L-.#####").with_user(&alice);

    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "shared row" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // Baseline: the list is empty for alice.
    let (rows, _) = engine
        .list(&alice_ctx, &ListQuery::default())
        .await
        .unwrap();
    assert!(rows.is_empty(), "alice starts with no visible documents");

    let shares = ShareManager::with_db(db.clone());
    shares.init().await.unwrap();
    shares
        .share(
            "ListShared",
            &name,
            "alice@local",
            vec![SharePermission::Read],
            "admin@local",
        )
        .await
        .unwrap();

    let (rows, total) = engine
        .list(&alice_ctx, &ListQuery::default())
        .await
        .unwrap();
    assert_eq!(total, 1, "the shared document is counted once");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("name").and_then(|v| v.as_str()),
        Some(name.as_str()),
        "the shared document is the one listed"
    );
}
