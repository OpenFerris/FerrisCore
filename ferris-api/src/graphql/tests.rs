//! Behavioural checks on the GraphQL subset: projection, aliases, variables,
//! introspection and the parser's two hard refusals (mutations, deep nesting).
//!
//! Reads go through [`DocStore`], which enforces `Permission::Read`, so every
//! query here runs as an administrator deliberately ([`as_admin`]) — anonymous
//! reads are refused by design, not by accident.

use serde_json::Map as JsonMap;

use crate::store::DocStore;

use super::execute_query;
use super::parser::{MAX_DEPTH, Parser, tokenize};

async fn pool_with_docs() -> ferris_db::Db {
    let pool = ferris_db::memory_db().await.unwrap();
    DocStore::init(&pool).unwrap();
    let mut admin = ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
    admin.roles = vec!["System Manager".to_string()];
    ferris_document::with_user(Some(admin), async {
        DocStore::create(
            &pool,
            "ToDo",
            ferris_core::value!({ "name": "TODO-1", "title": "Buy milk", "status": "open" }),
        )
        .await
        .unwrap();
        DocStore::create(
            &pool,
            "ToDo",
            ferris_core::value!({ "name": "TODO-2", "title": "Write docs", "status": "closed" }),
        )
        .await
        .unwrap();
    })
    .await;
    pool
}

/// Run `fut` with `admin` as the ambient engine user.
///
/// GraphQL reads go through `DocStore`, which now enforces
/// `Permission::Read` (§C2), and `/api/graphql` sits inside the router
/// layer that scopes the resolved caller. A test that queries outside such
/// a scope is anonymous, and anonymous reads are refused — which is the
/// behaviour these tests must exercise deliberately, not by accident.
async fn as_admin<T, F>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let mut admin = ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
    admin.roles = vec!["System Manager".to_string()];
    ferris_document::with_user(Some(admin), fut).await
}

/// A deeply nested query must be refused at parse time, not overflow the
/// stack and abort the process (V3-API-1). The recursion is what a single
/// tiny request would otherwise drive.
#[test]
fn deeply_nested_query_is_rejected_not_overflowed() {
    let depth = MAX_DEPTH + 5;
    let mut query = String::new();
    for _ in 0..depth {
        query.push_str("{ a");
    }
    for _ in 0..depth {
        query.push('}');
    }
    let tokens = tokenize(&query).expect("tokens");
    let mut parser = Parser::new(tokens, JsonMap::new());
    let err = parser.parse_document().unwrap_err();
    assert!(err.contains("nesting"), "{err}");
}

#[tokio::test]
async fn test_query_list_and_projection() {
    let pool = pool_with_docs().await;
    let response = as_admin(execute_query(
        &pool,
        "{ ToDo { name title } }",
        &JsonMap::new(),
    ))
    .await;
    assert!(response.errors.is_empty());
    let data = response.data.unwrap();
    let todos = data["ToDo"].as_array().unwrap();
    assert_eq!(todos.len(), 2);
    assert!(todos[0]["name"].is_string());
    assert!(
        todos[0].get("status").is_none(),
        "projection should drop status"
    );
}

#[tokio::test]
async fn test_query_single_and_alias_and_variables() {
    let pool = pool_with_docs().await;
    let response = as_admin(execute_query(
        &pool,
        "query { first: ToDo(name: $name) { name title } }",
        &serde_json::json!({ "name": "TODO-1" })
            .as_object()
            .unwrap()
            .clone(),
    ))
    .await;
    assert!(response.errors.is_empty());
    assert_eq!(response.data.unwrap()["first"]["name"], "TODO-1");
}

#[tokio::test]
async fn test_introspection_and_parse_error() {
    let pool = pool_with_docs().await;
    let response = execute_query(&pool, "{ __schema { types } }", &JsonMap::new()).await;
    assert!(response.errors.is_empty());
    assert!(response.data.unwrap()["__schema"]["queryType"]["name"].is_string());

    let response = execute_query(&pool, "{ ToDo ", &JsonMap::new()).await;
    assert!(!response.errors.is_empty());
    assert!(response.data.is_none());
}

#[tokio::test]
async fn test_mutations_rejected() {
    let pool = pool_with_docs().await;
    let response = execute_query(&pool, "mutation { ToDo { name } }", &JsonMap::new()).await;
    assert!(!response.errors.is_empty());
}
