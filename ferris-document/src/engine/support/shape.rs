//! The physical shape of one `DocType` at write/read time.
//!
//! Columns are the only source of truth for a document (ADR 0001). A *shape*
//! is the resolved list of columns and child tables the engine has to touch to
//! persist or reconstruct a `Value`:
//!
//! * the eight system columns of [`ferris_schema::migration::document_columns`];
//! * every declared field that owns a real column, read from the **effective**
//!   metadata ([`ferris_schema::effective_meta`]) so a custom field is not lost;
//! * one [`TableShape`] per `Table` field, carrying the child's own effective
//!   metadata and columns.
//!
//! [`DocShape::resolve`] goes through `effective_meta`, which merges the
//! compiled declaration, the site's `Custom Field` rows and its
//! `Property Setter` overrides. A `DocType` with no registry entry resolves to
//! an empty shape: the table keeps only its system columns, so a test or a
//! runtime name without a declaration still works without inventing columns.

use super::super::*;
use super::{is_password_field, is_secret_field};
use ferris_core::{Result, Value};
use ferris_db::{Dialect, Row};
use ferris_schema::{DoctypeMeta, FieldMeta};

/// The columns every document table owns, in read/write order.
pub(in crate::engine) const SYSTEM_COLUMNS: [&str; 8] = [
    "name",
    "owner",
    "docstatus",
    "creation",
    "modified",
    "modified_by",
    "amended_from",
    "workflow_state",
];

/// One `Table` field and the child DocType it stores.
pub(in crate::engine) struct TableShape {
    /// The fieldname on the parent (`items`, `taxes`, …).
    pub fieldname: String,
    /// The child `DocType` name (`Sales Invoice Item`, …).
    pub child: String,
    /// The child's declared fields with a physical column.
    pub columns: Vec<FieldMeta>,
}

/// The resolved columns and child tables of one `DocType`.
pub(in crate::engine) struct DocShape {
    /// The effective metadata, or `None` for an unregistered DocType.
    pub meta: Option<DoctypeMeta>,
    /// Declared fields with a physical column in this dialect.
    pub columns: Vec<FieldMeta>,
    /// `Table` fields with a resolvable child.
    pub tables: Vec<TableShape>,
}

impl DocShape {
    /// An unregistered DocType: only the system columns exist.
    const fn empty() -> Self {
        Self {
            meta: None,
            columns: Vec::new(),
            tables: Vec::new(),
        }
    }

    /// Resolve the effective shape of `doctype` against `db`.
    ///
    /// `effective_meta` is cached per database and invalidated by customization
    /// writes, so this is a metadata lookup plus the classification of each
    /// declared field into a column. Callers resolve **once per operation** and
    /// pass the shape down: a read and the write it drives must agree on the
    /// same columns, and re-resolving inside an open transaction would ask the
    /// customization store for a second connection.
    ///
    /// When the registry has no entry, `fallback` (the caller's context
    /// metadata, if any) supplies the columns; with neither, the table keeps
    /// only its system columns.
    #[allow(clippy::result_large_err)] // `ferris_core::Error` is the shared taxonomy
    pub(in crate::engine) async fn resolve(
        db: &Db,
        doctype: &str,
        dialect: Dialect,
        fallback: Option<&DoctypeMeta>,
    ) -> Result<Self> {
        let meta = match ferris_schema::effective_meta(db, doctype).await? {
            Some(meta) => Some(meta),
            None => fallback.cloned(),
        };
        let Some(meta) = meta else {
            return Ok(Self::empty());
        };
        let columns = stored_columns(&meta, dialect);
        let mut tables = Vec::new();
        for field in &meta.fields {
            if normalized_fieldtype(field) != "Table" {
                continue;
            }
            let Some(child) = field
                .options
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let Some(child_meta) = ferris_schema::effective_meta(db, child).await? else {
                continue;
            };
            tables.push(TableShape {
                fieldname: field.fieldname.clone(),
                child: child.to_string(),
                columns: stored_columns(&child_meta, dialect),
            });
        }
        Ok(Self {
            meta: Some(meta),
            columns,
            tables,
        })
    }

    /// The fieldnames whose values are credentials, not document text.
    ///
    /// A stored password hash or an encrypted secret must not be copied into the
    /// full-text index: searching them is never useful, they bloat the index on
    /// every write, and a second table holding a credential is one more place to
    /// leak it from. Child columns are included, because their values travel in
    /// the same document.
    pub(in crate::engine) fn sensitive_fieldnames(&self) -> std::collections::HashSet<&str> {
        let mut names = std::collections::HashSet::new();
        for field in &self.columns {
            if is_password_field(field) || is_secret_field(field) {
                names.insert(field.fieldname.as_str());
            }
        }
        for table in &self.tables {
            for field in &table.columns {
                if is_password_field(field) || is_secret_field(field) {
                    names.insert(field.fieldname.as_str());
                }
            }
        }
        names
    }

    /// The `SELECT` list that reconstructs one document.
    pub(in crate::engine) fn select_list(&self, dialect: Dialect) -> Result<String> {
        let mut names: Vec<&str> = SYSTEM_COLUMNS.to_vec();
        names.extend(self.columns.iter().map(|field| field.fieldname.as_str()));
        names
            .into_iter()
            .map(|name| q(dialect, name))
            .collect::<Result<Vec<_>>>()
            .map(|quoted| quoted.join(", "))
    }

    /// Rebuild a document `Value` from one row of [`Self::select_list`].
    ///
    /// A `NULL` column contributes no key: the field was never set, which is
    /// what the Rust evaluator expects. The three engine-stamped columns are
    /// the exception — `docstatus` is a word, and `creation`/`modified` are
    /// RFC 3339 in the document and epoch seconds on disk.
    pub(in crate::engine) fn row_to_document(&self, row: &Row) -> Result<Value> {
        let mut map = ferris_core::Map::new();
        map.insert(
            "name".to_string(),
            Value::from(row.opt_str("name").unwrap_or_default()),
        );
        if let Some(owner) = row.opt_str("owner") {
            map.insert("owner".to_string(), Value::from(owner));
        }
        let status = row.opt_i64("docstatus").unwrap_or(0);
        map.insert(
            "docstatus".to_string(),
            Value::from(docstatus_str(docstatus_from_code(status))),
        );
        for stamp in ["creation", "modified"] {
            if let Some(seconds) = row.opt_i64(stamp) {
                map.insert(stamp.to_string(), Value::from(super::rfc3339(seconds)?));
            }
        }
        for column in ["modified_by", "amended_from", "workflow_state"] {
            if let Some(text) = row.opt_str(column) {
                map.insert(column.to_string(), Value::from(text));
            }
        }
        for field in &self.columns {
            if let Some(value) = column_to_value(field, row) {
                map.insert(field.fieldname.clone(), value);
            }
        }
        Ok(Value::Object(map))
    }
}

impl TableShape {
    /// The `SELECT` list for one child row: its name and link columns plus the
    /// child's declared fields.
    pub(in crate::engine) fn select_list(&self, dialect: Dialect) -> Result<String> {
        let mut names = vec!["name", "parent", "parenttype", "parentfield", "idx"];
        names.extend(self.columns.iter().map(|field| field.fieldname.as_str()));
        names
            .into_iter()
            .map(|name| q(dialect, name))
            .collect::<Result<Vec<_>>>()
            .map(|quoted| quoted.join(", "))
    }

    /// Rebuild one child row from [`Self::select_list`].
    pub(in crate::engine) fn row_to_child(&self, row: &Row) -> Value {
        let mut map = ferris_core::Map::new();
        for column in ["name", "parent", "parenttype", "parentfield"] {
            if let Some(text) = row.opt_str(column) {
                map.insert(column.to_string(), Value::from(text));
            }
        }
        if let Some(index) = row.opt_i64("idx") {
            map.insert("idx".to_string(), Value::from(index));
        }
        for field in &self.columns {
            if let Some(value) = column_to_value(field, row) {
                map.insert(field.fieldname.clone(), value);
            }
        }
        Value::Object(map)
    }
}

/// The four link columns of a child table.
///
/// They are engine bookkeeping only on a child: a normal `DocType` that
/// declares a field named `parent` owns a real column of its own (Frappe's
/// `Custom DocPerm` is the live case), so the filter must not be global.
const CHILD_LINK_COLUMNS: [&str; 4] = ["parent", "parenttype", "parentfield", "idx"];

/// Declared fields of `meta` that own a physical column, in metadata order.
///
/// System fields are excluded: their column already exists and is written by
/// the engine, not from the document map. On a child table the four link
/// columns (`parent`, `parenttype`, `parentfield`, `idx`) are engine-owned and
/// are skipped too; on a normal `DocType` a field with one of those names is a
/// declared field and keeps its column. Table/layout/virtual fields have no
/// column ([`ferris_schema::doctype_columns::field_column`]).
pub(in crate::engine) fn stored_columns(meta: &DoctypeMeta, dialect: Dialect) -> Vec<FieldMeta> {
    let mut seen = std::collections::HashSet::new();
    meta.fields
        .iter()
        .filter(|field| !SYSTEM_COLUMNS.contains(&field.fieldname.as_str()))
        .filter(|field| {
            !meta.is_child_table || !CHILD_LINK_COLUMNS.contains(&field.fieldname.as_str())
        })
        .filter(|field| seen.insert(field.fieldname.clone()))
        .filter(|field| {
            ferris_schema::doctype_columns::field_column(field, dialect)
                .ok()
                .flatten()
                .is_some()
        })
        .cloned()
        .collect()
}

/// A money column's value: integral values come back as integers.
#[allow(clippy::cast_possible_truncation)] // guarded by the 2^53 bound below
fn currency_value(number: f64) -> Value {
    // 2^53 is the largest magnitude an `f64` represents exactly; above it the
    // integer conversion would be a guess.
    if number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_992.0 {
        Value::from(number as i64)
    } else {
        Value::from(number)
    }
}

/// The canonical fieldtype spelling of `field`.
pub(in crate::engine) fn normalized_fieldtype(field: &FieldMeta) -> &str {
    ferris_schema::fieldtype::canonical(&field.fieldtype).unwrap_or_else(|| field.fieldtype.trim())
}

/// A copy of `ctx` whose `meta` is the effective declaration.
///
/// The engine resolves metadata once per operation; every helper that reads
/// `ctx.meta` (validation, the post-submit lock, field permissions, workflows)
/// then sees the custom fields the stored row actually has.
pub(in crate::engine) fn effective_context<'m>(
    ctx: &EngineContext<'_>,
    meta: &'m DoctypeMeta,
) -> EngineContext<'m> {
    EngineContext {
        meta: Some(meta),
        doctype: ctx.doctype.clone(),
        naming_series: ctx.naming_series.clone(),
        naming_type: ctx.naming_type,
        naming_field: ctx.naming_field.clone(),
        user: ctx.user.clone(),
        allow_anonymous: ctx.allow_anonymous,
        allow_explicit_name: ctx.allow_explicit_name,
        suppress_events: ctx.suppress_events,
        reveal_password_hash: ctx.reveal_password_hash,
        reveal_secrets: ctx.reveal_secrets,
    }
}

/// A document's `docstatus` word from the `SMALLINT` column.
pub(in crate::engine) fn docstatus_from_code(code: i64) -> DocStatus {
    match code {
        1 => DocStatus::Submitted,
        2 => DocStatus::Cancelled,
        _ => DocStatus::Draft,
    }
}

/// One column's value as the document stores it, or `None` for `NULL`.
///
/// `NULL` is preserved as absence: a column that was never set must not become
/// `0` or `""`, or the reconstructed document would disagree with what was
/// written.
pub(in crate::engine) fn column_to_value(field: &FieldMeta, row: &Row) -> Option<Value> {
    let name = field.fieldname.as_str();
    match normalized_fieldtype(field) {
        "Int" => row.opt_i64(name).map(Value::from),
        "Float" => row.opt_f64(name).map(Value::from),
        // Money is exact on disk but the document stores the JSON number the
        // caller sent. An integral value round-trips as an integer, so a
        // `Currency` of `100` does not come back as `100.0`.
        "Currency" | "Percent" => row.opt_decimal_f64(name).map(currency_value),
        "Check" => row.opt_bool(name).map(Value::from),
        "Date" => row.opt_date(name).map(Value::from),
        "Datetime" => row.opt_datetime(name).map(Value::from),
        "JSON" => row
            .opt_str(name)
            .and_then(|text| serde_json::from_str(&text).ok()),
        // A `Geolocation` is either a GeoJSON object or a `"lat,lon"` string.
        // The column keeps the object as JSON text; returning that text as a
        // string would break the fieldtype contract the Desk renders.
        "Geolocation" => row.opt_str(name).map(|text| json_object_or_string(&text)),
        _ => row.opt_str(name).map(Value::from),
    }
}

/// The JSON value `text` spells, when it is an object or an array; the text
/// itself otherwise.
///
/// Only a structured document is restored: a plain string that happens to be
/// valid JSON (a number, `true`, a quoted string) stays a string, so a link or a
/// text value cannot change type on a round trip.
fn json_object_or_string(text: &str) -> Value {
    let trimmed = text.trim_start();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
    {
        return parsed;
    }
    Value::from(text)
}
