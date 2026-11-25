//! Tests for the `User Permission` store and snapshot.
//!
//! The snapshot is process-wide, so every test that touches it takes
//! [`guard`] and starts from [`clear`]; `cache.rs` reuses the same guard.

use super::*;
use ferris_document::EngineContext;
use ferris_schema::{DoctypeMeta, FieldMeta};

/// The snapshot is process-wide; these tests serialise on it.
static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) struct Guard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        clear();
    }
}

pub(super) fn guard() -> Guard {
    let guard = SERIALISE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear();
    Guard { _guard: guard }
}

fn tree_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "D1Tree".to_string(),
        is_tree: true,
        tree_parent_field: Some("tree_parent".to_string()),
        fields: vec![
            FieldMeta::link("tree_parent", "D1Tree"),
            FieldMeta::data("title"),
        ],
        ..Default::default()
    }
}

fn plain_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "D1Plain".to_string(),
        fields: vec![FieldMeta::data("title")],
        ..Default::default()
    }
}

fn system() -> ferris_core::User {
    let mut user =
        ferris_core::User::system_user("Administrator".to_string(), "Administrator".to_string());
    user.id = "Administrator".to_string();
    user.roles = vec![ferris_core::permissions::SYSTEM_MANAGER_ROLE.to_string()];
    user
}

#[tokio::test]
async fn upsert_round_trips_and_refresh_publishes_by_user() {
    let _guard = guard();
    let db = ferris_db::memory_db().await.unwrap();
    let store = UserPermissionStore::new(db.clone());
    store.init().await.unwrap();

    let rule =
        UserPermissionRule::new("ada@local", "Customer", "CUST-A").with_apply_to_all_doctypes(true);
    store.upsert(&rule).await.unwrap();
    // Same natural key: overwrite, not duplicate.
    store.upsert(&rule).await.unwrap();
    assert_eq!(store.list_all().await.unwrap().len(), 1);
    assert_eq!(store.list_for_user("ada@local").await.unwrap().len(), 1);
    assert!(store.list_for_user("bob@local").await.unwrap().is_empty());

    refresh(&db).await.unwrap();
    let installed = rules_for("ada@local");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].for_value, "CUST-A");
    assert!(installed[0].apply_to_all_doctypes);
    assert!(rules_for("bob@local").is_empty());

    store
        .delete("ada@local", "Customer", "CUST-A")
        .await
        .unwrap();
    refresh(&db).await.unwrap();
    assert!(rules_for("ada@local").is_empty());
}

/// A rule written by another replica (a second store over the same
/// database) only reaches this process through a refresh, not through the
/// hooks that fired on the writer.
#[tokio::test]
async fn a_rule_written_by_another_replica_is_visible_after_one_refresh() {
    let _guard = guard();
    let db = ferris_db::memory_db().await.unwrap();
    let local = UserPermissionStore::new(db.clone());
    local.init().await.unwrap();
    refresh(&db).await.unwrap();
    assert!(rules_for("ada@local").is_empty());

    let replica = UserPermissionStore::new(db.clone());
    replica
        .upsert(&UserPermissionRule::new("ada@local", "Customer", "CUST-A"))
        .await
        .unwrap();

    // One refresh cycle, called directly: no interval is waited for.
    refresh(&db).await.unwrap();
    let installed = rules_for("ada@local");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].for_value, "CUST-A");
}

/// `hide_descendants` on a tree expands to its descendants, so the per-row
/// check later sees a plain value list.
#[tokio::test]
async fn hide_descendants_expands_a_tree() {
    let _guard = guard();
    let meta = tree_meta();
    ferris_schema::register_doctype(meta.clone());
    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    engine.ensure_table("D1Tree").await.unwrap();
    let ctx = EngineContext::new("D1Tree", "")
        .with_meta(&meta)
        .with_user(&system())
        .with_explicit_name();
    engine
        .create(
            &ctx,
            ferris_core::value!({ "name": "ROOT", "title": "root" }),
        )
        .await
        .unwrap();
    engine
        .create(
            &ctx,
            ferris_core::value!({ "name": "CHILD", "title": "child", "tree_parent": "ROOT" }),
        )
        .await
        .unwrap();
    engine
        .create(
            &ctx,
            ferris_core::value!({ "name": "GRANDCHILD", "title": "g", "tree_parent": "CHILD" }),
        )
        .await
        .unwrap();

    let store = UserPermissionStore::new(db.clone());
    store.init().await.unwrap();
    store
        .upsert(&UserPermissionRule::new("ada@local", "D1Tree", "ROOT").with_hide_descendants(true))
        .await
        .unwrap();

    refresh(&db).await.unwrap();
    let installed = rules_for("ada@local");
    assert_eq!(installed.len(), 1);
    assert!(installed[0].allows("CHILD"));
    assert!(installed[0].allows("GRANDCHILD"));
    assert!(!installed[0].allows("OTHER"));
}

/// A `hide_descendants` rule that cannot be honoured is rejected, not
/// ignored: the load fails with a message naming the `DocType`.
#[tokio::test]
async fn hide_descendants_on_a_non_tree_is_rejected() {
    let _guard = guard();
    ferris_schema::register_doctype(plain_meta());
    let db = ferris_db::memory_db().await.unwrap();
    let store = UserPermissionStore::new(db.clone());
    store.init().await.unwrap();
    store
        .upsert(&UserPermissionRule::new("ada@local", "D1Plain", "X").with_hide_descendants(true))
        .await
        .unwrap();

    let error = refresh(&db)
        .await
        .expect_err("a non-tree rule must be rejected");
    let message = error.to_string();
    assert!(message.contains("D1Plain"), "{message}");
    assert!(message.contains("hide_descendants"), "{message}");
}

/// The same rule is rejected at write time, so the operator gets a `400`
/// instead of storing a rule the engine cannot honour.
#[test]
fn validation_rejects_hide_descendants_without_a_tree() {
    ferris_schema::register_doctype(plain_meta());
    let error =
        validate_rule_values(true, "D1Plain").expect_err("a non-tree target must be rejected");
    assert!(matches!(error, Error::Validation(_)), "{error:?}");

    // No flag, no complaint.
    validate_rule_values(false, "D1Plain").unwrap();
}
