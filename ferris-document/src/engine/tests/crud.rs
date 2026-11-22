use super::*;

#[tokio::test]
async fn test_crud_with_naming_and_docstatus() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "Buy milk" }))
        .await
        .unwrap();
    assert_eq!(
        created.get("name").and_then(|v| v.as_str()),
        Some("TODO-00001")
    );
    assert_eq!(
        created.get("docstatus").and_then(|v| v.as_str()),
        Some("draft")
    );

    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    let updated = engine
        .update(
            &ctx,
            &name,
            ferris_core::value!({ "title": "Buy oat milk" }),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.get("title").and_then(|v| v.as_str()),
        Some("Buy oat milk")
    );

    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    let fetched = engine.get(&ctx, &name).await.unwrap();
    assert_eq!(
        fetched.get("docstatus").and_then(|v| v.as_str()),
        Some("submitted")
    );

    // A17: a submitted record only answers to the workflow. Deleting it
    // would destroy its audit trail, so the engine refuses until it is
    // cancelled (Frappe: "Submitted Record cannot be deleted...").
    let err = engine.delete(&ctx, &name).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("Submitted Record cannot be deleted"),
        "{err}"
    );
    // The row is untouched.
    assert!(engine.get(&ctx, &name).await.is_ok());

    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();
    engine.delete(&ctx, &name).await.unwrap();
    assert!(engine.get(&ctx, &name).await.is_err());
}

/// DOC4: a docstatus transition is a save, so it must stamp `modified` /
/// `modified_by` from the session — `set_docstatus` used to write the stale read
/// back verbatim, leaving `modified` at the value the *previous* save set.
#[tokio::test]
async fn set_docstatus_stamps_modified_and_modified_by() {
    let engine = engine().await;
    let admin = sysadmin();
    let ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);

    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "transition me" }))
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // Force a visible gap between the insert's stamp and the transition's, so
    // "stamped at all" and "re-stamped" cannot be confused.
    let before = engine.get(&ctx, &name).await.unwrap();
    let before_modified = before
        .get("modified")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let submitted = engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    assert_eq!(
        submitted.get("docstatus").and_then(|v| v.as_str()),
        Some("submitted")
    );
    assert_eq!(
        submitted.get("modified_by").and_then(|v| v.as_str()),
        Some("admin@local"),
        "modified_by must be stamped from the session on a transition"
    );
    // The transition writes a *fresh* `modified` (the insert's stamp is older or
    // equal, never newer).
    let after_modified = submitted.get("modified").and_then(|v| v.as_str()).unwrap();
    assert!(
        after_modified >= before_modified.as_str(),
        "modified must be re-stamped on a transition: {before_modified} -> {after_modified}"
    );
    assert_eq!(
        submitted.get("title").and_then(|v| v.as_str()),
        Some("transition me"),
        "a transition preserves the document body"
    );
}

/// `docstore::write-no-events`: a `suppress_events` context persists and
/// stamps the row normally, but the plugin-event forwarder is told to stay
/// silent — the same write without the flag forwards. This is the guard that
/// breaks the loop a plugin creates when it reacts to its own write.
#[tokio::test]
async fn suppress_events_skips_the_forwarder_but_keeps_the_write() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let engine = engine().await;
    let forwarded = Arc::new(AtomicUsize::new(0));
    let counter = forwarded.clone();
    // A unique doctype (only this test writes it) keeps the global registry
    // from leaking into the other tests running in parallel.
    HookRegistry::replace_global(
        "test-suppress-events",
        "SuppressProbe".to_string(),
        HookPoint::AfterInsert,
        Box::new(move |ctx: &mut HookContext, _doc: &mut dyn DocumentHook| {
            let counter = counter.clone();
            let suppressed = ctx.suppress_events();
            Box::pin(async move {
                if !suppressed {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })
        }),
        0,
        None,
    );

    // A normal write reaches the forwarder.
    let ctx = EngineContext::new("SuppressProbe", "PROBE-.#####").allow_anonymous(true);
    engine
        .create(&ctx, ferris_core::value!({ "title": "visible" }))
        .await
        .unwrap();
    assert_eq!(forwarded.load(Ordering::SeqCst), 1);

    // The silent write is still persisted and named, but does not forward.
    let quiet = EngineContext::new("SuppressProbe", "PROBE-.#####")
        .allow_anonymous(true)
        .suppress_events();
    let created = engine
        .create(&quiet, ferris_core::value!({ "title": "quiet" }))
        .await
        .unwrap();
    assert!(
        created.get("name").and_then(|v| v.as_str()).is_some(),
        "the suppressed write is persisted with a name"
    );
    assert_eq!(
        forwarded.load(Ordering::SeqCst),
        1,
        "a suppressed write must not reach the plugin-event forwarder"
    );
}

/// A plugin that answers `AfterInsert` by writing the row again — exactly what
/// `SolmecBox`'s `dashlink` does through `docstore::write-no-events` when it seals
/// the `dashboard` Link — must see that write survive. The insert path used to
/// run the after-hooks and *then* persist its own document over the row, so the
/// nested write was clobbered and the freshly sealed Link disappeared (the row
/// then opened in the form view instead of its dashboard panel). The update
/// path persists *before* `AfterSave`, which is why a later save on the same
/// row did seal it: the two paths disagreed.
#[tokio::test]
async fn nested_write_from_an_after_insert_hook_is_not_clobbered() {
    // The outer insert now holds one connection for document + version, and
    // the AfterInsert hook re-enters with a second write. A one-connection
    // `:memory:` pool would deadlock on itself (and an in-memory SQLite cannot
    // share data across connections), so this test uses a small file pool.
    let path = std::env::temp_dir().join(format!(
        "ferris-nested-after-insert-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let db = ferris_db::open_unmanaged(&ferris_core::config::DatabaseConfig {
        r#type: ferris_core::config::DatabaseType::SQLite,
        name: path.to_string_lossy().into_owned(),
        max_connections: 2,
        min_connections: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    let engine = DocumentEngine::new(db.clone());

    // The hook re-enters the engine the way `dashlink::link` does: one write on
    // the same row, with the forwarder suppressed so the plugin cannot loop.
    HookRegistry::replace_global(
        "test-nested-after-insert",
        "NestedSealProbe".to_string(),
        HookPoint::AfterInsert,
        Box::new(
            move |hook_ctx: &mut HookContext, _doc: &mut dyn DocumentHook| {
                let db = db.clone();
                let docname = hook_ctx.docname.clone();
                Box::pin(async move {
                    let nested = DocumentEngine::new(db);
                    let quiet = EngineContext::new("NestedSealProbe", "SEAL-.#####")
                        .allow_anonymous(true)
                        .suppress_events();
                    nested
                        .update(&quiet, &docname, ferris_core::value!({ "sealed": "sbx-1" }))
                        .await?;
                    Ok(())
                })
            },
        ),
        0,
        None,
    );

    let ctx = EngineContext::new("NestedSealProbe", "SEAL-.#####").allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "probe" }))
        .await
        .unwrap();
    let name = created
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let stored = engine.get(&ctx, &name).await.unwrap();
    assert_eq!(
        stored.get("sealed").and_then(|v| v.as_str()),
        Some("sbx-1"),
        "the AfterInsert hook's nested write must survive the insert's trailing persist"
    );

    // The removed trailing `UPDATE` was also what made a freshly created blob
    // canonical (sorted keys); the INSERT has to carry that on its own, or the
    // plugin's event/read-back comparison ("stored data differs from the data I
    // was given") breaks again.
    let row = engine
        .db()
        .fetch_one(&Stmt::new(
            "SELECT data FROM tabNestedSealProbe WHERE name = ?",
            vec![Value::from(name.clone())],
        ))
        .await
        .unwrap();
    let blob = row.str("data").unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&blob).unwrap();
    assert_eq!(
        blob,
        serde_json::to_string(&reparsed).unwrap(),
        "the stored blob must be canonical: re-serializing it reorders nothing"
    );
    let _ = std::fs::remove_file(&path);
}

/// A17: deleting a document also removes every related row that keyed on
/// `(doctype, docname)` — share, version, comment, like — inside the same
/// transaction. Orphan rows here would keep `get_shares`/`get_versions`/
/// `get_comments` answering for a document that no longer exists.
#[tokio::test]
async fn test_delete_cascades_related_rows() {
    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    let name = engine
        .create(&ctx, ferris_core::value!({ "title": "Buy milk" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Seed the related tables exactly the way the feature stores do.
    let share = crate::sharing::ShareManager::with_db(db.clone());
    share.init().await.unwrap();
    share
        .share(
            &ctx.doctype,
            &name,
            "reader-1",
            vec![crate::sharing::SharePermission::Read],
            "admin",
        )
        .await
        .unwrap();

    let versions = crate::versioning::VersionManager::with_db(db.clone());
    versions.init().await.unwrap();
    versions
        .create_version(
            &ctx.doctype,
            &name,
            HashMap::from([("title".to_string(), Value::from("Buy oat milk"))]),
            "admin".to_string(),
            crate::versioning::ChangeType::Update,
            None,
        )
        .await
        .unwrap();

    let likes = crate::engagement::LikeStore::new(db.clone());
    likes.toggle(&ctx.doctype, &name, "reader-1").await.unwrap();

    let comments = crate::engagement::CommentStore::new(db.clone());
    comments
        .add(&ctx.doctype, &name, "reader-1", "pinned", None)
        .await
        .unwrap();

    engine.delete(&ctx, &name).await.unwrap();

    for side in ["tab_share", "tab_comment", "tab_like"] {
        let side_table = q(db.dialect(), side).unwrap();
        let row = db
            .fetch_one(&Stmt::new(
                format!("SELECT COUNT(*) AS n FROM {side_table} WHERE doctype = ? AND docname = ?"),
                vec![Value::from(ctx.doctype.clone()), Value::from(name.as_str())],
            ))
            .await
            .unwrap();
        assert_eq!(
            row.i64("n").unwrap(),
            0,
            "delete cascade missed {side} rows for {name}"
        );
    }
    // `tab_version` is cascaded too, and then the engine writes exactly one
    // tombstone on the same transaction so a re-created name keeps a timeline.
    let versions = versions.get_versions(&ctx.doctype, &name).await.unwrap();
    assert_eq!(versions.len(), 1, "one delete tombstone: {versions:?}");
    assert_eq!(
        versions[0].change_type,
        crate::versioning::ChangeType::Delete
    );
}

/// `docstore::apply`: an insert, an update and a delete across different
/// DocTypes commit as one transaction; the returned list is exactly the
/// documents written (deletes contribute none), in op order.
#[tokio::test]
async fn apply_commits_a_multi_doctype_batch() {
    let engine = engine().await;
    let todo = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);
    let note = EngineContext::new("Note", "NOTE-.#####").allow_anonymous(true);

    // Seed the row to update and the row to delete.
    let updated_name = engine
        .create(&todo, ferris_core::value!({ "title": "before" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let deleted_name = engine
        .create(&note, ferris_core::value!({ "title": "bye" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let written = engine
        .apply(vec![
            // op 0 -- upsert that inserts (mints a name).
            ApplyOp {
                ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
                op: ApplyKind::Upsert(ferris_core::value!({ "title": "fresh" })),
            },
            // op 1 -- upsert that updates the seeded row.
            ApplyOp {
                ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
                op: ApplyKind::Upsert(ferris_core::value!({
                    "name": (updated_name.clone()),
                    "title": "after",
                })),
            },
            // op 2 -- plain update of the Note.
            ApplyOp {
                ctx: EngineContext::new("Note", "NOTE-.#####").allow_anonymous(true),
                op: ApplyKind::Update {
                    name: deleted_name.clone(),
                    data: ferris_core::value!({ "title": "edited" }),
                },
            },
            // op 3 -- delete it; contributes no document.
            ApplyOp {
                ctx: EngineContext::new("Note", "NOTE-.#####").allow_anonymous(true),
                op: ApplyKind::Delete {
                    name: deleted_name.clone(),
                },
            },
        ])
        .await
        .unwrap();

    // Three writes produced a document; the delete produced none.
    assert_eq!(written.len(), 3, "{written:?}");
    assert_eq!(
        written[0].get("title").and_then(|v| v.as_str()),
        Some("fresh")
    );
    assert_eq!(
        written[1].get("name").and_then(|v| v.as_str()),
        Some(updated_name.as_str())
    );
    assert_eq!(
        written[1].get("title").and_then(|v| v.as_str()),
        Some("after")
    );
    assert_eq!(
        written[2].get("name").and_then(|v| v.as_str()),
        Some(deleted_name.as_str())
    );

    // The batch is durable: the update stuck, the delete committed.
    let stored = engine.get(&todo, &updated_name).await.unwrap();
    assert_eq!(stored.get("title").and_then(|v| v.as_str()), Some("after"));
    assert!(matches!(
        engine.get(&note, &deleted_name).await.unwrap_err(),
        Error::Document(DocumentError::NotFound { .. })
    ));
}

/// `docstore::apply` is all-or-nothing: one failing op (an `update` of a
/// row that does not exist) unwinds the writes that already succeeded.
#[tokio::test]
async fn apply_rolls_back_the_whole_batch_on_a_failing_op() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    let err = engine
        .apply(vec![
            ApplyOp {
                ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
                op: ApplyKind::Upsert(ferris_core::value!({ "title": "rolled back" })),
            },
            ApplyOp {
                ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
                op: ApplyKind::Update {
                    name: "TODO-99999".to_string(),
                    data: ferris_core::value!({ "title": "nope" }),
                },
            },
        ])
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Document(DocumentError::NotFound { .. })),
        "{err}"
    );

    // The insert that ran before the failure left nothing behind.
    let (documents, total) = engine
        .list(&ctx, &crate::ListQuery::default())
        .await
        .unwrap();
    assert!(
        documents.is_empty() && total == 0,
        "a rolled-back batch must not persist: {documents:?}"
    );
}

/// DOC1: a naming-series mint inside `apply`'s transaction must run on the
/// transaction's own connection, not open a second one.
///
/// The pool here has a single connection (`memory_db`), so the old
/// implementation — `apply` opens a transaction, then `mint_name` calls
/// `NamingSeriesStore::next(&db, …)` which begins a *second* transaction — could
/// never acquire the second connection: `apply_upsert` hung on `PoolExhausted`.
/// This asserts the batch completes, and that a rolled-back batch does not burn
/// a series number either (the same `db.begin()` used to commit the counter
/// outside the batch's rollback).
#[tokio::test]
async fn apply_mints_a_naming_series_inside_its_own_transaction() {
    let engine = engine().await;
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    // A rolled-back batch must not consume a number: `TODO-99999` does not
    // exist, so the update fails and the whole batch unwinds.
    let err = engine
        .apply(vec![
            ApplyOp {
                ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
                op: ApplyKind::Upsert(ferris_core::value!({ "title": "burned" })),
            },
            ApplyOp {
                ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
                op: ApplyKind::Update {
                    name: "TODO-99999".to_string(),
                    data: ferris_core::value!({ "title": "nope" }),
                },
            },
        ])
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Document(DocumentError::NotFound { .. })),
        "{err}"
    );

    // The next successful insert draws the *first* number, proving the rolled
    // back mint was undone with the rest of the batch.
    let written = engine
        .apply(vec![ApplyOp {
            ctx: EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true),
            op: ApplyKind::Upsert(ferris_core::value!({ "title": "first" })),
        }])
        .await
        .unwrap();
    assert_eq!(
        written[0].get("name").and_then(|v| v.as_str()),
        Some("TODO-00001"),
        "a rolled-back batch must not consume a naming-series number"
    );

    let stored = engine.get(&ctx, "TODO-00001").await.unwrap();
    assert_eq!(stored.get("title").and_then(|v| v.as_str()), Some("first"));
}

/// Authorization runs **before** the lifecycle hooks.
///
/// `before_validate` used to run before `authorize`, so a caller who could not
/// write made the hook run (hooks have side effects: notifications, nested
/// writes, API calls) and could tell a validation failure (400) from a refusal
/// (403) by shaping the payload. A unique doctype keeps the global hook registry
/// out of the other tests running in parallel.
#[tokio::test]
async fn an_unauthorized_create_does_not_run_lifecycle_hooks() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let engine = engine().await;
    let ran = Arc::new(AtomicUsize::new(0));
    let counter = ran.clone();
    HookRegistry::replace_global(
        "test-authorize-before-hook-insert",
        "AuthorizeFirstProbe".to_string(),
        HookPoint::BeforeValidate,
        Box::new(move |_ctx: &mut HookContext, _doc: &mut dyn DocumentHook| {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
        0,
        None,
    );

    // A registered user with no matching grant: refused, and the hook never ran.
    let alice = plain_user();
    let denied_ctx = EngineContext::new("AuthorizeFirstProbe", "AFP-.#####").with_user(&alice);
    let err = engine
        .create(&denied_ctx, ferris_core::value!({ "title": "loot" }))
        .await
        .unwrap_err();
    assert!(denied(&err), "{err:?}");
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "an unauthorized create must not reach `before_validate`"
    );

    // An authorized caller still runs it, and the write succeeds.
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("AuthorizeFirstProbe", "AFP-.#####").with_user(&admin);
    engine
        .create(&admin_ctx, ferris_core::value!({ "title": "ok" }))
        .await
        .unwrap();
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

/// The same rule on the update path, stated as the oracle it closes: the
/// unauthorized caller gets 403 even when the hook would have answered 400.
#[tokio::test]
async fn an_unauthorized_update_is_refused_before_the_hook_can_reject_the_body() {
    let engine = engine().await;
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("AuthorizeFirstUpdateProbe", "AFU-.#####").with_user(&admin);
    // Seed first: the hook below rejects *every* `before_validate`, and this is
    // the update path's test.
    let created = engine
        .create(&admin_ctx, ferris_core::value!({ "title": "seed" }))
        .await
        .unwrap();
    let name = created
        .get("name")
        .and_then(|v| v.as_str())
        .expect("a name")
        .to_string();

    HookRegistry::replace_global(
        "test-authorize-before-hook-update",
        "AuthorizeFirstUpdateProbe".to_string(),
        HookPoint::BeforeValidate,
        Box::new(|_ctx: &mut HookContext, _doc: &mut dyn DocumentHook| {
            Box::pin(async move {
                Err(Error::Validation(ValidationError::Invalid(
                    "the hook ran".to_string(),
                )))
            })
        }),
        0,
        None,
    );

    // Unauthorized: 403, not the hook's 400.
    let alice = plain_user();
    let denied_ctx =
        EngineContext::new("AuthorizeFirstUpdateProbe", "AFU-.#####").with_user(&alice);
    let err = engine
        .update(
            &denied_ctx,
            &name,
            ferris_core::value!({ "title": "changed" }),
        )
        .await
        .unwrap_err();
    assert!(
        denied(&err),
        "an unauthorized update must be refused, got {err:?}"
    );

    // Authorized: the hook runs, so its validation error is what surfaces —
    // the Frappe order (before_validate → validate) is untouched for callers
    // who are allowed.
    let err = engine
        .update(
            &admin_ctx,
            &name,
            ferris_core::value!({ "title": "changed" }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Validation(ValidationError::Invalid(ref m)) if m.contains("the hook ran")),
        "expected the hook's validation error, got {err:?}"
    );
}
