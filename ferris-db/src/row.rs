//! Dialect-tolerant row decoding.
//!
//! The same logical column decodes differently per backend: SQLite has no
//! boolean type so a `BOOLEAN` column arrives as `INTEGER`, `COUNT(*)` is an
//! `i64` on SQLite/Postgres but a `u64` on MySQL, a `SMALLINT` is an `i16` on
//! the native drivers, and a JSON blob may come back as `TEXT` or as raw bytes.
//! Every repository re-implemented those fallbacks by hand — several with the
//! fallback, several without, which is why `sharing.rs` failed on SQLite while
//! `user_store.rs` worked.
//!
//! [`Row`] resolves them once, over the concrete [`DbRow`] of the backend that
//! answered the query.

use ferris_core::{Error, Result, ValidationError, Value};
use sqlx::{
    Column, Decode, MySql, Postgres, Row as SqlxRow, Sqlite, Type, ValueRef, mysql::MySqlRow,
    postgres::PgRow, sqlite::SqliteRow,
};
use std::collections::HashMap;

/// A result row from one concrete driver.
///
/// The variant is the backend that produced it; keeping it concrete is what
/// lets a native `BOOLEAN`, `SMALLINT` or later `DATE`/`DECIMAL` decode instead
/// of being erased by `sqlx::Any`.
pub enum DbRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
    MySql(MySqlRow),
}

impl DbRow {
    /// Number of columns in the row.
    fn len(&self) -> usize {
        match self {
            Self::Sqlite(row) => row.len(),
            Self::Postgres(row) => row.len(),
            Self::MySql(row) => row.len(),
        }
    }

    /// Whether the row has no columns.
    fn is_empty(&self) -> bool {
        match self {
            Self::Sqlite(row) => row.is_empty(),
            Self::Postgres(row) => row.is_empty(),
            Self::MySql(row) => row.is_empty(),
        }
    }

    /// Column names, in order.
    pub(crate) fn columns(&self) -> Vec<&str> {
        match self {
            Self::Sqlite(row) => row.columns().iter().map(Column::name).collect(),
            Self::Postgres(row) => row.columns().iter().map(Column::name).collect(),
            Self::MySql(row) => row.columns().iter().map(Column::name).collect(),
        }
    }

    /// Whether the row has a column, matched case-insensitively.
    fn has(&self, name: &str) -> bool {
        self.columns()
            .iter()
            .any(|column| column.eq_ignore_ascii_case(name))
    }

    /// Whether the column's value is SQL `NULL`.
    ///
    /// Decoding has to know this before it runs: SQLite's driver answers a
    /// `NULL` blob as an empty slice and a `NULL` integer as zero, so a decoder
    /// alone cannot tell `NULL` from an empty string or a real zero.
    fn is_null(&self, name: &str) -> std::result::Result<bool, sqlx::Error> {
        match self {
            Self::Sqlite(row) => SqlxRow::try_get_raw(row, name).map(|value| value.is_null()),
            Self::Postgres(row) => SqlxRow::try_get_raw(row, name).map(|value| value.is_null()),
            Self::MySql(row) => SqlxRow::try_get_raw(row, name).map(|value| value.is_null()),
        }
    }

    /// Decode a column by name.
    ///
    /// The bounds name all three drivers, so the same helper is usable from
    /// [`Row`], whose backend is only known at runtime.
    fn try_get<T>(&self, name: &str) -> std::result::Result<T, sqlx::Error>
    where
        for<'r> T: Decode<'r, Sqlite>
            + Type<Sqlite>
            + Decode<'r, Postgres>
            + Type<Postgres>
            + Decode<'r, MySql>
            + Type<MySql>,
    {
        match self {
            Self::Sqlite(row) => SqlxRow::try_get(row, name),
            Self::Postgres(row) => SqlxRow::try_get(row, name),
            Self::MySql(row) => SqlxRow::try_get(row, name),
        }
    }

    /// A `NUMERIC`/`DECIMAL` column, only on the backends that have the type.
    ///
    /// SQLite has no decimal storage class; its `NUMERIC` affinity yields a
    /// real/integer, which [`Row::opt_f64`] handles. Asking the SQLite row for
    /// a `BigDecimal` would not even compile, hence the per-variant method.
    fn try_get_decimal(&self, name: &str) -> Option<sqlx::types::BigDecimal> {
        match self {
            Self::Sqlite(_) => None,
            Self::Postgres(row) => {
                if SqlxRow::try_get_raw(row, name).ok()?.is_null() {
                    return None;
                }
                SqlxRow::try_get::<sqlx::types::BigDecimal, _>(row, name).ok()
            }
            Self::MySql(row) => {
                if SqlxRow::try_get_raw(row, name).ok()?.is_null() {
                    return None;
                }
                SqlxRow::try_get::<sqlx::types::BigDecimal, _>(row, name).ok()
            }
        }
    }

    /// A `BIGINT UNSIGNED` column, which MySQL hands back as `u64`.
    ///
    /// `COUNT(*)` is unsigned on MySQL, and the concrete driver refuses to
    /// decode it as `i64`; the other two backends have no unsigned integer
    /// type, so this only ever answers on MySQL.
    fn try_get_unsigned(&self, name: &str) -> std::result::Result<Option<i64>, sqlx::Error> {
        let Self::MySql(row) = self else {
            return Ok(None);
        };
        if SqlxRow::try_get_raw(row, name)?.is_null() {
            return Ok(None);
        }
        match SqlxRow::try_get::<u64, _>(row, name) {
            Ok(value) => Ok(Some(i64::try_from(value).unwrap_or(i64::MAX))),
            Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// A result row with decoding that tolerates backend differences.
pub struct Row {
    inner: DbRow,
}

impl Row {
    pub fn new(inner: DbRow) -> Self {
        Self { inner }
    }

    /// Number of columns in the row.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Column names, in order.
    pub fn columns(&self) -> Vec<&str> {
        self.inner.columns()
    }

    /// Whether the row has a column, matched case-insensitively.
    ///
    /// MySQL returns column names with the case they were declared in; the
    /// drivers do not agree on folding it, so callers that index by a literal
    /// name need this.
    pub fn has(&self, name: &str) -> bool {
        self.inner.has(name)
    }

    /// A string column, or `None` when the column is absent or `NULL`.
    ///
    /// MySQL maps `TEXT` to a blob-shaped value, so a value that does not
    /// decode as `String` is retried as bytes and validated as UTF-8. The
    /// alternative was a `TEXT` column that inserted correctly and could never
    /// be read on MariaDB.
    pub fn opt_str(&self, name: &str) -> Option<String> {
        if let Some(value) = self.try_get::<String>(name).ok().flatten() {
            return Some(value);
        }
        self.try_get::<Vec<u8>>(name)
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    /// A string column. Errors when absent or `NULL`.
    pub fn str(&self, name: &str) -> Result<String> {
        self.opt_str(name).ok_or_else(|| missing(name, "string"))
    }

    /// An integer column, accepting any width the backend chose.
    ///
    /// MySQL `SMALLINT` arrives as `i16` and `COUNT(*)` as `u64` on the
    /// concrete driver, which is why the narrower widths and the unsigned
    /// fallback are not dead code.
    pub fn opt_i64(&self, name: &str) -> Option<i64> {
        if let Some(value) = self.try_get::<i64>(name).ok().flatten() {
            return Some(value);
        }
        if let Some(value) = self.try_get::<i32>(name).ok().flatten() {
            return Some(i64::from(value));
        }
        if let Some(value) = self.try_get::<i16>(name).ok().flatten() {
            return Some(i64::from(value));
        }
        if let Some(value) = self.inner.try_get_unsigned(name).ok().flatten() {
            return Some(value);
        }
        self.try_get::<String>(name)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
    }

    /// An integer column. Errors when absent or `NULL`.
    pub fn i64(&self, name: &str) -> Result<i64> {
        self.opt_i64(name).ok_or_else(|| missing(name, "integer"))
    }

    /// A floating-point column, falling back to integer and to text.
    ///
    /// A `REAL` declared column can be served as text by some driver, which is
    /// why the text parse is a fallback rather than dead code. The integer
    /// fallback widens `i64` to `f64`, which is lossy past 2^53 — a monetary
    /// column stored as an integer would want `bigdecimal` instead.
    #[allow(clippy::cast_precision_loss)]
    pub fn opt_f64(&self, name: &str) -> Option<f64> {
        if let Some(v) = self.try_get::<f64>(name).ok().flatten() {
            return Some(v);
        }
        if let Some(v) = self.opt_i64(name) {
            return Some(v as f64);
        }
        self.opt_str(name).and_then(|s| s.parse().ok())
    }

    /// A `DATE` column as ISO-8601 (`YYYY-MM-DD`).
    ///
    /// Postgres/MySQL decode to `NaiveDate`; SQLite stores the date as text, so
    /// the string fallback is the same value. A `NULL` is `None`.
    pub fn opt_date(&self, name: &str) -> Option<String> {
        if let Some(date) = self
            .try_get::<sqlx::types::chrono::NaiveDate>(name)
            .ok()
            .flatten()
        {
            return Some(date.format("%Y-%m-%d").to_string());
        }
        self.opt_str(name)
    }

    /// A `TIMESTAMP`/`DATETIME` column as RFC 3339.
    ///
    /// The document model and the auth/datetime consumers (`api_key`,
    /// `locked_until`, session timestamps) parse RFC 3339, so a native column
    /// is rendered back to that shape even though the SQL cast takes the
    /// space-separated spelling. SQLite stores text and may hand back either
    /// form; both are normalized here.
    pub fn opt_datetime(&self, name: &str) -> Option<String> {
        use sqlx::types::chrono::{DateTime, NaiveDateTime};
        if let Some(value) = self.try_get::<NaiveDateTime>(name).ok().flatten() {
            return Some(value.and_utc().to_rfc3339());
        }
        let text = self.opt_str(name)?;
        if let Ok(value) = DateTime::parse_from_rfc3339(&text) {
            return Some(value.to_rfc3339());
        }
        for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
            if let Ok(value) = NaiveDateTime::parse_from_str(&text, format) {
                return Some(value.and_utc().to_rfc3339());
            }
        }
        Some(text)
    }

    /// A `NUMERIC`/`DECIMAL` column as `f64`.
    ///
    /// Postgres/MySQL decode to `BigDecimal`; SQLite's `NUMERIC` affinity hands
    /// back a real or an integer, so `opt_f64` is the fallback. The document
    /// model represents money as a JSON number; exact decimals would need a
    /// dedicated `Value` variant.
    pub fn opt_decimal_f64(&self, name: &str) -> Option<f64> {
        if let Some(value) = self.inner.try_get_decimal(name) {
            return value.to_string().parse().ok();
        }
        self.opt_f64(name)
    }

    /// A boolean column, accepting the shapes the three backends produce.
    ///
    /// Postgres and MySQL hand back a real `bool`; SQLite applies INTEGER
    /// affinity to a `BOOLEAN` declared column and hands back an integer.
    pub fn opt_bool(&self, name: &str) -> Option<bool> {
        if let Some(v) = self.try_get::<bool>(name).ok().flatten() {
            return Some(v);
        }
        if let Some(v) = self.opt_i64(name) {
            return Some(v != 0);
        }
        match self.opt_str(name)?.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "1" => Some(true),
            "false" | "f" | "no" | "n" | "0" => Some(false),
            _ => None,
        }
    }

    /// A boolean column. Errors when absent or `NULL`.
    pub fn bool(&self, name: &str) -> Result<bool> {
        self.opt_bool(name).ok_or_else(|| missing(name, "boolean"))
    }

    /// A row count, clamped at zero.
    ///
    /// `COUNT(*)` comes back as `i64` on SQLite and Postgres and as `u64` on
    /// MySQL, so it goes through the integer fallbacks.
    pub fn count(&self, name: &str) -> u64 {
        self.opt_i64(name).map_or(0, |v| v.max(0).cast_unsigned())
    }

    /// A JSON column decoded into a [`Value`].
    ///
    /// Accepts text holding JSON or raw JSON bytes. The document tables store
    /// their fields in a `TEXT` column, so those are the two shapes it takes.
    pub fn json(&self, name: &str) -> Result<Value> {
        let raw = match self.try_get::<String>(name).ok().flatten() {
            Some(text) => text,
            None => self
                .try_get::<Vec<u8>>(name)
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .ok_or_else(|| missing(name, "json"))?,
        };
        let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
            Error::Validation(ValidationError::Invalid(format!(
                "Column {name} is not JSON: {e}"
            )))
        })?;
        Ok(Value::from_json(parsed))
    }

    /// A JSON object column decoded into its fields.
    ///
    /// The document tables store their fields in a single `data` JSON blob, so
    /// this is what every field-level read goes through. A blob that is not an
    /// object is a schema error rather than an empty field map.
    pub fn json_object(&self, name: &str) -> Result<HashMap<String, Value>> {
        self.json(name)?.as_object().cloned().ok_or_else(|| {
            Error::Validation(ValidationError::Invalid(format!(
                "Column {name} is not a JSON object"
            )))
        })
    }

    /// An optional JSON column.
    pub fn opt_json(&self, name: &str) -> Result<Option<Value>> {
        if self.opt_str(name).is_none() {
            return Ok(None);
        }
        self.json(name).map(Some)
    }

    /// Take the concrete row, for the crate's own test helper.
    #[cfg(test)]
    pub(crate) fn into_inner(self) -> DbRow {
        self.inner
    }

    fn try_get<T>(&self, name: &str) -> Result<Option<T>>
    where
        for<'r> T: Decode<'r, Sqlite>
            + Type<Sqlite>
            + Decode<'r, Postgres>
            + Type<Postgres>
            + Decode<'r, MySql>
            + Type<MySql>,
    {
        // A `NULL` and an absent column are both answered as `None`; decoding
        // is skipped because a backend may spell `NULL` as its zero value.
        match self.inner.is_null(name) {
            Ok(true) => return Ok(None),
            Ok(false) => {}
            // An absent column is as unusable as a NULL one for these readers.
            Err(sqlx::Error::ColumnNotFound(_)) => return Ok(None),
            Err(e) => return Err(crate::error::map(e)),
        }
        self.inner
            .try_get(name)
            .map(Some)
            .map_err(crate::error::map)
    }
}

fn missing(name: &str, expected: &str) -> Error {
    Error::Validation(ValidationError::Invalid(format!(
        "Expected {expected} in column {name}"
    )))
}

/// Turn a driver `fetch_optional` result into `Option`, mapping the error.
///
/// `fetch_optional` already answers `Ok(None)` for an empty result, so this
/// only has to normalise the error type; it exists so the repositories stop
/// repeating the `match` on [`crate::error::is_row_not_found`].
pub fn optional<T>(result: std::result::Result<Option<T>, sqlx::Error>) -> Result<Option<T>> {
    result.map_err(crate::error::map)
}

#[cfg(test)]
mod tests;
