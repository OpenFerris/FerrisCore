//! Bulk data entry points: import, export and mass update.
//!
//! Frappe ships these as `Data Import`, `Data Export` and `Bulk Update`, and the
//! Desk already lists the first two as `DocTypes` (`doctypes::data`). A form
//! without a runner is a facade: this module is the runner, built on the same
//! [`DocStore`] the resource routes use, so validation, lifecycle hooks, naming
//! and permissions are the engine's and there is no second write policy.
//!
//! Two rules shape the code: one bad row never sinks the batch (the caller gets
//! the failed rows by index), and every request is bounded — unbounded input is
//! an amplification factor, not an import. The routes are not behind a feature
//! flag: this is the generic store's own surface, like `/api/resource`.
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::Db;
use serde::{Deserialize, Serialize};

use crate::router::{ApiState, error_response};
use crate::store::{DocStore, effective_meta, validate_doctype};
use crate::types::ListParams;

/// Rows accepted by one `POST /api/data/import/{doctype}`.
pub const MAX_IMPORT_ROWS: usize = 1_000;

/// Documents accepted by one `POST /api/data/bulk-update/{doctype}`.
pub const MAX_BULK_NAMES: usize = 1_000;

/// Hard ceiling for one `GET /api/data/export/{doctype}`.
pub const MAX_EXPORT_ROWS: u32 = 10_000;

/// Default export size when the caller does not pass `limit`.
const DEFAULT_EXPORT_LIMIT: u32 = 1_000;

/// `DocStore::list` clamps one page to this many rows, so the export pages at
/// that size instead of asking for a limit the store silently reduces.
const EXPORT_PAGE: u32 = 1_000;

/// Field types that describe layout rather than data.
const LAYOUT_FIELDTYPES: &[&str] = &["Tab Break", "Section Break", "Column Break"];

/// How an import treats a row that carries a `name`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportMode {
    Insert,
    Upsert,
}

impl ImportMode {
    fn parse(raw: Option<&str>) -> Result<Self> {
        match raw.unwrap_or("insert") {
            "insert" => Ok(Self::Insert),
            "upsert" => Ok(Self::Upsert),
            other => Err(ValidationError::InvalidValue {
                field: "mode".to_string(),
                message: format!("expected \"insert\" or \"upsert\", got {other:?}"),
            }
            .into()),
        }
    }
}

/// Body of `POST /api/data/import/{doctype}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRequest {
    /// One JSON object per row, as `POST /api/resource` would take it.
    #[serde(default)]
    pub rows: Vec<Value>,
    /// `"insert"` (default) or `"upsert"`.
    #[serde(default)]
    pub mode: Option<String>,
}

/// A row that did not make it, addressed by its zero-based index in `rows`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowError {
    pub row: usize,
    pub error: String,
}

/// Outcome of one import run; the batch always finishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub inserted: usize,
    pub updated: usize,
    pub errors: Vec<RowError>,
}

/// Run an import against `doctype` as the ambient caller.
///
/// Each row goes through [`DocStore::create`], or [`DocStore::update`] when the
/// mode is `upsert` and the row names a document. A failure is recorded against
/// the row's index and the next row still runs.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype, an unknown mode or a
/// batch over [`MAX_IMPORT_ROWS`]. Row failures are not errors here: they land
/// in [`ImportReport::errors`].
pub async fn run_import(pool: &Db, doctype: &str, req: &ImportRequest) -> Result<ImportReport> {
    validate_doctype(doctype)?;
    if req.rows.len() > MAX_IMPORT_ROWS {
        return Err(ValidationError::Invalid(format!(
            "too many rows: {} (maximum {MAX_IMPORT_ROWS} per request)",
            req.rows.len()
        ))
        .into());
    }
    let mode = ImportMode::parse(req.mode.as_deref())?;

    let mut report = ImportReport {
        inserted: 0,
        updated: 0,
        errors: Vec::new(),
    };
    for (index, row) in req.rows.iter().enumerate() {
        if !matches!(row, Value::Object(_)) {
            report.errors.push(RowError {
                row: index,
                error: "row must be a JSON object".to_string(),
            });
            continue;
        }
        let target = upsert_name(mode, row);
        let outcome = match target {
            Some(name) => DocStore::update(pool, doctype, name, row.clone()).await,
            None => DocStore::create(pool, doctype, row.clone()).await,
        };
        match outcome {
            Ok(_) if target.is_some() => report.updated += 1,
            Ok(_) => report.inserted += 1,
            Err(e) => report.errors.push(RowError {
                row: index,
                error: row_error_message(&e),
            }),
        }
    }
    Ok(report)
}

/// The name an `upsert` row updates; a row without one is an insert, since
/// inventing a name from other fields would be a naming policy of its own.
fn upsert_name(mode: ImportMode, row: &Value) -> Option<&str> {
    match mode {
        ImportMode::Insert => None,
        ImportMode::Upsert => row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty()),
    }
}

/// The message a caller may see for a failed row, split like `error_response`:
/// a 4xx-class failure describes the caller's input; anything else stays
/// internal instead of leaking schema details.
fn row_error_message(error: &Error) -> String {
    match error {
        Error::Validation(_) | Error::Authz(_) | Error::Document(_) | Error::Schema(_) => {
            error.to_string()
        }
        _ => "internal error".to_string(),
    }
}

/// Body of `POST /api/data/bulk-update/{doctype}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkUpdateRequest {
    /// Document names to update, in order.
    #[serde(default)]
    pub names: Vec<String>,
    /// The field values applied to every named document.
    pub values: Value,
}

/// A document that could not be updated, addressed by its index in `names`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkRowError {
    pub row: usize,
    pub name: String,
    pub error: String,
}

/// Outcome of one bulk update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkUpdateReport {
    pub updated: usize,
    pub errors: Vec<BulkRowError>,
}

/// Apply `values` to every name, authorizing each document on its own.
///
/// [`DocStore::update`] enforces `Permission::Write` per document, so a caller
/// who may edit one row cannot use this endpoint to edit another. The result is
/// per row for the same reason as the import.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype, a non-object `values`,
/// or more than [`MAX_BULK_NAMES`] names.
pub async fn run_bulk_update(
    pool: &Db,
    doctype: &str,
    req: &BulkUpdateRequest,
) -> Result<BulkUpdateReport> {
    validate_doctype(doctype)?;
    if req.names.len() > MAX_BULK_NAMES {
        return Err(ValidationError::Invalid(format!(
            "too many documents: {} (maximum {MAX_BULK_NAMES} per request)",
            req.names.len()
        ))
        .into());
    }
    if !matches!(req.values, Value::Object(_)) {
        return Err(ValidationError::InvalidValue {
            field: "values".to_string(),
            message: "must be a JSON object of field values".to_string(),
        }
        .into());
    }

    let mut report = BulkUpdateReport {
        updated: 0,
        errors: Vec::new(),
    };
    for (index, name) in req.names.iter().enumerate() {
        match DocStore::update(pool, doctype, name, req.values.clone()).await {
            Ok(_) => report.updated += 1,
            Err(e) => report.errors.push(BulkRowError {
                row: index,
                name: name.clone(),
                error: row_error_message(&e),
            }),
        }
    }
    Ok(report)
}

/// Query parameters of `GET /api/data/export/{doctype}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportQuery {
    /// `csv` (default) or `json`.
    #[serde(default)]
    pub format: Option<String>,
    /// Row cap; clamped to [`MAX_EXPORT_ROWS`].
    #[serde(default)]
    pub limit: Option<u32>,
}

/// The header row and the documents of one export.
#[derive(Debug, Clone, Serialize)]
pub struct ExportData {
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
}

/// The column set: the `DocType`'s declared data fields plus `name`, never
/// system columns (`owner`, `docstatus`, …) or layout breaks. A `DocType` with
/// no registered metadata exports its `name`: there is no declared shape.
async fn export_columns(pool: &Db, doctype: &str) -> Result<Vec<String>> {
    let mut columns = vec!["name".to_string()];
    if let Some(meta) = effective_meta(pool, doctype).await? {
        for field in &meta.fields {
            if LAYOUT_FIELDTYPES.contains(&field.fieldtype.as_str()) {
                continue;
            }
            if !columns.iter().any(|column| column == &field.fieldname) {
                columns.push(field.fieldname.clone());
            }
        }
    }
    Ok(columns)
}

/// Read up to `limit` documents authorized as the ambient caller.
///
/// [`DocStore::list`] applies `Permission::Read` row by row (plus user
/// permissions and shares), so an export never contains a document the caller
/// could not open. The store clamps one page to 1000 rows, hence the paging
/// loop; `name asc` keeps the pages stable.
///
/// # Errors
///
/// Returns a validation error for a malformed doctype and the store's own
/// metadata/storage error while resolving or listing the `DocType`.
pub async fn collect_export(pool: &Db, doctype: &str, limit: u32) -> Result<ExportData> {
    validate_doctype(doctype)?;
    let limit = limit.clamp(1, MAX_EXPORT_ROWS);
    let columns = export_columns(pool, doctype).await?;
    let projection = Some(columns.clone());

    let mut rows: Vec<Value> = Vec::new();
    let mut collected: u32 = 0;
    let mut offset: u32 = 0;
    while collected < limit {
        let take = (limit - collected).min(EXPORT_PAGE);
        let params = ListParams {
            fields: projection.clone(),
            limit: Some(take),
            offset: Some(offset),
            order_by: Some("name asc".to_string()),
            ..Default::default()
        };
        let page = DocStore::list(pool, doctype, &params).await?;
        // The store clamps one page to 1000 rows, so `returned` fits in `u32`.
        let returned = u32::try_from(page.data.len()).unwrap_or(u32::MAX);
        rows.extend(page.data);
        collected = collected.saturating_add(returned);
        if returned < take {
            break;
        }
        offset = offset.saturating_add(take);
    }
    Ok(ExportData { columns, rows })
}

/// Render an export as RFC 4180 CSV.
///
/// The header is escaped like any cell, and every text cell is guarded against
/// spreadsheet formula injection: a leading `=`, `+`, `-`, `@`, tab or CR would
/// otherwise be evaluated when the file is opened. Prefixing `'` keeps it text;
/// quoting happens afterwards because the apostrophe is part of the value.
#[must_use]
pub fn to_csv(columns: &[String], rows: &[Value]) -> String {
    let mut out = String::new();
    let header: Vec<String> = columns.iter().map(|column| csv_escape(column)).collect();
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    for row in rows {
        let line: Vec<String> = columns
            .iter()
            .map(|column| csv_cell(row.get(column)))
            .collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

fn csv_cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        // A string is the caller's text verbatim: the only cell that can carry
        // a formula.
        Some(Value::String(text)) => csv_escape(text),
        // Numbers, booleans and JSON bodies cannot start a formula (`-5` is a
        // number, an array/object starts with `[`/`{`), so they are only
        // quoted when their JSON spelling needs it.
        Some(other) => csv_quote(&other.to_string()),
    }
}

fn csv_escape(raw: &str) -> String {
    let formula = raw
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'));
    let mut text = String::with_capacity(raw.len() + 2);
    if formula {
        text.push('\'');
    }
    text.push_str(raw);
    csv_quote(&text)
}

/// RFC 4180: a cell with a delimiter, a quote or a line break is wrapped and
/// its own quotes doubled.
fn csv_quote(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn export_filename(doctype: &str, extension: &str) -> String {
    let stem: String = doctype
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}.{extension}")
}

fn attachment_response(content_type: &str, filename: &str, body: String) -> Response {
    let mut headers = HeaderMap::new();
    let content_type = HeaderValue::from_str(content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, content_type);
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    headers.insert(header::CONTENT_DISPOSITION, disposition);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (StatusCode::OK, headers, body).into_response()
}

/// `POST /api/data/import/{doctype}`.
pub async fn import(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Json(req): Json<ImportRequest>,
) -> Response {
    match run_import(&state.pool, &doctype, &req).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /api/data/bulk-update/{doctype}`.
pub async fn bulk_update(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Json(req): Json<BulkUpdateRequest>,
) -> Response {
    match run_bulk_update(&state.pool, &doctype, &req).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /api/data/export/{doctype}?format=csv|json&limit=…`.
pub async fn export(
    State(state): State<Arc<ApiState>>,
    Path(doctype): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let format = query.format.as_deref().unwrap_or("csv");
    let limit = query.limit.unwrap_or(DEFAULT_EXPORT_LIMIT);
    let data = match collect_export(&state.pool, &doctype, limit).await {
        Ok(data) => data,
        Err(e) => return error_response(e),
    };
    match format {
        "csv" => attachment_response(
            "text/csv; charset=utf-8",
            &export_filename(&doctype, "csv"),
            to_csv(&data.columns, &data.rows),
        ),
        "json" => attachment_response(
            "application/json",
            &export_filename(&doctype, "json"),
            // `Value` serialization only fails on a bug in `Value`; its message
            // is internal either way.
            serde_json::to_string(&data).unwrap_or_else(|_| "internal error".to_string()),
        ),
        other => error_response(
            ValidationError::InvalidValue {
                field: "format".to_string(),
                message: format!("expected \"csv\" or \"json\", got {other:?}"),
            }
            .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Db {
        ferris_db::memory_db().await.expect("in-memory sqlite")
    }

    fn admin() -> ferris_auth::User {
        let mut user =
            ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
        user.roles = vec!["System Manager".to_string()];
        user
    }

    fn guest() -> ferris_auth::User {
        let mut user =
            ferris_auth::User::system_user("alice@local".to_string(), "Alice".to_string());
        user.id = "alice@local".to_string();
        user
    }

    #[tokio::test]
    async fn import_inserts_the_good_rows_and_reports_the_bad_one() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            let req = ImportRequest {
                rows: vec![
                    ferris_core::value!({ "name": "IMP-1", "title": "one" }),
                    ferris_core::value!({ "name": "IMP-2", "title": "two" }),
                    Value::String("not a row".to_string()),
                ],
                mode: Some("insert".to_string()),
            };
            let report = run_import(&pool, "Probe", &req).await.expect("batch runs");
            assert_eq!((report.inserted, report.updated), (2, 0));
            assert_eq!(report.errors.len(), 1);
            assert_eq!(report.errors[0].row, 2);
            let first = DocStore::get(&pool, "Probe", "IMP-1").await.unwrap();
            assert_eq!(first.get("title").and_then(Value::as_str), Some("one"));
        })
        .await;
    }

    #[tokio::test]
    async fn import_rejects_an_oversized_batch() {
        let pool = pool().await;
        let rows = vec![ferris_core::value!({ "title": "x" }); MAX_IMPORT_ROWS + 1];
        let error = run_import(&pool, "Probe", &ImportRequest { rows, mode: None })
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Validation(_)));
    }

    #[tokio::test]
    async fn export_respects_the_row_limit() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            for index in 1..=3 {
                let doc = serde_json::json!({ "name": format!("EXP-{index}") });
                DocStore::create(&pool, "Probe", Value::from_json(doc))
                    .await
                    .unwrap();
            }
            let data = collect_export(&pool, "Probe", 2).await.unwrap();
            assert_eq!(data.rows.len(), 2);
            assert_eq!(
                data.rows[0].get("name").and_then(Value::as_str),
                Some("EXP-1")
            );
            let csv = to_csv(&data.columns, &data.rows);
            assert_eq!(csv.lines().count(), 3, "header plus two rows");
        })
        .await;
    }

    #[test]
    fn csv_escapes_quotes_commas_newlines_and_formulas() {
        let columns = vec!["name".to_string(), "title".to_string()];
        let rows = vec![ferris_core::value!({
            "name": "P,1",
            "title": "say \"hi\"\nnext",
        })];
        let csv = to_csv(&columns, &rows);
        assert!(csv.contains("\"P,1\""));
        assert!(csv.contains("\"say \"\"hi\"\"\nnext\""));

        for trigger in ["=1+1", "+cmd", "-cmd", "@cmd"] {
            let rows = vec![ferris_core::value!({ "name": trigger })];
            assert!(to_csv(&["name".to_string()], &rows).contains(&format!("'{trigger}")));
        }

        // A negative number is a number, not a formula: the guard is for text.
        let rows = vec![Value::from_json(serde_json::json!({ "name": -5 }))];
        assert!(to_csv(&["name".to_string()], &rows).contains("\r\n-5\r\n"));
    }

    #[tokio::test]
    async fn bulk_update_is_authorized_per_document() {
        let pool = pool().await;
        ferris_document::with_user(Some(admin()), async {
            for name in ["BULK-1", "BULK-2"] {
                DocStore::create(&pool, "Probe", ferris_core::value!({ "name": name }))
                    .await
                    .unwrap();
            }
            let allowed = BulkUpdateRequest {
                names: vec!["BULK-1".to_string()],
                values: ferris_core::value!({ "title": "updated" }),
            };
            let report = run_bulk_update(&pool, "Probe", &allowed).await.unwrap();
            assert_eq!(report.updated, 1);
            assert!(report.errors.is_empty());
        })
        .await;

        ferris_document::with_user(Some(guest()), async {
            let request = BulkUpdateRequest {
                names: vec!["BULK-1".to_string(), "BULK-2".to_string()],
                values: ferris_core::value!({ "title": "hijacked" }),
            };
            let report = run_bulk_update(&pool, "Probe", &request).await.unwrap();
            assert_eq!(report.updated, 0);
            assert_eq!(report.errors.len(), 2);
            assert_eq!(report.errors[0].name, "BULK-1");
        })
        .await;

        ferris_document::with_user(Some(admin()), async {
            let doc = DocStore::get(&pool, "Probe", "BULK-1").await.unwrap();
            assert_eq!(doc.get("title").and_then(Value::as_str), Some("updated"));
        })
        .await;
    }
}
