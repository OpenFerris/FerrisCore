use super::*;

#[tokio::test]
async fn single_doctype_has_one_document_and_upserts_in_place() {
    let engine = engine().await;
    let meta = DoctypeMeta {
        name: "Print Settings".to_string(),
        module: "Printing".to_string(),
        is_single: true,
        fields: vec![ferris_schema::FieldMeta {
            fieldname: "company".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let ctx = EngineContext::new("Print Settings", "")
        .with_meta(&meta)
        .with_user(&sysadmin());

    // First read materialises the row with the fixed name.
    let created = engine.get_single(&ctx).await.unwrap();
    assert_eq!(
        created.get("name").and_then(|v| v.as_str()),
        Some("Print Settings")
    );
    assert_eq!(
        created.get("docstatus").and_then(|v| v.as_str()),
        Some("draft")
    );

    // A second read returns the same single row (there is no second row).
    let again = engine.get_single(&ctx).await.unwrap();
    assert_eq!(
        again.get("name").and_then(|v| v.as_str()),
        Some("Print Settings")
    );
    let (rows, total) = engine
        .list(&ctx, &crate::ListQuery::default())
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows.len(), 1);

    // `save_single` updates in place.
    let updated = engine
        .save_single(&ctx, ferris_core::value!({ "company": "Ferris" }))
        .await
        .unwrap();
    assert_eq!(
        updated.get("company").and_then(|v| v.as_str()),
        Some("Ferris")
    );
    let (_, total) = engine
        .list(&ctx, &crate::ListQuery::default())
        .await
        .unwrap();
    assert_eq!(total, 1, "save_single must not insert a second row");

    // A body that tries to rename the single is ignored: the name is fixed.
    let renamed = engine
        .save_single(
            &ctx,
            ferris_core::value!({ "name": "Something Else", "company": "X" }),
        )
        .await
        .unwrap();
    assert_eq!(
        renamed.get("name").and_then(|v| v.as_str()),
        Some("Print Settings")
    );
}

/// Direct `create`/`create_bulk`/`delete` on a Single are refused with a
/// typed error rather than silently minting or removing its one row.
#[tokio::test]
async fn single_doctype_refuses_direct_create_and_delete() {
    let engine = engine().await;
    let meta = DoctypeMeta {
        name: "System Settings".to_string(),
        is_single: true,
        ..Default::default()
    };
    let ctx = EngineContext::new("System Settings", "")
        .with_meta(&meta)
        .with_user(&sysadmin());

    let err = engine
        .create(&ctx, ferris_core::value!({ "x": 1 }))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Document(ferris_core::DocumentError::SingleNotCreatable { .. })
        ),
        "{err:?}"
    );

    let err = engine
        .create_bulk(&ctx, vec![ferris_core::value!({ "x": 1 })])
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Document(ferris_core::DocumentError::SingleNotCreatable { .. })
        ),
        "{err:?}"
    );

    // Materialise it, then deletion is refused.
    engine.get_single(&ctx).await.unwrap();
    let err = engine.delete(&ctx, "System Settings").await.unwrap_err();
    assert!(
        matches!(
            err,
            Error::Document(ferris_core::DocumentError::SingleNotDeletable { .. })
        ),
        "{err:?}"
    );
    // The row is still there.
    assert!(engine.get_single(&ctx).await.is_ok());
}

/// `get_single`/`save_single` on a doctype that is not a Single is a caller
/// bug, not a silent single.
#[tokio::test]
async fn get_single_rejects_a_non_single_doctype() {
    let engine = engine().await;
    let meta = DoctypeMeta {
        name: "ToDo".to_string(),
        ..Default::default()
    };
    let ctx = EngineContext::new("ToDo", "TODO-.#####")
        .with_meta(&meta)
        .with_user(&sysadmin());
    assert!(engine.get_single(&ctx).await.is_err());
    assert!(
        engine
            .save_single(&ctx, ferris_core::value!({ "x": 1 }))
            .await
            .is_err()
    );
}

/// Materialising a Single applies its declared defaults, so a Single with a
/// required field can be auto-created before anyone sets it.
#[tokio::test]
async fn get_single_applies_declared_defaults() {
    let engine = engine().await;
    let meta = DoctypeMeta {
        name: "Email Settings".to_string(),
        is_single: true,
        fields: vec![
            ferris_schema::FieldMeta {
                fieldname: "enabled".to_string(),
                fieldtype: "Check".to_string(),
                default: Some("1".to_string()),
                reqd: true,
                ..Default::default()
            },
            ferris_schema::FieldMeta {
                fieldname: "batch_size".to_string(),
                fieldtype: "Int".to_string(),
                default: Some("100".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let ctx = EngineContext::new("Email Settings", "")
        .with_meta(&meta)
        .with_user(&sysadmin());

    let doc = engine.get_single(&ctx).await.unwrap();
    assert_eq!(doc.get("enabled"), Some(&Value::Bool(true)));
    assert_eq!(doc.get("batch_size"), Some(&Value::from(100)));
}

/// The owner a permission check evaluates against is disabled for a Single
/// (its row is global) and preserved for every other `DocType`.
#[test]
fn authz_owner_is_disabled_for_singles() {
    let single = DoctypeMeta {
        name: "Global Settings".to_string(),
        is_single: true,
        ..Default::default()
    };
    let ctx = EngineContext::new("Global Settings", "").with_meta(&single);
    assert_eq!(authz_owner(&ctx, Some("ada@example.com")), None);

    let normal = DoctypeMeta {
        name: "ToDo".to_string(),
        ..Default::default()
    };
    let ctx = EngineContext::new("ToDo", "").with_meta(&normal);
    assert_eq!(
        authz_owner(&ctx, Some("ada@example.com")),
        Some("ada@example.com")
    );

    // An unregistered DocType (no meta) is never a single.
    let ctx = EngineContext::new("Probe", "");
    assert_eq!(
        authz_owner(&ctx, Some("ada@example.com")),
        Some("ada@example.com")
    );
}
