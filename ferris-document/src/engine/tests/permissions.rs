use super::*;

#[tokio::test]
async fn test_anonymous_is_denied_by_default() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####");

    let err = engine
        .create(&ctx, ferris_core::value!({ "title": "loot" }))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
    ));

    let anonymous_ok = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    engine
        .create(&anonymous_ok, ferris_core::value!({ "title": "ok" }))
        .await
        .unwrap();
}

/// The permission matrix, including the two cases that used to fail.
///
/// This test previously asserted the opposite of the fix: it created a
/// document as `admin@local` while passing `{"owner": "alice@local"}`, and
/// then commented *"the document is attributed to the caller"* as if that
/// were correct. It was the bug, not the policy — see §C1. The two
/// `owner`-injection cases below are the regression tests for it.
#[tokio::test]
async fn test_permissions_owner_matrix() {
    let engine = engine().await;
    let admin = sysadmin();
    let alice = plain_user();

    // Unauthorised create is denied (default-deny), even for an otherwise
    // registered user without any matching policy.
    let alice_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&alice);
    let err = engine
        .create(&alice_ctx, ferris_core::value!({ "title": "nope" }))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
    ));

    // System Manager bypasses, and the document is attributed to the caller.
    let admin_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);
    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "mine" }))
        .await
        .unwrap();
    assert_eq!(
        created.get("owner").and_then(|v| v.as_str()),
        Some("admin@local"),
        "owner must come from the session, not the body"
    );
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // --- Regression: C1, create half -----------------------------------
    // A caller may not choose the owner of the row it creates. Handing
    // ownership to a third party would grant them Read|Write|Amend over a
    // document they never had access to.
    let err = engine
        .create(
            &admin_ctx,
            ferris_core::value!({ "title": "gift", "owner": "alice@local" }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Validation(ValidationError::Invalid(ref m)) if m.contains("owner")),
        "expected BadRequest about `owner`, got {err:?}"
    );

    // --- Regression: C1, update half -----------------------------------
    // The actual escalation: before the fix, `update` authorised against
    // the *merged* map, so `{"owner": "alice@local"}` satisfied the owner
    // shortcut in `ferris_core::permissions::check_permission` and granted
    // Write on a document alice did not own — and then persisted the
    // takeover. Alice must still be refused, and the stored owner must not
    // move.
    let err = engine
        .update(
            &alice_ctx,
            &name,
            ferris_core::value!({ "title": "hijacked", "owner": "alice@local" }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Validation(ValidationError::Invalid(ref m)) if m.contains("owner")),
        "expected BadRequest about `owner`, got {err:?}"
    );
    // Same attack without the smuggling of `owner` in the *body* of an
    // update that would otherwise succeed for the real owner: the row is
    // still admin's, so alice is refused.
    let err = engine
        .update(
            &alice_ctx,
            &name,
            ferris_core::value!({ "title": "hijacked" }),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
    ));
    let unchanged = engine.get(&admin_ctx, &name).await.unwrap();
    assert_eq!(
        unchanged.get("owner").and_then(|v| v.as_str()),
        Some("admin@local"),
        "a refused update must not have moved the owner"
    );
    assert_eq!(
        unchanged.get("title").and_then(|v| v.as_str()),
        Some("mine"),
        "a refused update must not have written anything"
    );

    // The real owner may write their own document, and `modified` /
    // `modified_by` are stamped from the session.
    let updated = engine
        .update(
            &admin_ctx,
            &name,
            ferris_core::value!({ "title": "edited" }),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.get("title").and_then(|v| v.as_str()),
        Some("edited")
    );
    assert_eq!(
        updated.get("owner").and_then(|v| v.as_str()),
        Some("admin@local"),
        "owner must survive an update"
    );
    assert_eq!(
        updated.get("modified_by").and_then(|v| v.as_str()),
        Some("admin@local"),
        "modified_by must be stamped from the session"
    );

    // ...but never delete it; a System Manager may.
    let err = engine.delete(&admin_ctx, &name).await;
    assert!(err.is_ok(), "System Manager may delete: {err:?}");
    assert!(engine.get(&admin_ctx, &name).await.is_err());
}

/// `owner` is the engine's, not the caller's — for owners too.
///
/// The owner shortcut in `ferris_core::permissions` grants
/// `Read|Write|Amend` when `user.id == resource.owner`. An owner who could
/// also *reassign* ownership could hand their own document to a third
/// party in a single request, which is not something the permission model
/// describes.
#[tokio::test]
async fn owner_cannot_reassign_ownership() {
    let engine = engine().await;
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);

    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "mine" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    let err = engine
        .update(
            &admin_ctx,
            &name,
            ferris_core::value!({ "owner": "alice@local" }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Validation(ValidationError::Invalid(ref m)) if m.contains("owner")),
        "even the owner must not reassign ownership: {err:?}"
    );
    assert_eq!(
        engine
            .get(&admin_ctx, &name)
            .await
            .unwrap()
            .get("owner")
            .and_then(|v| v.as_str()),
        Some("admin@local")
    );
}

/// An unresolved owner stays unresolved — §C1, the second bypass.
///
/// `insert()` used to stamp the literal `"Administrator"` when the owner
/// resolved to nothing. `User::new` mints a UUID for `id` and never pins it
/// to the email, so any context built without a coherent session id ended up
/// owning a document as the system account, and the owner shortcut in
/// `ferris_core::permissions::check_permission` compares `user.id` against
/// it — handing `Read|Write|Amend` to whoever's id was that string.
///
/// It is the same defect as C1 proper: a privileged value written at insert
/// time, outside any authorization decision. The value only became reachable
/// once C1 forced `owner` to come from the session, which is why it surfaced
/// as a failing assertion while that fix was being written.
#[tokio::test]
async fn an_unresolved_owner_is_not_stamped_as_a_privileged_default() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "public" }))
        .await
        .unwrap();
    assert!(
        created.get("owner").is_none(),
        "an insert with no session must leave `owner` unset, got {:?}",
        created.get("owner")
    );
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // The stored owner is nobody, so nobody may reach the row by claiming to
    // be whoever the default used to name. Without the fix this read
    // succeeded: the row's owner column was `"Administrator"`.
    let mut named = plain_user();
    named.id = "Administrator".to_string();
    let as_named = EngineContext::new("ToDo", "TODO-.#####").with_user(&named);
    let err = engine.get(&as_named, &name).await.unwrap_err();
    assert!(
        denied(&err),
        "a user id equal to the old default must not own the row: {err:?}"
    );
}

#[tokio::test]
async fn reads_require_permission() {
    let engine = engine().await;
    let admin = sysadmin();
    let alice = plain_user();
    let admin_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);
    let alice_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&alice);

    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "admin's" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // A third party cannot read it. Before the fix this returned the
    // document verbatim, which is the IDOR.
    let err = engine.get(&alice_ctx, &name).await.unwrap_err();
    assert!(
        denied(&err),
        "alice must not read admin's document: {err:?}"
    );

    // The owner reads it.
    assert_eq!(
        engine
            .get(&admin_ctx, &name)
            .await
            .unwrap()
            .get("title")
            .and_then(|v| v.as_str()),
        Some("admin's")
    );

    // A System Manager reads it.
    let mut boss = sysadmin();
    boss.id = "boss@local".to_string();
    let boss_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&boss);
    assert!(engine.get(&boss_ctx, &name).await.is_ok());
}

#[tokio::test]
async fn anonymous_reads_are_denied_unless_opted_in() {
    let engine = engine().await;
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);

    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "secret" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // No user, no opt-in: fail closed, like the write path.
    let anon = EngineContext::new("ToDo", "TODO-.#####");
    assert!(denied(&engine.get(&anon, &name).await.unwrap_err()));
    let (_, total) = engine
        .list(&anon, &crate::ListQuery::default())
        .await
        .unwrap();
    assert_eq!(
        total, 0,
        "anonymous list must be empty, not the whole table"
    );

    // Explicit opt-in still works — this is what the website/SSR path and
    // the engine's own tests rely on.
    let open = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    assert!(engine.get(&open, &name).await.is_ok());
}

#[tokio::test]
async fn get_for_requires_the_named_action_not_just_read() {
    let engine = engine().await;
    let admin = sysadmin();
    let alice = plain_user();
    let admin_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);
    let alice_ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&alice);

    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "submitted" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    engine
        .set_docstatus(&admin_ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();

    // `Read` lets Alice see it; `Amend` does not, even though she can read.
    // `amend` clones the document, so gating it on `Read` alone would hand
    // every reader a copy.
    assert!(engine.get(&alice_ctx, &name).await.is_err());
    let err = engine
        .get_for(&alice_ctx, &name, Permission::Amend)
        .await
        .unwrap_err();
    assert!(denied(&err), "reading is not amending: {err:?}");

    // The owner may amend, via the owner shortcut.
    assert!(
        engine
            .get_for(&admin_ctx, &name, Permission::Amend)
            .await
            .is_ok()
    );
}

/// §C5: `docstatus` is a workflow state, not a payload field. A fresh
/// document must enter as `draft` — posting `submitted` (or `cancelled`)
/// in the insert body used to mint an already-submitted row with no
/// permission check at all.
#[tokio::test]
async fn insert_cannot_create_a_submitted_document() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    for docstatus in ["submitted", "cancelled"] {
        let err = engine
            .create(
                &ctx,
                ferris_core::value!(serde_json::json!({
                    "title": "x",
                    "docstatus": docstatus,
                })),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("docstatus"),
            "insert with {docstatus} must be rejected: {err:?}"
        );
    }

    // The word `draft` is the no-op it always was, and the numeric `0` is
    // what the amend copy (`DocStore::amend`) ships while clearing the
    // engine-owned fields.
    engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "ok", "docstatus": "draft" }),
        )
        .await
        .unwrap();
    engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "ok2", "docstatus": 0 }),
        )
        .await
        .unwrap();
}

/// §C5: the ordinary update path cannot touch `docstatus` — that is what
/// `set_docstatus` (and `can_transition_to`) is for. Rewriting a submitted
/// row's `docstatus` back to `draft` used to be a plain `update`, silently
/// bypassing `Amend` and the transition table.
#[tokio::test]
async fn update_cannot_tamper_docstatus() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "s" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();

    let err = engine
        .update(&ctx, &name, ferris_core::value!({ "docstatus": "draft" }))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("docstatus"),
        "submitted -> draft must go through amend: {err:?}"
    );
    let err = engine
        .update(
            &ctx,
            &name,
            ferris_core::value!({ "docstatus": "cancelled" }),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("docstatus"),
        "submitted -> cancelled must go through cancel: {err:?}"
    );

    // Sending the stored value is a no-op and stays legal.
    let ok = engine
        .update(
            &ctx,
            &name,
            ferris_core::value!({ "docstatus": "submitted" }),
        )
        .await
        .unwrap();
    assert_eq!(
        ok.get("docstatus").and_then(|v| v.as_str()),
        Some("submitted")
    );
}

/// §C5: after submission a document is locked — a field may change only
/// if the doctype marks it `allow_on_submit` (Frappe's
/// `_validate_update_after_submit`). Cancelling locks everything: an
/// in-place edit must never reopen a cancelled document.
#[tokio::test]
async fn update_after_submit_respects_allow_on_submit() {
    let engine = engine().await;
    let meta = ferris_schema::DoctypeMeta {
        name: "ToDo".to_string(),
        is_submittable: true,
        fields: vec![
            ferris_schema::FieldMeta {
                fieldname: "title".to_string(),
                fieldtype: "Data".to_string(),
                ..Default::default()
            },
            ferris_schema::FieldMeta {
                fieldname: "status".to_string(),
                fieldtype: "Data".to_string(),
                ..Default::default()
            },
            ferris_schema::FieldMeta {
                fieldname: "paid_amount".to_string(),
                fieldtype: "Currency".to_string(),
                allow_on_submit: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let ctx = EngineContext::new("ToDo", "TODO-.#####")
        .with_meta(&meta)
        .allow_anonymous(true);
    let created = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "t",
                "status": "draft",
                "paid_amount": 0,
            }),
        )
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();

    let err = engine
        .update(&ctx, &name, ferris_core::value!({ "status": "paid" }))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("status") && err.to_string().contains("submit"),
        "locked field must be called out: {err:?}"
    );
    let err = engine
        .update(&ctx, &name, ferris_core::value!({ "title": "new" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("title"), "{err:?}");

    // `allow_on_submit` survives submission.
    engine
        .update(&ctx, &name, ferris_core::value!({ "paid_amount": 100 }))
        .await
        .unwrap();
    let fetched = engine.get(&ctx, &name).await.unwrap();
    assert_eq!(
        fetched.get("paid_amount").and_then(|v| v.as_i64()),
        Some(100)
    );

    // Cancelling locks everything, `allow_on_submit` included — reopening
    // is the amend (copy + create) flow.
    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();
    let err = engine
        .update(&ctx, &name, ferris_core::value!({ "paid_amount": 50 }))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("paid_amount") && err.to_string().contains("cancelled"),
        "cancelled rows are fully locked: {err:?}"
    );
}

/// §C5 conservative default: without doctype metadata no field can prove
/// `allow_on_submit`, so every change to a submitted row is rejected
/// rather than quietly reopening one.
#[tokio::test]
async fn update_after_submit_without_metadata_locks_every_field() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "t" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();

    let err = engine
        .update(&ctx, &name, ferris_core::value!({ "title": "nope" }))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("title"),
        "no metadata, no free edits on a submitted row: {err:?}"
    );

    // Drafts remain fully editable even without metadata.
    let draft = engine
        .create(&ctx, ferris_core::value!({ "title": "d" }))
        .await
        .unwrap();
    let draft_name = draft.get("name").unwrap().as_str().unwrap().to_string();
    engine
        .update(&ctx, &draft_name, ferris_core::value!({ "title": "d2" }))
        .await
        .unwrap();
}

/// `User Permission` narrows writes, not only reads (FRA-03, write half).
///
/// Before this test the owner shortcut admitted a write on any row the user
/// owned, and `delete` never looked at the stored document at all, so an
/// operator restricting a user to `CUST-A` was protecting only the list view.
#[tokio::test]
async fn a_user_permission_narrows_update_and_delete() {
    use crate::engine::tests::user_permission_source::install;
    use ferris_core::permissions::{
        PermissionEngine, Policy, PolicyStatement, Principal, Resource, UserPermissionRule,
        global_engine_ref, set_global_engine,
    };
    use std::sync::{Mutex, MutexGuard};

    struct Restore {
        _serialised: MutexGuard<'static, ()>,
        previous: PermissionEngine,
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            set_global_engine(self.previous.clone());
        }
    }

    static POLICY_SERIALISE: Mutex<()> = Mutex::new(());

    fn allow_delete(doctype: &str, role: &str) -> Restore {
        let serialised = POLICY_SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = global_engine_ref()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut policy = PermissionEngine::default();
        policy.add_policy(Policy::new(vec![PolicyStatement::allow(
            vec![Principal::role(role)],
            vec![Permission::Delete],
            vec![Resource::new(doctype, "*")],
        )]));
        set_global_engine(policy);
        Restore {
            _serialised: serialised,
            previous,
        }
    }

    let meta = DoctypeMeta {
        name: "D1Customer".to_string(),
        fields: vec![field("title", "Data", 0)],
        ..Default::default()
    };
    register_common_doctypes();
    let engine = DocumentEngine::new(ferris_db::memory_db().await.unwrap());
    let create_ctx = EngineContext::new("D1Customer", "")
        .with_meta(&meta)
        .with_explicit_name()
        .allow_anonymous(true);
    for (name, title) in [("CUST-A", "a"), ("CUST-B", "b")] {
        engine
            .create_owned_by(
                &create_ctx,
                "ada@local",
                ferris_core::value!({ "name": name, "title": title }),
            )
            .await
            .unwrap();
    }

    let mut user = ferris_core::User::system_user("ada@local".to_string(), "Ada".to_string());
    user.id = "ada@local".to_string();
    user.roles = vec!["D1 Reader".to_string()];
    let ctx = EngineContext::new("D1Customer", "")
        .with_meta(&meta)
        .with_user(&user);

    let _rules = install(vec![UserPermissionRule::new(
        "ada@local",
        "D1Customer",
        "CUST-A",
    )]);

    // The owner shortcut would grant Write on CUST-B; the row rule must win.
    let error = engine
        .update(&ctx, "CUST-B", ferris_core::value!({ "title": "changed" }))
        .await
        .unwrap_err();
    assert!(
        denied(&error),
        "update of a forbidden row must be denied: {error:?}"
    );
    let untouched = engine.get(&create_ctx, "CUST-B").await.unwrap();
    assert_eq!(untouched.get("title").and_then(Value::as_str), Some("b"));

    engine
        .update(&ctx, "CUST-A", ferris_core::value!({ "title": "ok" }))
        .await
        .unwrap();

    // Delete needs its own grant; the owner shortcut does not include it.
    let _policy = allow_delete("D1Customer", "D1 Reader");
    let error = engine.delete(&ctx, "CUST-B").await.unwrap_err();
    assert!(
        denied(&error),
        "delete of a forbidden row must be denied: {error:?}"
    );
    engine.delete(&ctx, "CUST-A").await.unwrap();
}
