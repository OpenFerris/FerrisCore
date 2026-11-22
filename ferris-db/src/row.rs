//! Dialect-tolerant row decoding.
//!
//! The same logical column decodes differently per backend: SQLite has no
//! boolean type so a `BOOLEAN` column arrives as `INTEGER`, `COUNT(*)` is an
//! `i64` on SQLite/Postgres but can arrive as `i32` on MySQL, and a JSON blob
//! may come back as `TEXT` or as a native JSON value. Every repository
//! re-implemented those fallbacks by hand — several with the fallback, several
//! without, which is why `sharing.rs` failed on SQLite while
//! `user_store.rs` worked.
//!
//! [`Row`] resolves them once.

use ferris_core::{Error, Result, ValidationError, Value};
use sqlx::{Any, Column, Row as SqlxRow, any::AnyRow};
use std::collections::HashMap;

/// A result row with decoding that tolerates backend differences.
pub struct Row {
    inner: AnyRow,
}

impl Row {
    pub fn new(inner: AnyRow) -> Self {
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
        self.inner.columns().iter().map(Column::name).collect()
    }

    /// Whether the row has a column, matched case-insensitively.
    ///
    /// MySQL returns column names with the case they were declared in; the
    /// drivers do not agree on folding it, so callers that index by a literal
    /// name need this.
    pub fn has(&self, name: &str) -> bool {
        self.inner
            .columns()
            .iter()
            .any(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// A string column, or `None` when the column is absent or `NULL`.
    pub fn opt_str(&self, name: &str) -> Option<String> {
        self.try_get(name).ok().flatten()
    }

    /// A string column. Errors when absent or `NULL`.
    pub fn str(&self, name: &str) -> Result<String> {
        self.opt_str(name).ok_or_else(|| missing(name, "string"))
    }

    /// An integer column, accepting any width the backend chose.
    pub fn opt_i64(&self, name: &str) -> Option<i64> {
        if let Some(v) = self.try_get::<i64>(name).ok().flatten() {
            return Some(v);
        }
        if let Some(v) = self.try_get::<i32>(name).ok().flatten() {
            return Some(i64::from(v));
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
    /// A `REAL` declared column is served as text by the `Any` driver on some
    /// backends, which is why the text parse is a fallback rather than dead
    /// code. The integer fallback widens `i64` to `f64`, which is lossy past
    /// 2^53 — a monetary column stored as an integer would want `bigdecimal`
    /// instead.
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
    /// `COUNT(*)` comes back as `i64` on SQLite and Postgres and can be `i32`
    /// on MySQL, so it goes through the integer fallback.
    pub fn count(&self, name: &str) -> u64 {
        self.opt_i64(name).map_or(0, |v| v.max(0).cast_unsigned())
    }

    /// A JSON column decoded into a [`Value`].
    ///
    /// Accepts text holding JSON or raw JSON bytes. There is no native-JSON
    /// fast path: the `Any` driver erases JSON to a string, so a JSON column
    /// always arrives in one of those two shapes.
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

    fn try_get<T>(&self, name: &str) -> Result<Option<T>>
    where
        for<'r> T: sqlx::Decode<'r, Any> + sqlx::Type<Any>,
    {
        match self.inner.try_get::<T, _>(name) {
            Ok(value) => Ok(Some(value)),
            // An absent column is as unusable as a NULL one for these readers.
            Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
            Err(e) => Err(crate::error::map(e)),
        }
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
