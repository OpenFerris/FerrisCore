use ferris_core::Value;

use crate::types::{ListParams, ListRequest};

use super::super::DocStore;
use super::harness::{admin, pool};

#[tokio::test]
async fn test_unregistered_doctype_crud() {
    let pool = pool().await;
    DocStore::init(&pool).unwrap();

    ferris_document::with_user(Some(admin()), async {
        let created = DocStore::create(
            &pool,
            "Probe",
            ferris_core::value!({ "name": "P-1", "title": "hello" }),
        )
        .await
        .unwrap();
        assert_eq!(created.get("name").and_then(|v| v.as_str()), Some("P-1"));

        let fetched = DocStore::get(&pool, "Probe", "P-1").await.unwrap();
        assert_eq!(fetched.get("title").and_then(|v| v.as_str()), Some("hello"));
        assert_eq!(
            fetched.get("docstatus").and_then(|v| v.as_str()),
            Some("draft")
        );

        let updated = DocStore::update(
            &pool,
            "Probe",
            "P-1",
            ferris_core::value!({ "title": "bye" }),
        )
        .await
        .unwrap();
        assert_eq!(updated.get("title").and_then(|v| v.as_str()), Some("bye"));

        let mut params = ListParams::default();
        params.filters = Some(
            [("title".to_string(), Value::from("bye"))]
                .into_iter()
                .collect(),
        );
        let listed = DocStore::list(&pool, "Probe", &params).await.unwrap();
        assert_eq!(listed.total, 1);

        DocStore::delete(&pool, "Probe", "P-1").await.unwrap();
        assert!(DocStore::get(&pool, "Probe", "P-1").await.is_err());
    })
    .await;
}

#[tokio::test]
async fn test_list_requested_ops_order_and_search() {
    let pool = pool().await;
    ferris_document::with_user(Some(admin()), async {
        for (name, qty) in [("P-1", 3u64), ("P-2", 8), ("P-3", 5), ("P-4", 1)] {
            let doc = Value::from_json(serde_json::json!({
                "name": name,
                "qty": qty,
                "status": if qty >= 5 { "high" } else { "low" },
            }));
            DocStore::create(&pool, "Probe", doc).await.unwrap();
        }

        // Range filter (gte 5), desc ordering by qty, no page truncation.
        let req = ListRequest {
            limit_start: 0,
            limit_page_length: 10,
            filters: Some(r#"[["qty","gte","5"]]"#.to_string()),
            order_by: Some("qty desc".to_string()),
            search: None,
            fields: Some(vec!["name".to_string(), "qty".to_string()]),
            only_liked: None,
        };
        let resp = DocStore::list_requested(&pool, "Probe", &req, None)
            .await
            .unwrap();
        let names: Vec<String> = resp
            .data
            .iter()
            .map(|v| {
                v.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_eq!(names, vec!["P-2", "P-3"]);

        // OR group: qty <= 1 OR like "P-2" name.
        let req = ListRequest {
            limit_start: 0,
            limit_page_length: 10,
            filters: Some(r#"[["qty","lte","1",false],["name","like","P-2",true]]"#.to_string()),
            order_by: None,
            search: None,
            fields: None,
            only_liked: None,
        };
        let resp = DocStore::list_requested(&pool, "Probe", &req, None)
            .await
            .unwrap();
        assert_eq!(resp.total, 2);

        // Search across a field (case-insensitive substring).
        let req = ListRequest {
            limit_start: 0,
            limit_page_length: 10,
            filters: None,
            order_by: None,
            search: Some("p-1".to_string()),
            fields: None,
            only_liked: None,
        };
        let resp = DocStore::list_requested(&pool, "Probe", &req, None)
            .await
            .unwrap();
        assert_eq!(resp.total, 1);
    })
    .await;
}

/// `only_liked` is the one filter the document cannot answer by itself: the
/// predicate is a row in `tab_like`, so the store resolves it into a set of
/// docnames and hands that to the engine.
#[tokio::test]
async fn test_only_liked_narrows_the_list_to_the_users_likes() {
    let pool = pool().await;
    ferris_document::with_user(Some(admin()), async {
        for name in ["P-1", "P-2", "P-3"] {
            DocStore::create(
                &pool,
                "Probe",
                Value::from_json(serde_json::json!({ "name": name })),
            )
            .await
            .unwrap();
        }

        let likes = ferris_document::LikeStore::new(pool.clone());
        likes
            .toggle("Probe", "P-1", "ada@example.com")
            .await
            .unwrap();
        likes
            .toggle("Probe", "P-3", "ada@example.com")
            .await
            .unwrap();
        likes
            .toggle("Probe", "P-2", "grace@example.com")
            .await
            .unwrap();

        // `limit_page_length` gets its default through serde on the wire,
        // not through `Default`, so it is set here the way the wire sets it.
        let req = ListRequest {
            limit_page_length: 20,
            only_liked: Some(true),
            ..Default::default()
        };

        // The whole set, not just the loaded page: `total` follows the likes.
        let resp = DocStore::list_requested(&pool, "Probe", &req, Some("ada@example.com"))
            .await
            .unwrap();
        assert_eq!(resp.total, 2);
        let mut names: Vec<String> = resp
            .data
            .iter()
            .map(|v| {
                v.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["P-1", "P-3"]);

        // Another user sees their own likes, not these.
        let resp = DocStore::list_requested(&pool, "Probe", &req, Some("grace@example.com"))
            .await
            .unwrap();
        assert_eq!(resp.total, 1);

        // Without an identity the flag is ignored rather than matching
        // nothing: an unidentified caller sees the unfiltered list.
        let resp = DocStore::list_requested(&pool, "Probe", &req, None)
            .await
            .unwrap();
        assert_eq!(resp.total, 3);
    })
    .await;
}

/// A user who liked nothing gets an empty list, not the whole doctype.
#[tokio::test]
async fn test_only_liked_for_a_user_without_likes() {
    let pool = pool().await;
    ferris_document::with_user(Some(admin()), async {
        DocStore::create(
            &pool,
            "Probe",
            Value::from_json(serde_json::json!({ "name": "P-1" })),
        )
        .await
        .unwrap();

        // `limit_page_length` gets its default through serde on the wire,
        // not through `Default`, so it is set here the way the wire sets it.
        let req = ListRequest {
            limit_page_length: 20,
            only_liked: Some(true),
            ..Default::default()
        };
        let resp = DocStore::list_requested(&pool, "Probe", &req, Some("ada@example.com"))
            .await
            .unwrap();
        assert_eq!(resp.total, 0);
        assert!(resp.data.is_empty());
    })
    .await;
}
