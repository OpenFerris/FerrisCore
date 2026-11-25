//! Runtime `DocType` creation and editing: the server half of the Desk builder.
//!
//! A `DocType` created here is stored in `tabDocType` (`custom = true`), its
//! table is materialized with the real columns of the Phase 1 schema, and the
//! metadata is registered in-process so the next request can use it. Compiled
//! declarations are never overwritten: they belong to the code, and an operator
//! edit of one would disappear on the next boot.
//!
//! This is deliberately a small, admin-only surface. Validation rejects the
//! names and field shapes that would create a table the engine cannot read; the
//! actual DDL goes through the same `doctype_columns` generator the engine uses,
//! so a runtime `DocType` and a compiled one cannot drift.

use std::collections::HashSet;

use ferris_core::error::{AuthzError, DocumentError, SchemaError, ValidationError};
use ferris_core::permissions::SYSTEM_MANAGER_ROLE;
use ferris_core::{Error, Result, User};
use ferris_db::Db;
use ferris_document::{DoctypeStore, DocumentEngine};
use ferris_schema::DoctypeMeta;

/// Names owned by the framework. A runtime `DocType` may not shadow one.
const RESERVED: &[&str] = &[
    "DocType",
    "DocField",
    "DocPerm",
    "Custom DocPerm",
    "User",
    "Role",
    "Has Role",
    "Session",
    "File",
    "Version",
    "Error Log",
    "System Settings",
    "Singles",
    "Module Def",
    "Report",
    "Print Format",
    "Workspace",
    "Custom Field",
    "Property Setter",
    "User Permission",
    "API Key",
    "Deleted Document",
];

fn require_admin(user: &User) -> Result<()> {
    if user.has_role(SYSTEM_MANAGER_ROLE) {
        return Ok(());
    }
    Err(Error::Authz(AuthzError::PermissionDenied {
        action: "manage doctypes".to_string(),
        resource: "DocType".to_string(),
    }))
}

/// Reject a metadata body the engine could not serve.
fn validate(meta: &DoctypeMeta, pool: &Db) -> Result<()> {
    // The table name is the DocType name without whitespace; this validates the
    // characters and rejects an empty name at the same time.
    ferris_schema::migration::document_table(&meta.name)?;
    if meta.name.starts_with('_') || RESERVED.contains(&meta.name.as_str()) {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{}` is a framework-owned DocType name",
            meta.name
        ))));
    }
    if meta.fields.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "a DocType needs at least one field".to_string(),
        )));
    }

    let mut seen = HashSet::new();
    for field in &meta.fields {
        if field.fieldname.trim().is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "a field has an empty name".to_string(),
            )));
        }
        if !seen.insert(field.fieldname.as_str()) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "field `{}` is declared twice",
                field.fieldname
            ))));
        }
        if field.fieldtype == "Table" {
            let child = field
                .options
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    Error::Validation(ValidationError::Invalid(format!(
                        "Table field `{}` needs an `options` child DocType",
                        field.fieldname
                    )))
                })?;
            if ferris_schema::get_doctype(child).is_none() {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "Table field `{}` names child DocType `{child}`, which is not registered",
                    field.fieldname
                ))));
            }
            continue;
        }
        // Layout/virtual types return `None`; an unknown fieldtype is an error.
        ferris_schema::doctype_columns::field_column(field, pool.dialect())?;
    }

    if meta.is_child_table {
        for reserved in ["parent", "parenttype", "parentfield", "idx"] {
            if meta.fields.iter().any(|field| field.fieldname == reserved) {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "child DocType `{}` cannot declare the link column `{reserved}`",
                    meta.name
                ))));
            }
        }
    }
    Ok(())
}

/// Create a runtime `DocType`, materialize its table and register it.
///
/// # Errors
///
/// Non-admin caller, a compiled/reserved/duplicate name, an invalid field, or a
/// database failure while creating the table or storing the metadata.
pub async fn create(pool: &Db, user: &User, mut meta: DoctypeMeta) -> Result<DoctypeMeta> {
    require_admin(user)?;
    if ferris_schema::get_doctype(&meta.name).is_some() {
        return Err(Error::Document(DocumentError::AlreadyExists {
            doctype: "DocType".to_string(),
            name: meta.name,
        }));
    }
    validate(&meta, pool)?;
    meta.custom = true;
    materialize(pool, &meta).await?;
    Ok(meta)
}

/// Edit an existing **custom** `DocType`.
///
/// Adding a field adds a nullable column and backfills nothing; removing a field
/// stops serving it but leaves its column and data in place, so an accidental
/// edit cannot destroy rows.
///
/// # Errors
///
/// Non-admin caller, unknown or compiled `DocType`, a body whose name does not
/// match the path, an invalid field, or a database failure.
pub async fn update(
    pool: &Db,
    user: &User,
    name: &str,
    mut meta: DoctypeMeta,
) -> Result<DoctypeMeta> {
    require_admin(user)?;
    if meta.name != name {
        return Err(Error::Validation(ValidationError::Invalid(
            "the body name must match the path".to_string(),
        )));
    }
    let Some(existing) = ferris_schema::get_doctype(name) else {
        return Err(Error::Schema(SchemaError::DocTypeNotFound(
            name.to_string(),
        )));
    };
    if !existing.custom {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{name}` is declared in code and cannot be edited at runtime"
        ))));
    }
    validate(&meta, pool)?;
    meta.custom = true;
    materialize(pool, &meta).await?;
    Ok(meta)
}

/// Register, create the table and store the row.
///
/// Registration comes first so the table generator can see the declared fields;
/// the row is written last. A failure after registration leaves a table and an
/// in-process metadata but no stored row; the next boot's `sync_registry` writes
/// it, and the caller gets the error.
async fn materialize(pool: &Db, meta: &DoctypeMeta) -> Result<()> {
    ferris_schema::register_doctype(meta.clone());
    // The registry is the base `effective_meta` merges over, and the previous
    // shape may already be cached. Dropping the cache here is what makes an
    // added field reach `refresh_table`; otherwise the merge still reports the
    // field list from before the edit and the `ALTER TABLE` never runs.
    ferris_schema::CustomizationStore::invalidate_meta(&meta.name);
    DocumentEngine::new(pool.clone())
        .refresh_table(&meta.name)
        .await?;
    let report = DoctypeStore::new(pool)
        .sync(std::slice::from_ref(meta))
        .await;
    if report.warnings.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(ValidationError::Invalid(
            report.warnings.join("; "),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_core::value;
    use ferris_db::Stmt;
    use ferris_document::EngineContext;
    use ferris_schema::FieldMeta;

    fn admin() -> User {
        let mut user = User::system_user("admin@local".to_string(), "Admin".to_string());
        user.id = "admin@local".to_string();
        user.roles = vec![SYSTEM_MANAGER_ROLE.to_string()];
        user
    }

    fn reader() -> User {
        let mut user = User::system_user("reader@local".to_string(), "Reader".to_string());
        user.id = "reader@local".to_string();
        user
    }

    fn meta(name: &str) -> DoctypeMeta {
        DoctypeMeta {
            name: name.to_string(),
            fields: vec![
                FieldMeta {
                    fieldname: "title".to_string(),
                    fieldtype: "Data".to_string(),
                    ..Default::default()
                },
                FieldMeta {
                    fieldname: "qty".to_string(),
                    fieldtype: "Int".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_admin_creates_a_doctype_with_real_columns() {
        let pool = ferris_db::memory_db().await.unwrap();
        let created = create(&pool, &admin(), meta("AdminProbe"))
            .await
            .expect("the admin creates it");
        assert!(created.custom);

        let columns = pool
            .fetch_all(&Stmt::raw("PRAGMA table_info(\"tabAdminProbe\")"))
            .await
            .unwrap();
        let names: Vec<String> = columns
            .iter()
            .filter_map(|row| row.opt_str("name"))
            .collect();
        assert!(names.contains(&"title".to_string()), "{names:?}");
        assert!(names.contains(&"qty".to_string()), "{names:?}");

        // The table the builder created is the table the engine writes.
        let engine = DocumentEngine::new(pool.clone());
        let ctx = EngineContext::new("AdminProbe", "ADM-.#####")
            .with_meta(&created)
            .allow_anonymous(true);
        let document = engine
            .create(&ctx, value!({ "title": "probe", "qty": 4 }))
            .await
            .unwrap();
        let name = document.get("name").and_then(|v| v.as_str()).unwrap();
        let row = pool
            .fetch_one(&Stmt::new(
                "SELECT title, qty FROM tabAdminProbe WHERE name = ?",
                vec![ferris_core::Value::from(name)],
            ))
            .await
            .unwrap();
        assert_eq!(row.opt_str("title").as_deref(), Some("probe"));
        assert_eq!(row.opt_i64("qty"), Some(4));
    }

    #[tokio::test]
    async fn a_non_admin_cannot_create_a_doctype() {
        let pool = ferris_db::memory_db().await.unwrap();
        let error = create(&pool, &reader(), meta("DeniedProbe"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Authz(AuthzError::PermissionDenied { .. })
        ));
    }

    #[tokio::test]
    async fn a_reserved_or_duplicate_doctype_is_rejected() {
        let pool = ferris_db::memory_db().await.unwrap();
        // A framework-owned name is a validation error even when the compiled
        // registry is not linked into this test binary.
        let error = create(&pool, &admin(), meta("User")).await.unwrap_err();
        assert!(matches!(
            error,
            Error::Validation(ValidationError::Invalid(_))
        ));

        create(&pool, &admin(), meta("AdminDup")).await.unwrap();
        let error = create(&pool, &admin(), meta("AdminDup")).await.unwrap_err();
        assert!(matches!(
            error,
            Error::Document(DocumentError::AlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn a_child_doctype_created_first_serves_its_parent() {
        let pool = ferris_db::memory_db().await.unwrap();
        let child = DoctypeMeta {
            name: "AdminLine".to_string(),
            is_child_table: true,
            fields: vec![
                FieldMeta {
                    fieldname: "item".to_string(),
                    fieldtype: "Data".to_string(),
                    ..Default::default()
                },
                FieldMeta {
                    fieldname: "amount".to_string(),
                    fieldtype: "Currency".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        create(&pool, &admin(), child).await.unwrap();

        let parent = DoctypeMeta {
            name: "AdminOrder".to_string(),
            fields: vec![
                FieldMeta {
                    fieldname: "title".to_string(),
                    fieldtype: "Data".to_string(),
                    ..Default::default()
                },
                FieldMeta {
                    fieldname: "lines".to_string(),
                    fieldtype: "Table".to_string(),
                    options: Some("AdminLine".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        create(&pool, &admin(), parent.clone()).await.unwrap();

        let columns = pool
            .fetch_all(&Stmt::raw("PRAGMA table_info(\"tabAdminLine\")"))
            .await
            .unwrap();
        assert!(
            columns
                .iter()
                .any(|row| row.opt_str("name").as_deref() == Some("parent"))
        );

        let engine = DocumentEngine::new(pool.clone());
        let ctx = EngineContext::new("AdminOrder", "ORD-.#####")
            .with_meta(&parent)
            .allow_anonymous(true);
        let order = engine
            .create(
                &ctx,
                value!({ "title": "order", "lines": [{"item": "a", "amount": 1.5}] }),
            )
            .await
            .unwrap();
        let order_name = order.get("name").and_then(|v| v.as_str()).unwrap();
        let rows = pool
            .fetch_all(&Stmt::new(
                "SELECT item, parent, parentfield FROM tabAdminLine",
                Vec::new(),
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].opt_str("item").as_deref(), Some("a"));
        assert_eq!(rows[0].opt_str("parent").as_deref(), Some(order_name));
        assert_eq!(rows[0].opt_str("parentfield").as_deref(), Some("lines"));
    }
    #[tokio::test]
    async fn adding_a_field_adds_a_nullable_column() {
        let pool = ferris_db::memory_db().await.unwrap();
        let mut first = meta("AdminGrow");
        create(&pool, &admin(), first.clone()).await.unwrap();

        first.fields.push(FieldMeta {
            fieldname: "note".to_string(),
            fieldtype: "Text".to_string(),
            ..Default::default()
        });
        update(&pool, &admin(), "AdminGrow", first).await.unwrap();

        let columns = pool
            .fetch_all(&Stmt::raw("PRAGMA table_info(\"tabAdminGrow\")"))
            .await
            .unwrap();
        assert!(
            columns
                .iter()
                .any(|row| row.opt_str("name").as_deref() == Some("note"))
        );
    }
}
