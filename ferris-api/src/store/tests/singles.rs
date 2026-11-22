use ferris_core::Error;
use ferris_schema::DoctypeMeta;

use super::super::{DocStore, effective_meta};
use super::harness::{admin, pool};

/// The `tags` standard field is part of the *effective* meta the REST layer
/// serves, and a tagged document round-trips through `DocStore`: a client
/// can tag any `DocType` without that `DocType` declaring a field. This is
/// the read/write half of "every `DocType` has `tags`".
#[tokio::test]
async fn tags_are_a_standard_field_that_rest_round_trips() {
    let pool = pool().await;
    DocStore::init(&pool).unwrap();
    crate::doctypes::register_doctypes();

    let meta = effective_meta(&pool, "ToDo")
        .await
        .unwrap()
        .expect("ToDo is a registered DocType");
    assert!(
        meta.fields
            .iter()
            .any(|f| f.fieldname == ferris_schema::TAGS_FIELD),
        "effective meta carries the standard `tags` field: {:?}",
        meta.fields
    );

    ferris_document::with_user(Some(admin()), async {
        let created = DocStore::create(
            &pool,
            "ToDo",
            ferris_core::value!({ "title": "call the client", "tags": "urgent, billing" }),
        )
        .await
        .unwrap();
        let name = created
            .get("name")
            .and_then(|v| v.as_str())
            .expect("a created document has a name")
            .to_string();
        let fetched = DocStore::get(&pool, "ToDo", &name).await.unwrap();
        assert_eq!(
            fetched.get("tags").and_then(|v| v.as_str()),
            Some("urgent, billing")
        );
    })
    .await;
}

/// A Single `DocType` read/save through the store: one row named after the
/// doctype, materialised lazily, upserted in place, and refusing the
/// resource-level `create`/`delete` verbs.
#[tokio::test]
async fn single_doctype_store_roundtrip() {
    let pool = pool().await;
    ferris_schema::register_doctype(DoctypeMeta {
        name: "Test Single Settings".to_string(),
        module: "Test".to_string(),
        is_single: true,
        fields: vec![ferris_schema::FieldMeta {
            fieldname: "company".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });

    ferris_document::with_user(Some(admin()), async {
        let first = DocStore::get_single(&pool, "Test Single Settings")
            .await
            .unwrap();
        assert_eq!(
            first.get("name").and_then(|v| v.as_str()),
            Some("Test Single Settings")
        );

        let saved = DocStore::save_single(
            &pool,
            "Test Single Settings",
            ferris_core::value!({ "company": "Ferris" }),
        )
        .await
        .unwrap();
        assert_eq!(
            saved.get("company").and_then(|v| v.as_str()),
            Some("Ferris")
        );

        // Reading again sees the saved value, still one row.
        let again = DocStore::get_single(&pool, "Test Single Settings")
            .await
            .unwrap();
        assert_eq!(
            again.get("company").and_then(|v| v.as_str()),
            Some("Ferris")
        );

        // `POST /api/resource/{doctype}` cannot mint a second row.
        let err = DocStore::create(
            &pool,
            "Test Single Settings",
            ferris_core::value!({ "x": 1 }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Document(ferris_core::DocumentError::SingleNotCreatable { .. })
            ),
            "{err:?}"
        );

        // `DELETE /api/resource/{doctype}/{name}` cannot remove it either.
        let err = DocStore::delete(&pool, "Test Single Settings", "Test Single Settings")
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Document(ferris_core::DocumentError::SingleNotDeletable { .. })
            ),
            "{err:?}"
        );
    })
    .await;
}

/// Neighbours come from the engine's list path: the document before and
/// after in name order, and `None` at either end so the form's prev/next
/// controls can disable themselves.
#[tokio::test]
async fn neighbors_are_the_adjacent_documents_in_name_order() {
    let pool = pool().await;
    ferris_document::with_user(Some(admin()), async {
        for name in ["P-1", "P-2", "P-3"] {
            DocStore::create(&pool, "Probe", ferris_core::value!({ "name": name }))
                .await
                .unwrap();
        }

        let middle = DocStore::neighbors(&pool, "Probe", "P-2").await.unwrap();
        assert_eq!(middle.prev.as_deref(), Some("P-1"));
        assert_eq!(middle.next.as_deref(), Some("P-3"));

        // The first document has no predecessor, the last no successor.
        let first = DocStore::neighbors(&pool, "Probe", "P-1").await.unwrap();
        assert_eq!(first.prev, None);
        assert_eq!(first.next.as_deref(), Some("P-2"));

        let last = DocStore::neighbors(&pool, "Probe", "P-3").await.unwrap();
        assert_eq!(last.prev.as_deref(), Some("P-2"));
        assert_eq!(last.next, None);
    })
    .await;
}
