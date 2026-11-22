//! Engine-backed storage for runtime role permissions — the framework's
//! analogue of Frappe's `DocPerm` rows.
//!
//! # Why this exists
//!
//! Frappe's *Role Permissions Builder* writes `DocPerm` rows. They are the
//! unit of "who may do what on a `DocType`", they live in the database, and the
//! desk edits them without touching Python. Ferris used to have no such layer:
//! the only way an operator could change a `DocType`'s permissions was to edit
//! its JSON or install a raw Cedar-shaped policy document. This store is the
//! remaining surface — a persisted, `DocPerm`-shaped set of documents that
//! `ferris-api::authz::install_policies_with_role_permissions` merges into the
//! live engine on every boot and every edit.
//!
//! # Storage
//!
//! Rows are [`DocPerm`](crate::doctypes::docperm_meta) documents: the document
//! engine is the single source of truth, so a row written here and one written
//! through the REST path are the same row, and the engine applies validation,
//! hooks and field permissions once. The store acts as `System Manager` (see
//! `store_support.rs`) because it runs on trusted framework paths, and it
//! keys a document by the natural key `(doctype, role, permlevel)` so a re-save
//! overwrites instead of duplicating. The row shape mirrors
//! [`crate::permissions::PermissionRule`] (the two differ only by `doc_type`,
//! which is the storage key).
//!
//! The engine translation (`doc_type_policy`, `if_owner`, `apply_to_all_doctypes`,
//! `permlevel` filtering) lives in `ferris-core` and is shared, so a persisted
//! row and a declared rule produce byte-identical statements.

use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::permissions::PermissionRule;
use crate::store_support::system_context;

/// The DocType the store persists.
const DOCTYPE: &str = "DocPerm";
/// The engine field holding the DocType a rule applies to.
///
/// Named `doc_type`, not `doctype`: every document already carries its own
/// `doctype` (its DocType name), so the column the store calls `doctype` maps
/// onto this field.
const FIELD_DOCTYPE: &str = "doc_type";

/// One persisted `DocPerm` row: the same fields as [`PermissionRule`] plus the
/// `doctype` they apply to.
///
/// `serde(default)` keeps the wire format desk-friendly: a POST may name only
/// the rights it is turning on under a `(doctype, role)` key; everything else
/// reads as its enabled-by-default false.
///
/// `struct_excessive_bools` is allowed deliberately: the row must mirror
/// Frappe's `DocPerm` model field-for-field, and all 13 rights are plain
/// switches there too, so collapsing them into enums would obscure the parity.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocPermRow {
    pub doctype: String,
    pub role: String,
    pub permlevel: u8,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub delete: bool,
    pub submit: bool,
    pub cancel: bool,
    pub amend: bool,
    pub print: bool,
    pub email: bool,
    pub share: bool,
    pub report: bool,
    pub import: bool,
    pub export: bool,
    pub if_owner: bool,
    pub apply_to_all_doctypes: bool,
}

impl DocPermRow {
    /// The rights tuple this row expresses, ready for `doc_type_policy`.
    ///
    /// `doctype` is dropped: the engine translation groups rows per `DocType`,
    /// so the grouping key is not part of the rule itself.
    #[must_use]
    pub fn to_rule(&self) -> PermissionRule {
        PermissionRule {
            role: self.role.clone(),
            read: self.read,
            write: self.write,
            create: self.create,
            delete: self.delete,
            submit: self.submit,
            cancel: self.cancel,
            amend: self.amend,
            print: self.print,
            email: self.email,
            share: self.share,
            report: self.report,
            import: self.import,
            export: self.export,
            permlevel: self.permlevel,
            if_owner: self.if_owner,
            apply_to_all_doctypes: self.apply_to_all_doctypes,
            custom_permissions: Vec::new(),
        }
    }

    /// The document name for this row's natural key.
    ///
    /// `(doctype, role, permlevel)` is the key Frappe's builder uses to let one
    /// role hold several rules for a `DocType` (one per permission level). The
    /// engine keys a document by its `name`, so the natural key is encoded into
    /// one — the encoding is a storage detail, not part of the row.
    fn document_name(&self) -> String {
        format!("{}|{}|{}", self.doctype, self.role, self.permlevel)
    }

    /// The engine document for this row.
    fn to_document(&self) -> Value {
        let mut map = ferris_core::Map::new();
        map.insert("name".to_string(), Value::from(self.document_name()));
        map.insert(
            FIELD_DOCTYPE.to_string(),
            Value::from(self.doctype.as_str()),
        );
        map.insert("role".to_string(), Value::from(self.role.as_str()));
        map.insert(
            "permlevel".to_string(),
            Value::from(i64::from(self.permlevel)),
        );
        for (field, right) in [
            ("read", self.read),
            ("write", self.write),
            ("create", self.create),
            ("delete", self.delete),
            ("submit", self.submit),
            ("cancel", self.cancel),
            ("amend", self.amend),
            ("print", self.print),
            ("email", self.email),
            ("share", self.share),
            ("report", self.report),
            ("import", self.import),
            ("export", self.export),
            ("if_owner", self.if_owner),
            ("apply_to_all_doctypes", self.apply_to_all_doctypes),
        ] {
            map.insert(field.to_string(), Value::from(right));
        }
        Value::Object(map)
    }

    /// Decode a `DocPerm` document back into a row.
    fn from_document(document: &Value) -> Result<Self> {
        let field_str = |name: &str| {
            document
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let right = |name: &str| document.get(name).and_then(Value::as_bool).unwrap_or(false);
        let permlevel = document
            .get("permlevel")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .clamp(0, i64::from(u8::MAX)) as u8;
        Ok(Self {
            doctype: field_str(FIELD_DOCTYPE),
            role: field_str("role"),
            permlevel,
            read: right("read"),
            write: right("write"),
            create: right("create"),
            delete: right("delete"),
            submit: right("submit"),
            cancel: right("cancel"),
            amend: right("amend"),
            print: right("print"),
            email: right("email"),
            share: right("share"),
            report: right("report"),
            import: right("import"),
            export: right("export"),
            if_owner: right("if_owner"),
            apply_to_all_doctypes: right("apply_to_all_doctypes"),
        })
    }
}

/// Persisted role-permission storage.
#[derive(Debug, Clone)]
pub struct DocPermStore {
    engine: DocumentEngine,
}

impl DocPermStore {
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self {
            engine: DocumentEngine::new(db),
        }
    }

    /// Ensure the `DocPerm` table and its indexes exist. Idempotent; called
    /// before any other operation and again at boot so a fresh database never
    /// lacks the table.
    ///
    /// # Errors
    ///
    /// Database errors from creating the table or its indexes.
    pub async fn init(&self) -> Result<()> {
        // Register the meta first so `ensure_table` materializes any declared
        // indexes, exactly as the boot-time registry does.
        ferris_schema::register_doctype(crate::doctypes::docperm_meta());
        self.engine.ensure_table(DOCTYPE).await.map(|_| ())
    }

    /// Every row, in key order. One doctype may repeat with different roles and
    /// permission levels; the engine translation handles that on its own.
    ///
    /// # Errors
    ///
    /// Database or document-decoding errors.
    pub async fn list_all(&self) -> Result<Vec<DocPermRow>> {
        let mut rows = self.fetch(&ListQuery::default()).await?;
        rows.sort_by(|a, b| {
            (&a.doctype, &a.role, a.permlevel).cmp(&(&b.doctype, &b.role, b.permlevel))
        });
        Ok(rows)
    }

    /// The rows for one doctype.
    ///
    /// # Errors
    ///
    /// Database or document-decoding errors.
    pub async fn list(&self, doctype: &str) -> Result<Vec<DocPermRow>> {
        let query = ListQuery {
            filters: HashMap::from([(FIELD_DOCTYPE.to_string(), Value::from(doctype))]),
            ..ListQuery::default()
        };
        let mut rows = self.fetch(&query).await?;
        rows.sort_by(|a, b| (&a.role, a.permlevel).cmp(&(&b.role, b.permlevel)));
        Ok(rows)
    }

    /// The number of stored rows.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn count(&self) -> Result<u64> {
        let query = ListQuery {
            limit: Some(1),
            ..ListQuery::default()
        };
        let (_, total) = self.list_via(&query).await?;
        Ok(total)
    }

    /// Insert or overwrite one row. The natural key is
    /// `(doctype, role, permlevel)`, which matches how Frappe's builder lets
    /// one role hold several rules for a `DocType` (one per permission level).
    ///
    /// # Errors
    ///
    /// Validation, permission or storage errors from the engine.
    pub async fn upsert(&self, row: &DocPermRow) -> Result<()> {
        if row.doctype.trim().is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "a DocPerm row requires a DocType".to_string(),
            )));
        }
        let meta = crate::doctypes::docperm_meta();
        let ctx = system_context(&meta);
        let document = row.to_document();
        let name = row.document_name();
        match self.engine.get(&ctx, &name).await {
            Ok(_) => {
                self.engine.update(&ctx, &name, document).await?;
            }
            Err(Error::Document(DocumentError::NotFound { .. })) => {
                self.engine.create(&ctx, document).await?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// Remove every row for a `(doctype, role)` pair, across permission levels.
    ///
    /// # Errors
    ///
    /// Database, permission or storage errors from the engine.
    pub async fn delete(&self, doctype: &str, role: &str) -> Result<()> {
        let meta = crate::doctypes::docperm_meta();
        let ctx = system_context(&meta);
        for row in self.list(doctype).await? {
            if row.role == role {
                self.engine.delete(&ctx, &row.document_name()).await?;
            }
        }
        Ok(())
    }

    /// List through the engine and decode each document.
    async fn fetch(&self, query: &ListQuery) -> Result<Vec<DocPermRow>> {
        let (documents, _) = self.list_via(query).await?;
        documents.iter().map(DocPermRow::from_document).collect()
    }

    /// The engine `list` call, shared by [`Self::fetch`] and [`Self::count`].
    ///
    /// Every store read asks for all rows: the DocPerm table is small, and the
    /// row-by-row filtering the engine would otherwise page over buys nothing.
    async fn list_via(&self, query: &ListQuery) -> Result<(Vec<Value>, u64)> {
        let meta = crate::doctypes::docperm_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            limit: query.limit.or(Some(u32::MAX)),
            ..query.clone()
        };
        self.engine.list(&ctx, &query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        ferris_db::memory_db().await.unwrap()
    }

    fn base_row() -> DocPermRow {
        DocPermRow {
            doctype: "ToDo".to_string(),
            role: "Sales".to_string(),
            read: true,
            ..DocPermRow::default()
        }
    }

    #[tokio::test]
    async fn upsert_round_trips_and_overwrites_on_the_natural_key() {
        let store = DocPermStore::new(db().await);
        store.init().await.unwrap();

        store.upsert(&base_row()).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1);

        // Same (doctype, role, permlevel): overwrite, not duplicate.
        let mut updated = base_row();
        updated.write = true;
        store.upsert(&updated).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1);

        let rows = store.list_all().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].read && rows[0].write && !rows[0].delete);

        drop(store);
    }

    #[tokio::test]
    async fn roles_and_permlevels_coexist_under_one_doctype() {
        let store = DocPermStore::new(db().await);
        store.init().await.unwrap();

        let sales = base_row();
        store.upsert(&sales).await.unwrap();
        let mut sales_l1 = sales.clone();
        sales_l1.permlevel = 1;
        sales_l1.report = true;
        store.upsert(&sales_l1).await.unwrap();
        let mut hr = base_row();
        hr.role = "HR".to_string();
        store.upsert(&hr).await.unwrap();

        assert_eq!(store.count().await.unwrap(), 3);

        let todo = store.list("ToDo").await.unwrap();
        assert_eq!(todo.len(), 3);

        // Deleting the role removes only its rows, both permlevels.
        store.delete("ToDo", "Sales").await.unwrap();
        let todo = store.list("ToDo").await.unwrap();
        assert_eq!(todo.len(), 1);
        assert_eq!(todo[0].role, "HR");
    }

    #[tokio::test]
    async fn row_to_rule_maps_permission_level_and_ownership() {
        let row = DocPermRow {
            permlevel: 2,
            if_owner: true,
            apply_to_all_doctypes: true,
            submit: true,
            ..base_row()
        };
        let rule = row.to_rule();
        assert_eq!(rule.role, "Sales");
        assert_eq!(rule.permlevel, 2);
        assert!(rule.if_owner);
        assert!(rule.apply_to_all_doctypes);
        assert!(rule.submit);
        assert!(!rule.cancel);
    }

    #[tokio::test]
    async fn init_is_idempotent_and_an_empty_store_lists_nothing() {
        let store = DocPermStore::new(db().await);
        store.init().await.unwrap();
        store.init().await.unwrap();
        assert_eq!(store.count().await.unwrap(), 0);
        assert!(store.list_all().await.unwrap().is_empty());
    }
}
