//! Tests for the tag service and the hook adapter.
//!
//! The hook is exercised through a **local** [`HookRegistry`] and a test-double
//! [`DocumentHook`], never the process-wide one: installing the real hooks in a
//! test would leak into every other test in this binary (the registry has no
//! uninstall) and make them order-dependent.

use std::collections::HashMap;
use std::sync::Arc;

use ferris_core::Value;
use ferris_core::hooks::{DocStatus, DocumentHook, HookContext, HookPoint, HookRegistry};
use ferris_db::Db;

use super::*;
use crate::store::DocStore;

fn admin() -> ferris_auth::User {
    let mut user = ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
    user.roles = vec!["System Manager".to_string()];
    user
}

async fn pool() -> Db {
    ferris_db::memory_db().await.unwrap()
}

// --- parse_tags (pure) -----------------------------------------------------

#[test]
fn parses_a_comma_separated_list() {
    let tags = Value::string("urgent, billing");
    assert_eq!(parse_tags(Some(&tags)), vec!["urgent", "billing"]);
}

#[test]
fn collapses_whitespace_and_drops_duplicates() {
    let tags = Value::string("  New   York , urgent,New York, urgent , ");
    assert_eq!(parse_tags(Some(&tags)), vec!["New York", "urgent"]);
}

#[test]
fn parses_a_multi_select_row_list() {
    let tags = Value::from_json(serde_json::json!([
        { "tag": "urgent" },
        { "tag": "billing" },
        { "not_a_tag": "ignored" },
        "plain"
    ]));
    assert_eq!(parse_tags(Some(&tags)), vec!["urgent", "billing", "plain"]);
}

#[test]
fn tags_are_case_sensitive_and_empty_input_yields_nothing() {
    let tags = Value::string("Urgent, urgent");
    assert_eq!(parse_tags(Some(&tags)), vec!["Urgent", "urgent"]);
    assert!(parse_tags(None).is_empty());
    assert!(parse_tags(Some(&Value::Null)).is_empty());
    assert!(parse_tags(Some(&Value::string(" , , "))).is_empty());
}

// --- DocType declaration ---------------------------------------------------

#[test]
fn tag_link_is_a_hidden_child_table_into_the_catalogue() {
    register();
    crate::doctypes::register_doctypes();
    let meta = ferris_schema::get_doctype(TAG_LINK).expect("Tag Link is registered");
    assert!(meta.is_child_table, "Tag Link is a child table");
    assert!(
        meta.hidden,
        "Tag Link is reached through `tags`, not the sidebar"
    );
    for fieldname in ["document_type", "document_name", "tag"] {
        assert!(
            meta.fields.iter().any(|f| f.fieldname == fieldname),
            "Tag Link needs `{fieldname}`"
        );
    }
}

// --- service (database) ----------------------------------------------------

/// Saving `tags` creates the `Tag` catalogue row and the `Tag Link`
/// associations, and `documents_with_tag` answers from the registry.
#[tokio::test]
async fn sync_populates_the_catalogue_and_associations() {
    crate::doctypes::register_doctypes();
    let pool = pool().await;

    let (name, tags) = ferris_document::with_user(Some(admin()), async {
        let todo = DocStore::create(
            &pool,
            "ToDo",
            Value::from_json(serde_json::json!({
                "title": "Ship tags",
                "tags": "Urgent, Billing",
            })),
        )
        .await
        .expect("ToDo created");
        let name = todo
            .get("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let tags = todo.get("tags").cloned();
        (name, tags)
    })
    .await;

    sync_document(&pool, "ToDo", &name, tags.as_ref())
        .await
        .expect("sync");

    // Catalogue: one row per label, keyed by the label.
    let tag = ferris_document::with_user(Some(admin()), DocStore::get(&pool, TAG, "Urgent"))
        .await
        .expect("Urgent is catalogued");
    assert_eq!(tag.get("tag").and_then(Value::as_str), Some("Urgent"));

    // Association registry.
    let links = documents_with_tag(&pool, "Urgent").await.expect("query");
    assert_eq!(
        links,
        vec![TagLink {
            document_type: "ToDo".to_string(),
            document_name: name.clone(),
            tag: "Urgent".to_string(),
        }]
    );
    assert_eq!(
        documents_with_tag(&pool, "Billing")
            .await
            .expect("query")
            .len(),
        1
    );
}

/// Re-syncing with a shorter list drops the stale link; deleting the document
/// drops the rest. The catalogue row survives (operators curate it).
#[tokio::test]
async fn sync_reconciles_and_remove_clears_links() {
    crate::doctypes::register_doctypes();
    let pool = pool().await;

    let name = ferris_document::with_user(Some(admin()), async {
        let todo = DocStore::create(
            &pool,
            "ToDo",
            Value::from_json(serde_json::json!({
                "title": "Reconcile",
                "tags": "Urgent, Billing",
            })),
        )
        .await
        .expect("toDo created");
        todo.get("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string()
    })
    .await;

    sync_document(
        &pool,
        "ToDo",
        &name,
        Some(&Value::string("Urgent, Billing")),
    )
    .await
    .expect("first sync");
    assert_eq!(documents_with_tag(&pool, "Billing").await.unwrap().len(), 1);

    // "Billing" removed, "Legal" added.
    sync_document(&pool, "ToDo", &name, Some(&Value::string("Urgent, Legal")))
        .await
        .expect("re-sync");
    assert!(
        documents_with_tag(&pool, "Billing")
            .await
            .unwrap()
            .is_empty(),
        "the stale Billing link must be gone"
    );
    assert_eq!(documents_with_tag(&pool, "Legal").await.unwrap().len(), 1);

    // Re-running the same sync is a no-op.
    sync_document(&pool, "ToDo", &name, Some(&Value::string("Urgent, Legal")))
        .await
        .expect("idempotent re-sync");
    assert_eq!(documents_with_tag(&pool, "Legal").await.unwrap().len(), 1);

    remove_document(&pool, "ToDo", &name).await.expect("remove");
    assert!(
        documents_with_tag(&pool, "Urgent")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(documents_with_tag(&pool, "Legal").await.unwrap().is_empty());

    // The catalogue is intentionally untouched by a document removal.
    assert!(
        ferris_document::with_user(Some(admin()), DocStore::get(&pool, TAG, "Urgent"))
            .await
            .is_ok()
    );
}

/// `Tag`/`Tag Link` are the service's own output: syncing them would reconcile
/// their own standard `tags` field and write associations *under themselves*.
/// This is the one place the exclusion lives (`install` skips mounting a hook on
/// them for the same reason, but that is an optimisation, not the guard).
#[tokio::test]
async fn the_service_refuses_its_own_doctypes() {
    crate::doctypes::register_doctypes();
    let pool = pool().await;

    let labels = Value::string("Urgent, Billing");
    sync_document(&pool, TAG, "Urgent", Some(&labels))
        .await
        .expect("guarded call");
    sync_document(&pool, TAG_LINK, "TL-1", Some(&labels))
        .await
        .expect("guarded call");
    remove_document(&pool, TAG, "Urgent")
        .await
        .expect("guarded call");
    remove_document(&pool, TAG_LINK, "TL-1")
        .await
        .expect("guarded call");

    assert!(
        documents_with_tag(&pool, "Urgent")
            .await
            .unwrap()
            .is_empty(),
        "no association may be written under a catalogue/registry row"
    );
    assert!(
        ferris_document::with_user(Some(admin()), DocStore::get(&pool, TAG, "Urgent"))
            .await
            .is_err(),
        "the service must not catalogue its own DocType"
    );
}

/// The hook adapter reads the standard field off the document and drives the
/// service for the matching lifecycle point.
#[tokio::test]
async fn hook_syncs_on_update_and_clears_on_trash() {
    crate::doctypes::register_doctypes();
    let pool = pool().await;

    let name = ferris_document::with_user(Some(admin()), async {
        let todo = DocStore::create(
            &pool,
            "ToDo",
            Value::from_json(serde_json::json!({ "title": "Hooked" })),
        )
        .await
        .expect("toDo created");
        todo.get("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string()
    })
    .await;

    let ctx = Arc::new(TagsContext { db: pool.clone() });
    let mut registry = HookRegistry::new();
    registry.register(
        "ToDo".to_string(),
        HookPoint::OnUpdate,
        document_hook(ctx.clone(), HookPoint::OnUpdate),
        0,
        None,
    );
    registry.register(
        "ToDo".to_string(),
        HookPoint::OnTrash,
        document_hook(ctx, HookPoint::OnTrash),
        0,
        None,
    );

    let mut doc = FakeDoc::new("ToDo", &name);
    doc.fields
        .insert("tags".to_string(), Value::string("Hooked, Urgent"));
    let mut hook_ctx = HookContext::new("ToDo".to_string(), name.clone(), String::new());
    registry
        .execute("ToDo", HookPoint::OnUpdate, &mut hook_ctx, &mut doc)
        .await
        .expect("hook ran");
    assert_eq!(documents_with_tag(&pool, "Hooked").await.unwrap().len(), 1);

    // Trash clears the document's links.
    let mut hook_ctx = HookContext::new("ToDo".to_string(), name.clone(), String::new());
    registry
        .execute("ToDo", HookPoint::OnTrash, &mut hook_ctx, &mut doc)
        .await
        .expect("hook ran");
    assert!(
        documents_with_tag(&pool, "Hooked")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        documents_with_tag(&pool, "Urgent")
            .await
            .unwrap()
            .is_empty()
    );
}

/// Minimal [`DocumentHook`] over a field map, so the hook can be driven without
/// a database write.
struct FakeDoc {
    doctype: String,
    name: String,
    fields: HashMap<String, Value>,
}

impl FakeDoc {
    fn new(doctype: &str, name: &str) -> Self {
        Self {
            doctype: doctype.to_string(),
            name: name.to_string(),
            fields: HashMap::new(),
        }
    }
}

impl DocumentHook for FakeDoc {
    fn doctype(&self) -> &str {
        &self.doctype
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn get_field(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }
    fn set_field(&mut self, field: String, value: Value) {
        self.fields.insert(field, value);
    }
    fn get_fields(&self) -> &HashMap<String, Value> {
        &self.fields
    }
    fn is_new(&self) -> bool {
        false
    }
    fn docstatus(&self) -> DocStatus {
        DocStatus::Draft
    }
    fn set_docstatus(&mut self, _status: DocStatus) {}
    fn owner(&self) -> &'static str {
        ""
    }
    fn set_owner(&mut self, _owner: String) {}
    fn modified_by(&self) -> &'static str {
        ""
    }
    fn set_modified_by(&mut self, _user: String) {}
}
