//! Trash, restore and retention tests (`SEC-07`/`DX-01`).

use super::*;

/// SEC-07/DX-01: cancelling a document and deleting it keeps a trash row with
/// the complete snapshot, and the version chain survives (it is no longer
/// cascaded for a cancelled record).
#[tokio::test]
async fn cancelled_delete_leaves_a_tombstone_and_keeps_history() {
    use crate::versioning::{ChangeType, VersionManager};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let admin = sysadmin();
    let ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);

    let name = engine
        .create(&ctx, ferris_core::value!({ "title": "evidence" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();
    engine.delete(&ctx, &name).await.unwrap();

    assert!(
        engine.get(&ctx, &name).await.is_err(),
        "the live row is gone"
    );
    let trash = engine.deleted_documents(&ctx.doctype, &name).await.unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].deleted_by, "admin@local");
    assert_eq!(
        trash[0].data.get("title").and_then(|v| v.as_str()),
        Some("evidence"),
        "the tombstone keeps the whole document"
    );

    let manager = VersionManager::with_db(db);
    let versions = manager.get_versions(&ctx.doctype, &name).await.unwrap();
    assert_eq!(
        versions.iter().map(|v| v.change_type).collect::<Vec<_>>(),
        vec![
            ChangeType::Create,
            ChangeType::Submit,
            ChangeType::Cancel,
            ChangeType::Delete
        ],
        "the audit chain survives a soft delete"
    );
}

/// SEC-07/DX-01: restoring a tombstone reinserts the original name and appends
/// a `Restore` version; the token works once and the name must be free.
#[tokio::test]
async fn restore_reinserts_the_snapshot_and_records_the_restore() {
    use crate::versioning::{ChangeType, VersionManager};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let admin = sysadmin();
    let ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);

    let name = engine
        .create(&ctx, ferris_core::value!({ "title": "round trip" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();
    engine
        .delete_with(&ctx, &name, Some("duplicate invoice"))
        .await
        .unwrap();

    let trash = engine.deleted_documents(&ctx.doctype, &name).await.unwrap();
    assert_eq!(trash[0].reason.as_deref(), Some("duplicate invoice"));
    let token = trash[0].restore_token.clone();

    let restored = engine.restore(&ctx, &token).await.unwrap();
    assert_eq!(
        restored.get("title").and_then(|v| v.as_str()),
        Some("round trip")
    );
    assert_eq!(
        restored.get("docstatus").and_then(|v| v.as_str()),
        Some("cancelled"),
        "the restored row keeps the state the snapshot held"
    );
    assert_eq!(
        engine
            .get(&ctx, &name)
            .await
            .unwrap()
            .get("title")
            .and_then(|v| v.as_str()),
        Some("round trip")
    );
    let trash = engine.deleted_documents(&ctx.doctype, &name).await.unwrap();
    assert!(trash[0].restored_at.is_some());

    let manager = VersionManager::with_db(db);
    let versions = manager.get_versions(&ctx.doctype, &name).await.unwrap();
    assert_eq!(versions.last().unwrap().change_type, ChangeType::Restore);
    let err = engine.restore(&ctx, &token).await.unwrap_err();
    assert!(err.to_string().contains("already restored"), "{err}");
}

/// SEC-07/DX-01: the explicit purge requires a reason and records the actor and
/// the reason on the surviving version chain.
#[tokio::test]
async fn purge_requires_a_reason_and_audits_the_destruction() {
    use crate::versioning::{ChangeType, VersionManager};

    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let admin = sysadmin();
    let ctx = EngineContext::new("ToDo", "TODO-.#####").with_user(&admin);

    let name = engine
        .create(&ctx, ferris_core::value!({ "title": "to purge" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();

    let err = engine.purge(&ctx, &name, "  ").await.unwrap_err();
    assert!(err.to_string().contains("requires a reason"), "{err}");
    assert!(
        engine.get(&ctx, &name).await.is_ok(),
        "nothing was destroyed"
    );

    engine.purge(&ctx, &name, "court order 42").await.unwrap();
    assert!(engine.get(&ctx, &name).await.is_err());
    assert!(
        engine
            .deleted_documents(&ctx.doctype, &name)
            .await
            .unwrap()
            .is_empty(),
        "a purge also removes the trash row"
    );

    let manager = VersionManager::with_db(db);
    let versions = manager.get_versions(&ctx.doctype, &name).await.unwrap();
    let marker = versions.last().unwrap();
    assert_eq!(marker.change_type, ChangeType::Delete);
    assert_eq!(marker.changed_by, "admin@local");
    assert!(
        marker
            .comment
            .as_deref()
            .is_some_and(|c| c.contains("court order 42")),
        "the reason survives on the chain: {marker:?}"
    );
}

/// SEC-07/DX-01: retention keeps a tombstone until the window elapses and a
/// legal hold disables the automatic purge.
#[tokio::test]
async fn retention_does_not_purge_before_the_window() {
    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    register_common_doctypes();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new("ToDo", "TODO-.#####").allow_anonymous(true);

    let name = engine
        .create(&ctx, ferris_core::value!({ "title": "kept" }))
        .await
        .unwrap()
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();
    engine.delete(&ctx, &name).await.unwrap();

    assert_eq!(engine.purge_expired(30, false).await.unwrap(), 0);
    assert_eq!(engine.purge_expired(1, true).await.unwrap(), 0);
    assert_eq!(
        engine
            .deleted_documents(&ctx.doctype, &name)
            .await
            .unwrap()
            .len(),
        1,
        "a legal hold never purges"
    );

    // Backdate the tombstone past the window: only then does the sweep remove
    // the trash row, and the version chain is still there.
    let trash = q(db.dialect(), "tabDeletedDocument").unwrap();
    db.execute(&Stmt::new(
        format!("UPDATE {trash} SET deleted_at = ? WHERE docname = ?"),
        vec![
            Value::from("2020-01-01T00:00:00+00:00"),
            Value::from(name.as_str()),
        ],
    ))
    .await
    .unwrap();
    assert_eq!(engine.purge_expired(30, false).await.unwrap(), 1);
    assert!(
        engine
            .deleted_documents(&ctx.doctype, &name)
            .await
            .unwrap()
            .is_empty()
    );
    let manager = crate::versioning::VersionManager::with_db(db);
    assert_eq!(
        manager
            .get_versions(&ctx.doctype, &name)
            .await
            .unwrap()
            .len(),
        4,
        "expiring the trash keeps the audit chain"
    );
}
