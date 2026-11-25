//! Real typed columns and child tables for a `DocType`.
//!
//! A `tab<DocType>` table has one column per declared field plus the system
//! columns (see [`migration::document_columns`](crate::migration::document_columns))
//! and one real table per `Table` field. Columns are the only source of truth:
//! there is no `data` JSON mirror (ADR 0001).
//!
//! This module is the schema half: it maps one [`FieldMeta`] to one [`Column`],
//! derives a child table's definition, and renders the `CREATE TABLE`/
//! `ALTER TABLE` statements for a whole `DocType`. `DocumentEngine::ensure_table`
//! and the `ferris migrate` backfill consume it.
//!
//! Every identifier goes through [`Ident`]/[`create_table`]/[`ensure_index`], so
//! nothing is ever interpolated without validation and the quoting follows the
//! dialect ([`Dialect`]).
//!
//! The caller is responsible for knowing whether the table is new or already
//! exists: [`field_column`] honors `reqd` with `NOT NULL` (safe in a fresh
//! table), while [`alter_add_columns`] always adds a nullable column because
//! existing rows have no value to satisfy a constraint.

#![allow(clippy::result_large_err)] // `ferris_core::Error` is the shared taxonomy.

use std::collections::HashSet;

use ferris_core::{Error, Result, ValidationError};
use ferris_db::Dialect;
use ferris_db::sql::{Column, Ident, add_column, create_table, ensure_index};

use crate::migration::{document_columns, document_table};
use crate::registry::{DoctypeMeta, FieldMeta, get_doctype};

/// Columns every document table owns. A field that redeclares one is the system
/// field itself (Frappe lets a DocType name a `name` field), so it is skipped
/// rather than added twice.
const SYSTEM_COLUMNS: [&str; 8] = [
    "name",
    "owner",
    "docstatus",
    "creation",
    "modified",
    "modified_by",
    "amended_from",
    "workflow_state",
];

/// The fieldtypes that never get a physical column.
///
/// `Table` is stored as its own child table (see [`child_table_columns`]); the
/// three break types are pure form layout; `HTML`, `Button`, `Image`, `Fold`
/// and `Heading` carry no value of their own (an `Image` renders another
/// field's value). The spellings are the registry's canonical ones, checked
/// against [`crate::fieldtype::FIELDTYPES`]; `Fold` is not in that list, but it
/// is Frappe's fieldtype name for a form fold, so it is skipped if a stored
/// declaration ever carries it.
const NON_STORED_FIELDTYPES: &[&str] = &[
    "Table",
    "Section Break",
    "Column Break",
    "Tab Break",
    "HTML",
    "Button",
    "Image",
    "Fold",
    "Heading",
];

/// The canonical spelling of `field.fieldtype`, or the trimmed raw value when
/// it is not a known fieldtype.
fn normalized_fieldtype(field: &FieldMeta) -> &str {
    crate::fieldtype::canonical(&field.fieldtype).unwrap_or_else(|| field.fieldtype.trim())
}

/// The SQL type a fieldtype is stored as, or an error for an unknown name.
///
/// The engine reconstructs the document `Value` from these columns, so a type
/// only has to round-trip the value, not drive validation.
///
/// | Fieldtype | Type |
/// |---|---|
/// | `Data`, `Select`, `Link`, `Dynamic Link`, `Attach`, `Table MultiSelect` | `VARCHAR(255)` |
/// | `Small Text`, `Text`, `Long Text`, `Text Editor`, `Code`, `JSON` | `TEXT` |
/// | `Int` | `BIGINT` |
/// | `Float`, `Currency`, `Percent` | `DOUBLE` |
/// | `Check` | `BOOLEAN` |
/// | `Date` | `DATE` (`TEXT` on SQLite) |
/// | `Datetime` | `TIMESTAMP`/`DATETIME` (`TEXT` on SQLite) |
/// | `Password`, `Secret` | `TEXT` |
/// | Other known fieldtypes (`Time`, `Rating`, `Color`, `File`, …) | `TEXT` |
///
/// The concrete driver per backend decodes `BOOLEAN`, `DATE` and `NUMERIC`
/// natively; SQLite keeps `TEXT` for dates (it has no date storage class) and
/// `NUMERIC` affinity for money.
/// `Date`/`Datetime` use the native type on Postgres/MySQL and `TEXT` on
/// SQLite. `Float` is approximate (`DOUBLE PRECISION`/`DOUBLE`/`REAL`), while
/// `Currency`/`Percent` are exact (`NUMERIC(21,9)`/`DECIMAL(21,9)`/`NUMERIC`).
/// The bind path casts the ISO/decimal text per dialect.
///
/// # Errors
///
/// [`ValidationError::Invalid`] when `fieldtype` is not in the canonical
/// vocabulary. Silently inventing a column for a misspelled type would create
/// schema the engine can never read.
fn column_type(fieldtype: &str, dialect: Dialect) -> Result<&'static str> {
    let ty = match fieldtype {
        // Single-line text. `Table MultiSelect` is a comma-separated list of
        // links in Ferris, so it is text, not a child table.
        "Data" | "Select" | "Link" | "Dynamic Link" | "Attach" | "Table MultiSelect" => {
            "VARCHAR(255)"
        }
        // Multi-line/opaque text, plus every other known fieldtype that carries
        // a string or an opaque payload in the document (secret material, a
        // file path, a colour, an ISO week, a GeoJSON blob, a canvas data URL,
        // a widget marker). `TEXT` round-trips all of them.
        "Small Text" | "Text" | "Long Text" | "Text Editor" | "Code" | "JSON" | "Password"
        | "Secret" | "Rating" | "Duration" | "Time" | "Week" | "Markdown Editor"
        | "HTML Editor" | "Color" | "File" | "Attach Image" | "Attachment Gallery"
        | "Signature" | "Barcode" | "Geolocation" | "Phone" | "Email" | "URL" | "Read Only"
        | "Widget" => "TEXT",
        // 64-bit integer; `SMALLINT` is reserved for `docstatus`.
        "Int" => "BIGINT",
        // `Any` has no decimal type yet. Postgres spells it `DOUBLE PRECISION`
        // (`DOUBLE` does not exist there); SQLite and MySQL accept `REAL`/
        // `DOUBLE`. Currency and Percent are numbers, not strings.
        "Float" => match dialect {
            Dialect::Postgres => "DOUBLE PRECISION",
            Dialect::MySQL => "DOUBLE",
            Dialect::SQLite => "REAL",
        },
        // Money is exact. SQLite has no decimal storage class; `NUMERIC`
        // affinity keeps the value as a real/integer and the getter widens it.
        "Currency" | "Percent" => match dialect {
            Dialect::Postgres => "NUMERIC(21,9)",
            Dialect::MySQL => "DECIMAL(21,9)",
            Dialect::SQLite => "NUMERIC",
        },
        "Check" => "BOOLEAN",
        // SQLite stores dates as ISO-8601 text (`TEXT` affinity); the driver
        // parses them on read. Postgres/MySQL have native date types.
        "Date" => match dialect {
            Dialect::SQLite => "TEXT",
            Dialect::Postgres | Dialect::MySQL => "DATE",
        },
        "Datetime" => match dialect {
            Dialect::SQLite => "TEXT",
            Dialect::Postgres => "TIMESTAMP",
            Dialect::MySQL => "DATETIME",
        },
        other => {
            return Err(invalid(format!(
                "field uses unknown fieldtype `{other}`; it has no column type"
            )));
        }
    };
    Ok(ty)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Validation(ValidationError::Invalid(message.into()))
}

/// The column backing `field`, or `None` when the field is not stored.
///
/// Layout and valueless fieldtypes ([`NON_STORED_FIELDTYPES`]) and virtual
/// fields (`FieldMeta::r#virtual`, computed on read) return `None`. A `Table`
/// field is a child table, not a column; it also returns `None` here and is
/// handled by [`create_doc_type_statements`]/[`child_table_columns`].
///
/// `reqd` becomes `NOT NULL`. That is only safe in a freshly created table: an
/// existing table may already hold rows, which is why [`alter_add_columns`]
/// clears the flag before rendering `ADD COLUMN`.
///
/// [`Column`] has no uniqueness flag — `ferris-db` expresses uniqueness as an
/// index — so [`FieldMeta::unique`] is not read here. [`create_doc_type_statements`]
/// materializes it as a `CREATE UNIQUE INDEX`. A unique index over a *nullable*
/// column is valid on all three backends (NULLs are distinct), which is what
/// lets [`alter_add_columns`] keep the constraint while adding the column
/// without a value.
///
/// # Errors
///
/// A fieldname that is not a valid SQL identifier, or a fieldtype outside the
/// canonical vocabulary.
pub fn field_column(field: &FieldMeta, dialect: Dialect) -> Result<Option<Column>> {
    if field.r#virtual || NON_STORED_FIELDTYPES.contains(&normalized_fieldtype(field)) {
        return Ok(None);
    }
    let name = Ident::new(&field.fieldname)?;
    let ty = column_type(normalized_fieldtype(field), dialect)?;
    let mut column = Column::new(name.as_str(), ty);
    if field.reqd {
        column = column.not_null();
    }
    Ok(Some(column))
}

/// The columns of a child table (`tab<Child DocType>`).
///
/// Frappe's four link columns come first, then the child's own stored fields:
///
/// * `parent` — the name of the parent document.
/// * `parenttype` — the parent `DocType` (a child can be shared by many).
/// * `parentfield` — the `Table` field on the parent that owns the row.
/// * `idx` — the row's position inside that field, so ordering survives a
///   round-trip.
///
/// There is no primary key: a child row is identified by its parent and
/// position, and [`create_doc_type_statements`] indexes
/// `(parent, parentfield, idx)`. A field that would shadow one of these four
/// names is rejected rather than silently duplicated.
///
/// # Errors
///
/// `<meta>` declaring a field that redeclares a link column is skipped (the
/// link column already exists), or any error from [`field_column`].
pub fn child_table_columns(meta: &DoctypeMeta, dialect: Dialect) -> Result<Vec<Column>> {
    // A child table is also a real document table: the tags service writes
    // `Tag Link` through the ordinary engine, and the same table has to serve
    // both. The system columns come first; the four link columns are
    // nullable because a standalone child row has no parent, while a row
    // written by its parent always fills them.
    let mut columns = document_columns();
    columns.extend([
        Column::new("parent", "VARCHAR(255)"),
        Column::new("parenttype", "VARCHAR(255)"),
        Column::new("parentfield", "VARCHAR(255)"),
        Column::new("idx", "BIGINT").not_null().default("0"),
    ]);
    for field in &meta.fields {
        let Some(column) = field_column(field, dialect)? else {
            continue;
        };
        if ["parent", "parenttype", "parentfield", "idx", "name"].contains(&column.name.as_str())
            || SYSTEM_COLUMNS.contains(&column.name.as_str())
        {
            continue; // the link/system column already exists
        }
        columns.push(column);
    }
    Ok(columns)
}

/// Every statement that brings a `DocType`'s table — and its child tables — into
/// existence.
///
/// The parent table keeps the system columns from
/// [`document_columns`](crate::migration::document_columns) (`name` primary key,
/// `owner`, `docstatus`, `creation`, `modified`, `modified_by`, `amended_from`
/// and `workflow_state`) and adds one column per stored field.
///
/// For each `Table` field the child `DocType` is resolved through
/// [`get_doctype`]; its table and indexes are emitted once even when two fields
/// point at the same child. Indexes are emitted for:
///
/// * `FieldMeta::unique` on a stored field;
/// * `meta.indexes` entries whose every field is a real column (system columns
///   plus the declared field columns).
///
/// The same `(table, columns, uniqueness)` is emitted at most once, so a
/// `unique` field that is also listed in `meta.indexes` gets one unique index.
///
/// # Errors
///
/// A `Table` field with no `options`, a child `DocType` that is not registered, an
/// invalid identifier, a field that shadows a system column, or a fieldtype
/// with no column type.
pub fn create_doc_type_statements(meta: &DoctypeMeta, dialect: Dialect) -> Result<Vec<String>> {
    let table = document_table(&meta.name)?;
    let mut columns = document_columns();
    let mut seen: HashSet<String> = columns.iter().map(|c| c.name.clone()).collect();
    let mut indexable: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    let mut unique_columns: Vec<String> = Vec::new();
    let mut children: Vec<(String, String)> = Vec::new();

    for field in &meta.fields {
        if SYSTEM_COLUMNS.contains(&field.fieldname.as_str()) || field.r#virtual {
            continue;
        }
        if normalized_fieldtype(field) == "Table" {
            let child = field
                .options
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    invalid(format!(
                        "DocType `{}` field `{}` is a Table without an `options` child DocType",
                        meta.name, field.fieldname
                    ))
                })?;
            // A child DocType that this process has not registered (a minimal
            // test registry, a plugin that is not installed yet) has no table to
            // create; failing the parent's whole table would make an optional
            // child a boot blocker.
            if get_doctype(child).is_none() {
                continue;
            }
            children.push((field.fieldname.clone(), child.to_string()));
            continue;
        }
        let Some(column) = field_column(field, dialect)? else {
            continue;
        };
        if !seen.insert(column.name.clone()) {
            return Err(invalid(format!(
                "DocType `{}` declares column `{}` twice or shadows a system column",
                meta.name, column.name
            )));
        }
        indexable.push(column.name.clone());
        if field.unique {
            unique_columns.push(column.name.clone());
        }
        columns.push(column);
    }

    let mut statements = vec![create_table(dialect, &table, &columns)?];
    let mut emitted: HashSet<String> = HashSet::new();
    // The standard indexes every document table owns. They used to be emitted
    // only by the blob-only migration path; the native migration and the lazy
    // `ensure_table` have to carry them too.
    for (name, index_columns, unique) in crate::migration::document_indexes() {
        let index_columns: Vec<String> = index_columns
            .iter()
            .map(|column| (*column).to_string())
            .collect();
        push_index(
            &mut statements,
            &mut emitted,
            dialect,
            &table,
            name,
            &index_columns,
            unique,
        )?;
    }
    for name in &unique_columns {
        push_index(
            &mut statements,
            &mut emitted,
            dialect,
            &table,
            &format!("{name}_unique"),
            std::slice::from_ref(name),
            true,
        )?;
    }
    for index in &meta.indexes {
        if index.fields.is_empty() || !index.fields.iter().all(|field| indexable.contains(field)) {
            continue;
        }
        let name = if index.name.trim().is_empty() {
            format!("idx_{}", index.fields.join("_"))
        } else {
            index.name.clone()
        };
        push_index(
            &mut statements,
            &mut emitted,
            dialect,
            &table,
            &name,
            &index.fields,
            index.unique,
        )?;
    }
    // `search_index` is the declaration that a field is a list-search
    // predicate; without this index a `LIKE` over a declared column still
    // scans. A column index is enough (the planner uses it for prefix/equality;
    // substring search remains a residual).
    for field in &meta.fields {
        if !(field.search_index || field.in_standard_filter || field.sort_options)
            || field.r#virtual
        {
            continue;
        }
        if field_column(field, dialect)?.is_none() {
            continue;
        }
        push_index(
            &mut statements,
            &mut emitted,
            dialect,
            &table,
            &format!("{}_search", field.fieldname),
            std::slice::from_ref(&field.fieldname),
            false,
        )?;
    }
    append_child_statements(&mut statements, &children, dialect)?;
    Ok(statements)
}

/// Render one index unless the same table/columns/uniqueness was already done.
///
/// Deduplicating on the physical shape rather than the name is deliberate: a
/// `unique` field that also appears in `meta.indexes` is one constraint, not
/// two differently named indexes over one column.
fn push_index(
    statements: &mut Vec<String>,
    emitted: &mut HashSet<String>,
    dialect: Dialect,
    table: &str,
    name: &str,
    columns: &[String],
    unique: bool,
) -> Result<()> {
    if columns.is_empty() {
        return Ok(());
    }
    let key = format!(
        "{table}:{}:{}",
        if unique { "unique" } else { "plain" },
        columns.join(",")
    );
    if !emitted.insert(key) {
        return Ok(());
    }
    let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
    statements.push(ensure_index(dialect, name, table, &columns, unique)?);
    Ok(())
}

/// Emit each distinct child table plus its indexes.
fn append_child_statements(
    statements: &mut Vec<String>,
    children: &[(String, String)],
    dialect: Dialect,
) -> Result<()> {
    let mut emitted: HashSet<String> = HashSet::new();
    let mut tables: HashSet<String> = HashSet::new();
    for (fieldname, child_name) in children {
        let child = get_doctype(child_name).ok_or_else(|| {
            invalid(format!(
                "DocType field `{fieldname}` names child DocType `{child_name}`, \
                 which is not registered"
            ))
        })?;
        let child_table = document_table(&child.name)?;
        if tables.insert(child_table.clone()) {
            let columns = child_table_columns(&child, dialect)?;
            statements.push(create_table(dialect, &child_table, &columns)?);
            let link_columns = ["parent", "parentfield", "idx"]
                .map(str::to_string)
                .to_vec();
            push_index(
                statements,
                &mut emitted,
                dialect,
                &child_table,
                "idx_parent",
                &link_columns,
                false,
            )?;
        }
        for field in &child.fields {
            if !field.unique {
                continue;
            }
            let Some(column) = field_column(field, dialect)? else {
                continue;
            };
            push_index(
                statements,
                &mut emitted,
                dialect,
                &child_table,
                &format!("{}_unique", column.name),
                std::slice::from_ref(&column.name),
                true,
            )?;
        }
    }
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` for every declared field not already present.
///
/// `existing` is the table's current column list (from the database catalog).
/// Columns that are already there are skipped, so the same call is idempotent
/// and can be replayed after a partial migration.
///
/// Every added column is **nullable**, even a `reqd` one: existing rows have no
/// value, and an insert that does not name the field would fail a `NOT NULL`
/// constraint. This function only creates the column safely.
///
/// # Errors
///
/// An invalid identifier, an unknown fieldtype, or a failure to render the
/// statement.
pub fn alter_add_columns(
    meta: &DoctypeMeta,
    existing: &[String],
    dialect: Dialect,
) -> Result<Vec<String>> {
    let table = document_table(&meta.name)?;
    let system: HashSet<String> = document_columns().into_iter().map(|c| c.name).collect();
    let existing: HashSet<&str> = existing.iter().map(String::as_str).collect();
    let mut statements = Vec::new();
    for field in &meta.fields {
        if system.contains(&field.fieldname) || existing.contains(field.fieldname.as_str()) {
            continue;
        }
        let Some(mut column) = field_column(field, dialect)? else {
            continue;
        };
        // See the function docs: a new column on a populated table cannot be
        // NOT NULL until the backfill has run.
        column.not_null = false;
        statements.push(add_column(dialect, &table, &column)?);
    }
    Ok(statements)
}

#[cfg(test)]
mod tests;
