//! Declared fields that own a physical column.
//!
//! The schema half lives in [`ferris_schema::doctype_columns`]. This module is
//! the engine half: it converts a document value into the value a column takes
//! and persists child rows. Reading is the inverse and lives on [`DocShape`].
//!
//! Columns are the only source of truth (ADR 0001). A document is rebuilt from
//! its system and declared columns, plus one real table per `Table` field; no
//! JSON blob is read or written.

use super::super::*;
use super::{DocShape, WriteSink};
use ferris_db::{Dialect, Stmt};
use ferris_schema::FieldMeta;

/// A field's value as the type its column takes.
///
/// `NULL` is preserved: a missing field must not be guessed into `0` or `""`.
pub(in crate::engine) fn column_value(field: &FieldMeta, document: &Value) -> Value {
    let raw = document
        .get(&field.fieldname)
        .cloned()
        .unwrap_or(Value::Null);
    match super::normalized_fieldtype(field) {
        "Int" => raw
            .as_i64()
            .or_else(|| {
                raw.as_str()
                    .and_then(|text| text.trim().parse::<i64>().ok())
            })
            .map_or(Value::Null, Value::from),
        "Float" | "Currency" | "Percent" => raw
            .as_f64()
            .or_else(|| {
                raw.as_str()
                    .and_then(|text| text.trim().parse::<f64>().ok())
            })
            .map_or(Value::Null, Value::from),
        "Check" => raw
            .as_bool()
            .or_else(|| raw.as_i64().map(|number| number != 0))
            .or_else(|| {
                raw.as_str().map(|text| {
                    matches!(
                        text.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "t" | "yes" | "y"
                    )
                })
            })
            .map_or(Value::Null, Value::from),
        // Text-backed columns: a JSON field is stored as its JSON text, and
        // every other value is expected to be a string.
        // `DATE`/`TIMESTAMP` are native columns; the document stores ISO text,
        // but MySQL's `DATETIME` cast rejects the `T`/timezone spelling that
        // RFC 3339 uses. Normalize once, here, for both engines.
        "Datetime" => raw
            .as_str()
            .and_then(normalize_datetime)
            .map_or(Value::Null, Value::from),
        "Date" => raw.as_str().map_or(Value::Null, Value::from),
        "JSON" => match raw {
            Value::Null => Value::Null,
            Value::String(text) => Value::from(text),
            other => Value::from(other.to_json().to_string()),
        },
        _ => match raw {
            Value::Null => Value::Null,
            Value::String(text) => Value::from(text),
            other => Value::from(other.to_json().to_string()),
        },
    }
}

/// `, "col", "col2"` for a `SELECT`/`INSERT` column list.
pub(in crate::engine) fn column_list(fields: &[FieldMeta], dialect: Dialect) -> Result<String> {
    let mut out = String::new();
    for field in fields {
        out.push_str(", ");
        out.push_str(&q(dialect, &field.fieldname)?);
    }
    Ok(out)
}

/// The values that go with [`column_list`]'s trailing columns.
pub(in crate::engine) fn column_values(fields: &[FieldMeta], document: &Value) -> Vec<Value> {
    fields
        .iter()
        .map(|field| column_value(field, document))
        .collect()
}

/// The placeholder a field needs on an `INSERT`/`UPDATE`, with the cast its
/// native column type requires.
///
/// `DATE`/`TIMESTAMP`/`NUMERIC` take an ISO string or a decimal number; the
/// string is bound and the cast makes the backend parse it.
fn placeholder(field: &FieldMeta, dialect: Dialect) -> &'static str {
    match (super::normalized_fieldtype(field), dialect) {
        ("Date", Dialect::Postgres | Dialect::MySQL) => "CAST(? AS DATE)",
        ("Datetime", Dialect::Postgres) => "CAST(? AS TIMESTAMP)",
        ("Datetime", Dialect::MySQL) => "CAST(? AS DATETIME)",
        ("Currency" | "Percent", Dialect::Postgres) => "CAST(? AS NUMERIC(21,9))",
        ("Currency" | "Percent", Dialect::MySQL) => "CAST(? AS DECIMAL(21,9))",
        _ => "?",
    }
}

/// `YYYY-MM-DD HH:MM:SS` for the native `TIMESTAMP`/`DATETIME` cast.
///
/// Accepts RFC 3339 and the space-separated form the Desk and SQL already
/// produce. A value that does not parse is returned as-is so the database
/// rejects it loudly instead of storing a guessed time.
fn normalize_datetime(text: &str) -> Option<String> {
    if let Ok(value) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(value.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(value) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Some(value.format("%Y-%m-%d %H:%M:%S").to_string());
        }
    }
    Some(text.to_string())
}

/// `, CAST(? AS DATE), ?, ...` for the declared columns of an `INSERT`.
pub(in crate::engine) fn column_placeholders(fields: &[FieldMeta], dialect: Dialect) -> String {
    let mut out = String::new();
    for field in fields {
        out.push_str(", ");
        out.push_str(placeholder(field, dialect));
    }
    out
}

/// `, "col" = ?, "date" = CAST(? AS DATE), ...` for an `UPDATE`.
pub(in crate::engine) fn update_set(fields: &[FieldMeta], dialect: Dialect) -> Result<String> {
    let mut out = String::new();
    for field in fields {
        out.push_str(", ");
        out.push_str(&q(dialect, &field.fieldname)?);
        out.push_str(" = ");
        out.push_str(placeholder(field, dialect));
    }
    Ok(out)
}

/// The column a field would add to an existing table.
///
/// Always nullable: an `ALTER TABLE ADD COLUMN ... NOT NULL` without a default
/// fails on every backend as soon as the table has a row.
pub(in crate::engine) fn nullable_column(
    field: &FieldMeta,
    dialect: Dialect,
) -> Result<Option<ferris_db::sql::Column>> {
    let Some(mut column) = ferris_schema::doctype_columns::field_column(field, dialect)? else {
        return Ok(None);
    };
    column.not_null = false;
    column.default = None;
    Ok(Some(column))
}

/// Replace a parent's child rows with the array the document carries.
///
/// Only a field the document actually carries is touched: a partial update that
/// does not name a child table must not erase it. Every child row is written
/// with its system columns and link columns; there is no blob.
pub(in crate::engine) async fn write_children(
    sink: &mut WriteSink<'_>,
    shape: &DocShape,
    parent_doctype: &str,
    parent_name: &str,
    document: &Value,
) -> Result<()> {
    let dialect = sink.dialect();
    let Value::Object(map) = document else {
        return Ok(());
    };
    let modified_by = document.get("modified_by").cloned();
    for table in &shape.tables {
        let Some(rows) = map.get(&table.fieldname).and_then(Value::as_array) else {
            continue;
        };
        let child_table = table_name(&table.child)?;
        sink.execute(&Stmt::new(
            format!(
                "DELETE FROM {} WHERE parent = ? AND parentfield = ? AND parenttype = ?",
                q(dialect, &child_table)?
            ),
            vec![
                Value::from(parent_name),
                Value::from(table.fieldname.clone()),
                Value::from(parent_doctype),
            ],
        ))
        .await?;

        // One multi-row INSERT per chunk instead of one round-trip per child
        // row. Forms with hundreds of lines were paying N network round-trips
        // for a single save. The chunk keeps the bind count under SQLite's
        // classic 999-variable ceiling.
        const MAX_PARAMS: usize = 800;
        let mut columns: Vec<String> = Vec::with_capacity(12 + table.columns.len());
        for name in [
            "name",
            "owner",
            "docstatus",
            "creation",
            "modified",
            "modified_by",
            "amended_from",
            "workflow_state",
            "parent",
            "parenttype",
            "parentfield",
            "idx",
        ] {
            columns.push(q(dialect, name)?);
        }
        for child in &table.columns {
            columns.push(q(dialect, &child.fieldname)?);
        }
        let per_row = columns.len().max(1);
        let rows_per_chunk = (MAX_PARAMS / per_row).max(1);
        let row_placeholders = format!(
            "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?{}",
            column_placeholders(&table.columns, dialect)
        );
        let values_group = format!("({row_placeholders})");
        let quoted_table = q(dialect, &child_table)?;
        let columns_sql = columns.join(", ");

        for (chunk_index, chunk) in rows.chunks(rows_per_chunk).enumerate() {
            let mut params: Vec<Value> = Vec::with_capacity(chunk.len() * per_row);
            for (offset, row) in chunk.iter().enumerate() {
                let index = chunk_index * rows_per_chunk + offset;
                let idx = i64::try_from(index).unwrap_or(i64::MAX);
                let child_name = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now().timestamp();
                params.extend([
                    Value::from(child_name),
                    Value::Null,
                    Value::from(0_i64),
                    Value::from(now),
                    Value::from(now),
                    modified_by.clone().unwrap_or(Value::Null),
                    Value::Null,
                    Value::Null,
                    Value::from(parent_name),
                    Value::from(parent_doctype),
                    Value::from(table.fieldname.clone()),
                    Value::from(idx),
                ]);
                for child in &table.columns {
                    params.push(column_value(child, row));
                }
            }
            let placeholders = std::iter::repeat_n(values_group.as_str(), chunk.len())
                .collect::<Vec<_>>()
                .join(", ");
            sink.execute(&Stmt::new(
                format!("INSERT INTO {quoted_table} ({columns_sql}) VALUES {placeholders}"),
                params,
            ))
            .await?;
        }
    }
    Ok(())
}

/// Delete every child row a document owns, across all of its `Table` fields.
///
/// A child row has no life of its own once the parent is gone, and the link
/// columns are the only key: `parent` names the document and `parenttype` keeps
/// two DocTypes that reuse the same child table apart. One statement per child
/// table covers every `Table` field that shares it, so a DocType with two table
/// fields pointing at the same child still issues a single delete.
pub(in crate::engine) async fn delete_children_via(
    sink: &mut WriteSink<'_>,
    shape: &DocShape,
    parent_doctype: &str,
    parent_name: &str,
) -> Result<()> {
    if shape.tables.is_empty() {
        return Ok(());
    }
    let dialect = sink.dialect();
    let mut seen = std::collections::HashSet::new();
    for table in &shape.tables {
        if !seen.insert(table.child.as_str()) {
            continue;
        }
        let child_table = table_name(&table.child)?;
        sink.execute(&Stmt::new(
            format!(
                "DELETE FROM {} WHERE parent = ? AND parenttype = ?",
                q(dialect, &child_table)?
            ),
            vec![Value::from(parent_name), Value::from(parent_doctype)],
        ))
        .await?;
    }
    Ok(())
}

/// Hydrate a parent's `Table` fields from their real child tables.
///
/// A child table with no rows leaves the field absent; children only ever live
/// in their own table now.
pub(in crate::engine) async fn read_children(
    sink: &mut WriteSink<'_>,
    shape: &DocShape,
    parent_doctype: &str,
    parent_name: &str,
    document: &mut Value,
) -> Result<()> {
    let dialect = sink.dialect();
    let Value::Object(map) = document else {
        return Ok(());
    };
    for table in &shape.tables {
        let child_table = table_name(&table.child)?;
        let rows = sink
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT {} FROM {} \
                     WHERE parent = ? AND parentfield = ? AND parenttype = ? ORDER BY idx",
                    table.select_list(dialect)?,
                    q(dialect, &child_table)?
                ),
                vec![
                    Value::from(parent_name),
                    Value::from(table.fieldname.clone()),
                    Value::from(parent_doctype),
                ],
            ))
            .await?;
        if rows.is_empty() {
            continue;
        }
        let children = rows
            .iter()
            .map(|row| table.row_to_child(row))
            .collect::<Vec<_>>();
        map.insert(table.fieldname.clone(), Value::Array(children));
    }
    Ok(())
}

/// [`read_children`] for a whole page: one query per child table instead of one
/// per row.
pub(in crate::engine) async fn read_children_batch(
    sink: &mut WriteSink<'_>,
    shape: &DocShape,
    parent_doctype: &str,
    documents: &mut [Value],
) -> Result<()> {
    if documents.is_empty() || shape.tables.is_empty() {
        return Ok(());
    }
    let dialect = sink.dialect();
    let positions: std::collections::HashMap<String, usize> = documents
        .iter()
        .enumerate()
        .filter_map(|(index, document)| {
            document
                .get("name")
                .and_then(Value::as_str)
                .map(|name| (name.to_string(), index))
        })
        .collect();
    if positions.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = positions.keys().cloned().collect();
    // SQLite's classic variable limit is 999; a page of 1,000 parents would
    // otherwise produce a query that fails only on the smallest backend.
    const MAX_PARAMS: usize = 800;
    for table in &shape.tables {
        let child_table = table_name(&table.child)?;
        let mut grouped: std::collections::HashMap<String, Vec<Value>> =
            std::collections::HashMap::new();
        // The two link predicates are part of the key: `parent` names the
        // document, `parenttype` keeps two DocTypes that reuse the same child
        // table (and the same field name) from reading each other's rows.
        for chunk in names.chunks(MAX_PARAMS.saturating_sub(2).max(1)) {
            let holes = vec!["?"; chunk.len()].join(", ");
            let mut params: Vec<Value> = chunk
                .iter()
                .map(|name| Value::string(name.clone()))
                .collect();
            params.push(Value::from(table.fieldname.clone()));
            params.push(Value::from(parent_doctype));
            let rows = sink
                .fetch_all(&Stmt::new(
                    format!(
                        "SELECT {} FROM {} \
                         WHERE parent IN ({holes}) AND parentfield = ? AND parenttype = ? \
                         ORDER BY parent, idx",
                        table.select_list(dialect)?,
                        q(dialect, &child_table)?
                    ),
                    params,
                ))
                .await?;
            for row in &rows {
                let Some(parent) = row.opt_str("parent") else {
                    continue;
                };
                grouped
                    .entry(parent)
                    .or_default()
                    .push(table.row_to_child(row));
            }
        }
        for (parent, children) in grouped {
            if let Some(index) = positions.get(&parent)
                && let Value::Object(map) = &mut documents[*index]
            {
                map.insert(table.fieldname.clone(), Value::Array(children));
            }
        }
    }
    Ok(())
}
