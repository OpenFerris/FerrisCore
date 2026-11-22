//! Document trait and core document functionality

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferris_core::{DocStatus, Result as CoreResult, Value};
use ferris_db::Db;
use ferris_schema::registry::get_doctype;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Document trait - implemented by all doctype structs
#[async_trait]
pub trait Document: Send + Sync + Debug + Serialize + for<'de> Deserialize<'de> + Clone {
    type Key: Send + Sync + Debug + Serialize + for<'de> Deserialize<'de>;
    type Child: ChildDocument;

    // Static metadata
    const DOCTYPE: &'static str;
    const MODULE: &'static str;
    const IS_SUBMITTABLE: bool;
    const IS_TREE: bool;
    const IS_VIRTUAL: bool;
    const IS_SINGLE: bool;
    const IS_CHILD_TABLE: bool;
    const NAMING_SERIES: &'static str;
    /// Naming strategy, mirroring the doctype's `naming_type` declaration.
    /// `None` keeps the implicit resolution: series if `NAMING_SERIES` is not
    /// empty, UUID otherwise (see [`ferris_schema::contract::NamingType`]).
    const NAMING_TYPE: Option<ferris_schema::contract::NamingType> = None;
    /// Field the name is read from when `NAMING_TYPE` is
    /// [`ferris_schema::contract::NamingType::Field`].
    const NAMING_FIELD: Option<&'static str> = None;
    const WORKFLOW: Option<&'static str>;

    // Permissions
    fn permissions() -> &'static [ferris_core::PermissionRule];

    // Child table fields
    fn child_table_fields() -> &'static [&'static str];

    // Generate name from naming series
    fn naming_series() -> String;

    // Validation
    fn validate(&self) -> CoreResult<()> {
        Ok(())
    }

    /// Get a field value by name (implementations may override).
    ///
    /// Returns an owned [`Value`] so typed DocType structs can convert their
    /// fields on demand (via serde) without storing a dynamic map.
    fn get_field(&self, _field: &str) -> Option<Value> {
        None
    }

    // CRUD operations
    async fn insert(&mut self, db: &Db) -> CoreResult<String>;
    async fn save(&mut self, db: &Db) -> CoreResult<()>;
    async fn delete(&self, db: &Db) -> CoreResult<()>;
    async fn submit(&mut self, db: &Db) -> CoreResult<()>;
    async fn cancel(&mut self, db: &Db) -> CoreResult<()>;
    async fn amend(&self, db: &Db) -> CoreResult<String>;

    // Getters
    fn get_key(&self) -> &Self::Key;
    fn get_name(&self) -> &str;
    fn get_doctype(&self) -> &str;
    fn get_owner(&self) -> &str;
    fn get_creation(&self) -> &DateTime<Utc>;
    fn get_modified(&self) -> &DateTime<Utc>;
    fn get_modified_by(&self) -> &str;
    fn get_docstatus(&self) -> DocStatus;
    fn get_parent(&self) -> Option<&str>;
    fn get_parentfield(&self) -> Option<&str>;
    fn get_parenttype(&self) -> Option<&str>;
    fn get_idx(&self) -> i32;

    // Setters
    fn set_name(&mut self, name: String);
    fn set_owner(&mut self, owner: String);
    fn set_modified(&mut self, modified: DateTime<Utc>);
    fn set_modified_by(&mut self, modified_by: String);
    fn set_docstatus(&mut self, docstatus: DocStatus);
    fn set_parent(&mut self, parent: Option<String>);
    fn set_parentfield(&mut self, parentfield: Option<String>);
    fn set_parenttype(&mut self, parenttype: Option<String>);
    fn set_idx(&mut self, idx: i32);
}

/// Extension trait for Document with convenience methods
pub trait DocumentExt: Document {
    /// Get the DocType metadata
    fn meta() -> Option<std::sync::Arc<ferris_schema::registry::DoctypeMeta>> {
        get_doctype(Self::DOCTYPE)
    }

    /// Check if document is new (not saved)
    fn is_new(&self) -> bool {
        self.get_name().is_empty()
    }

    /// Check if document is submitted
    fn is_submitted(&self) -> bool {
        self.get_docstatus() == DocStatus::Submitted
    }

    /// Check if document is cancelled
    fn is_cancelled(&self) -> bool {
        self.get_docstatus() == DocStatus::Cancelled
    }

    /// Check if document is draft
    fn is_draft(&self) -> bool {
        self.get_docstatus() == DocStatus::Draft
    }
}

impl<T: Document> DocumentExt for T {}

/// Child document trait
#[async_trait]
pub trait ChildDocument:
    Send + Sync + Debug + Serialize + for<'de> Deserialize<'de> + Clone
{
    fn get_parent(&self) -> Option<String>;
    fn set_parent(&mut self, parent: String);
    fn get_parentfield(&self) -> Option<String>;
    fn set_parentfield(&mut self, parentfield: String);
    fn get_parenttype(&self) -> Option<String>;
    fn set_parenttype(&mut self, parenttype: String);
    fn get_idx(&self) -> i32;
    fn set_idx(&mut self, idx: i32);
}
