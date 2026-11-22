//! Integration test for the `#[doctype]` macro: metadata, validation and CRUD.

use ferris_core::Value;
use ferris_db::{Db, Stmt};
use ferris_document::{Document, DocumentExt};
use ferris_macro::doctype;
use ferris_macro::permission;
use serde::{Deserialize, Serialize};

#[doctype(
    name = "ToDo",
    module = "core",
    naming = "TODO-.#####",
    permissions = [
        { role = "System Manager", read = true, write = true, create = true, delete = true },
        { role = "User", read = true, write = true, create = true },
    ]
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToDo {
    #[key]
    pub name: String,
    #[field(type = "Data", label = "Title", reqd = true)]
    pub title: String,
    #[field(type = "Select", label = "Status", options = "open\nclosed")]
    pub status: String,
    #[field(type = "Email", label = "Owner Email")]
    pub email: Option<String>,
    pub docstatus: ferris_core::DocStatus,
}

async fn memory_db() -> Db {
    let db = ferris_db::memory_db().await.unwrap();
    ferris_document::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    db
}

fn todo(name: &str, title: &str) -> ToDo {
    // `#[doctype]` registers lazily on first use, but `DocumentExt::meta()` only
    // reads the registry. Register explicitly so a test that calls `amend()`
    // before any naming/validate path does not depend on test ordering.
    let _ = ToDo::ferris_register_doctype();
    ToDo {
        name: name.to_string(),
        title: title.to_string(),
        status: "open".to_string(),
        email: Some("user@example.com".to_string()),
        docstatus: ferris_core::DocStatus::Draft,
    }
}

fn admin_user() -> ferris_core::User {
    let mut user = ferris_core::User::system_user("admin@local".to_string(), "Admin".to_string());
    user.roles = vec!["System Manager".to_string()];
    user
}

async fn stored(db: &Db, name: &str) -> Value {
    db.fetch_one(&Stmt::new(
        "SELECT data FROM tabToDo WHERE name = ?",
        vec![Value::from(name)],
    ))
    .await
    .unwrap()
    .json("data")
    .unwrap()
}

#[test]
fn test_metadata_is_generated_and_registered() {
    let meta = ToDo::ferris_register_doctype();
    assert_eq!(meta.name, "ToDo");
    assert_eq!(meta.module, "core");
    assert_eq!(meta.naming_series, "TODO-.#####");
    assert_eq!(meta.permissions.len(), 2);

    let fieldnames: Vec<&str> = meta.fields.iter().map(|f| f.fieldname.as_str()).collect();
    assert!(fieldnames.contains(&"title"));
    assert!(fieldnames.contains(&"status"));
    assert!(fieldnames.contains(&"email"));

    assert!(<ToDo as DocumentExt>::meta().is_some());
}

#[test]
fn test_validation_rejects_invalid_select() {
    let mut doc = todo("TODO-1", "Buy milk");
    assert!(doc.validate().is_ok());

    doc.status = "bogus".to_string();
    assert!(doc.validate().is_err());
}

#[tokio::test]
async fn test_crud_roundtrip() {
    let db = memory_db().await;

    ferris_document::with_user(Some(admin_user()), async {
        let mut doc = todo("TODO-1", "Buy milk");
        let name = doc.insert(&db).await.unwrap();
        assert_eq!(name, "TODO-1");

        let value = stored(&db, "TODO-1").await;
        assert_eq!(value.get("title").and_then(Value::as_str), Some("Buy milk"));
        assert_eq!(value.get("status").and_then(Value::as_str), Some("open"));

        doc.title = "Buy oat milk".to_string();
        doc.save(&db).await.unwrap();
        let value = stored(&db, "TODO-1").await;
        assert_eq!(
            value.get("title").and_then(Value::as_str),
            Some("Buy oat milk")
        );

        doc.submit(&db).await.unwrap();
        assert_eq!(doc.get_docstatus(), ferris_core::DocStatus::Submitted);
        doc.cancel(&db).await.unwrap();
        assert_eq!(doc.get_docstatus(), ferris_core::DocStatus::Cancelled);

        doc.delete(&db).await.unwrap();
        let row = db
            .fetch_one(&Stmt::new("SELECT COUNT(*) AS n FROM tabToDo", vec![]))
            .await
            .unwrap();
        assert_eq!(row.count("n"), 0);
    })
    .await;
}

#[tokio::test]
async fn test_anonymous_typed_crud_is_denied() {
    let db = memory_db().await;
    let mut doc = todo("TODO-1", "Buy milk");
    let err = doc.insert(&db).await.unwrap_err();
    assert!(matches!(
        err,
        ferris_core::Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
    ));
}

#[tokio::test]
async fn test_lifecycle_hook_runs_on_insert() {
    use ferris_core::hooks::{HookPoint, HookRegistry};

    HookRegistry::register_global(
        "ToDo".to_string(),
        HookPoint::BeforeInsert,
        Box::new(|_ctx, doc| {
            if doc.get_field("title") == Some(&ferris_core::Value::from("hook-me")) {
                doc.set_field("status".to_string(), ferris_core::Value::from("hooked"));
            }
            Box::pin(async { Ok::<(), ferris_core::Error>(()) })
        }),
        0,
        None,
    );

    let db = memory_db().await;
    ferris_document::with_user(Some(admin_user()), async {
        let mut doc = todo("TODO-HOOK", "hook-me");
        doc.insert(&db).await.unwrap();

        let value = stored(&db, "TODO-HOOK").await;
        assert_eq!(value.get("status").and_then(Value::as_str), Some("hooked"));
    })
    .await;
}

#[tokio::test]
async fn test_amend_creates_new_draft() {
    let db = memory_db().await;

    let new_name = ferris_document::with_user(Some(admin_user()), async {
        let mut doc = todo("TODO-1", "Buy milk");
        doc.insert(&db).await.unwrap();
        doc.submit(&db).await.unwrap();
        // Frappe amends a cancelled document, not a live one (DOC-02).
        doc.cancel(&db).await.unwrap();

        let new_name = doc.amend(&db).await.unwrap();
        assert_ne!(new_name, "TODO-1");
        new_name
    })
    .await;

    let value = stored(&db, &new_name).await;
    assert_eq!(
        value.get("docstatus").and_then(Value::as_str),
        Some("draft")
    );
}

/// A18: `#[permission]` used to expand to `#input` and drop the attribute —
/// the rule compiled away to nothing and was never enforced.
///
/// It must now install the declared grants into the global (default-deny)
/// engine.
#[permission(resource = "ToDo", actions = ["Read", "Write"], roles = ["Support Agent"])]
#[derive(Debug, Clone)]
pub struct ToDoSupportPermission {}

#[test]
fn permission_macro_installs_a_policy() {
    ToDoSupportPermission::register_permission().unwrap();

    let engine = ferris_core::permissions::global_engine_ref();
    let guard = engine.read().unwrap();

    let allowed = guard.check(
        &ferris_core::permissions::Principal::role("Support Agent"),
        ferris_core::permissions::Permission::Read,
        &ferris_core::permissions::Resource::new("ToDo", "TODO-100"),
        &std::collections::HashMap::default(),
    );
    assert_eq!(allowed, ferris_core::permissions::Decision::Allow);

    // The grant is role-scoped: a stranger with no rule gets default-deny,
    // and an action outside the granted list stays ungranted.
    assert_eq!(
        guard.check(
            &ferris_core::permissions::Principal::role("Intern"),
            ferris_core::permissions::Permission::Read,
            &ferris_core::permissions::Resource::new("ToDo", "TODO-200"),
            &std::collections::HashMap::default(),
        ),
        ferris_core::permissions::Decision::NotApplicable
    );
    assert_eq!(
        guard.check(
            &ferris_core::permissions::Principal::role("Support Agent"),
            ferris_core::permissions::Permission::Delete,
            &ferris_core::permissions::Resource::new("ToDo", "TODO-300"),
            &std::collections::HashMap::default(),
        ),
        ferris_core::permissions::Decision::NotApplicable
    );
}

/// The declared naming strategy reaches both the registry metadata (which the
/// Desk reads) and the trait consts (which the typed engine context reads). One
/// declaration, two consumers, so a drift is a compile error, not a runtime
/// surprise.
#[doctype(
    name = "AutoNameProbe",
    module = "core",
    naming = "PROBE-.#####",
    naming_type = "auto",
    permissions = [
        { role = "System Manager", read = true, write = true, create = true, delete = true },
    ]
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoNameProbe {
    #[key]
    pub name: String,
    #[field(type = "Data", label = "Title")]
    pub title: String,
    pub docstatus: ferris_core::DocStatus,
}

#[test]
fn naming_type_declaration_lands_in_metadata_and_trait() {
    use ferris_schema::contract::NamingType;
    let meta = AutoNameProbe::ferris_register_doctype();
    assert_eq!(meta.naming_type, Some(NamingType::AutoIncrement));
    assert_eq!(meta.effective_naming_type(), NamingType::AutoIncrement);
    assert_eq!(
        <AutoNameProbe as Document>::NAMING_TYPE,
        Some(NamingType::AutoIncrement)
    );
}

/// A `naming_type = "field"` doctype is named after that field end-to-end
/// through the typed CRUD path, not only in the engine unit tests.
#[doctype(
    name = "CodeBook",
    module = "core",
    naming = "",
    naming_type = "field",
    naming_field = "code",
    permissions = [
        { role = "System Manager", read = true, write = true, create = true, delete = true },
    ]
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBook {
    #[key]
    pub name: String,
    #[field(type = "Data", label = "Code", reqd = true)]
    pub code: String,
    #[field(type = "Data", label = "Title")]
    pub title: String,
    pub docstatus: ferris_core::DocStatus,
}

#[tokio::test]
async fn field_naming_names_the_typed_document_after_the_field() {
    let db = memory_db().await;
    ferris_document::with_user(Some(admin_user()), async {
        let mut doc = CodeBook {
            name: String::new(),
            code: "SKU-9".to_string(),
            title: "Widget".to_string(),
            docstatus: ferris_core::DocStatus::Draft,
        };
        let name = doc.insert(&db).await.unwrap();
        assert_eq!(name, "SKU-9");
        assert_eq!(doc.get_name(), "SKU-9");
    })
    .await;
}
