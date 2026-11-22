//! Dialect-aware SQL construction.
//!
//! The framework needs three things that `sqlx` does not give it:
//!
//! 1. **Validated identifiers.** A table name is derived at runtime from a
//!    `DocType` (`tabToDo`), so it cannot be a compile-time literal. [`Ident`]
//!    validates on construction and quotes on render, which removes the
//!    `format!("SELECT ... FROM {table}")` pattern the repositories used.
//! 2. **Portable upserts.** `ON CONFLICT (...) DO UPDATE` is Postgres/SQLite
//!    only; MySQL needs `ON DUPLICATE KEY UPDATE` with no conflict target.
//!    [`upsert`] emits the right spelling for the dialect.
//! 3. **Portable DDL.** Oracle MySQL rejects `CREATE INDEX IF NOT EXISTS`
//!    and a `DEFAULT` on a `TEXT` column. [`create_table`] and [`ensure_index`]
//!    account for that.
//!
//! Values are never interpolated: they are collected as bound parameters.
//!
//! The module is split by concern: identifier/upsert construction lives here,
//! and the portable DDL builders (`create_table`, `ensure_index`, the JSON
//! index helpers, `add_column`) live in [`ddl`], re-exported so every existing
//! `crate::sql::…` path keeps resolving.

mod ddl;

#[cfg(test)]
mod tests;

pub use ddl::{
    Column, add_column, create_table, ensure_index, ensure_json_index, json_field_column,
    json_field_target,
};

use crate::dialect::{Dialect, is_plain_ident, is_qualified_ident};
use ferris_core::{Error, Result, ValidationError, Value};

/// A validated SQL identifier or qualified name (`tabToDo`, `tabToDo.name`).
///
/// Construction validates the characters, so rendering cannot fail and cannot
/// inject. Use [`Ident::quote`] to interpolate it into SQL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident(String);

impl Ident {
    /// Validate a bare identifier (`[A-Za-z0-9_]+`).
    pub fn new(name: &str) -> Result<Self> {
        if !is_plain_ident(name) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "Invalid identifier: {name}"
            ))));
        }
        Ok(Self(name.to_string()))
    }

    /// Validate a possibly qualified name (`table.column`).
    pub fn qualified(name: &str) -> Result<Self> {
        if !is_qualified_ident(name) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "Invalid identifier: {name}"
            ))));
        }
        Ok(Self(name.to_string()))
    }

    /// Build a `prefix.name` identifier from two validated parts.
    pub fn pair(prefix: &str, name: &str) -> Result<Self> {
        Self::new(prefix).and_then(|p| Self::new(name).map(|n| Self(format!("{p}.{n}"))))
    }

    /// The unquoted name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The last segment, i.e. the column name of `table.column`.
    pub fn column(&self) -> &str {
        self.0.rsplit('.').next().unwrap_or(&self.0)
    }

    /// Render quoted for `dialect`.
    ///
    /// Infallible: the name was validated at construction.
    pub fn quote(&self, dialect: Dialect) -> String {
        self.0
            .split('.')
            .map(|part| {
                dialect
                    .try_quote_ident(part)
                    .unwrap_or_else(|| part.to_string())
            })
            .collect::<Vec<_>>()
            .join(".")
    }
}

impl std::fmt::Display for Ident {
    /// Unquoted rendering. Only for diagnostics — interpolate
    /// [`Ident::quote`] into SQL.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One assignment in an upsert's `DO UPDATE` clause.
#[derive(Debug, Clone)]
pub enum UpsertSet {
    /// `col = ?`, bound to the value.
    Value(Value),
    /// `col = col + ?`, used for counters such as `version = version + 1`.
    Increment(Value),
    /// `col = excluded.col`, i.e. take the value from the incoming row.
    FromExcluded(String),
}

impl UpsertSet {
    fn render(&self, column: &Ident, dialect: Dialect, params: &mut Vec<Value>) -> Result<String> {
        Ok(match self {
            Self::Value(v) => {
                params.push(v.clone());
                format!("{} = ?", column.quote(dialect))
            }
            Self::Increment(v) => {
                params.push(v.clone());
                let col = column.quote(dialect);
                format!("{col} = {col} + ?")
            }
            Self::FromExcluded(source) => {
                let source = Ident::new(source)?;
                format!("{} = excluded.{}", column.quote(dialect), source.as_str())
            }
        })
    }
}

/// An `INSERT` that resolves a uniqueness conflict portably.
///
/// Build it with [`UpsertBuilder`] so the column count and value count cannot
/// drift apart.
#[derive(Debug)]
pub struct Upsert {
    sql: String,
    params: Vec<Value>,
}

impl Upsert {
    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn params(&self) -> &[Value] {
        &self.params
    }

    pub fn into_parts(self) -> (String, Vec<Value>) {
        (self.sql, self.params)
    }
}

/// Incremental builder for a portable upsert.
///
/// Names are taken as `&str` and validated once in [`UpsertBuilder::build`],
/// which is the only place that renders SQL — so a caller spells its columns
/// literally instead of threading an [`Ident`] through every call.
pub struct UpsertBuilder {
    dialect: Dialect,
    table: String,
    columns: Vec<String>,
    values: Vec<Value>,
    conflict: Vec<String>,
    updates: Vec<(String, UpsertSet)>,
    do_nothing: bool,
}

impl UpsertBuilder {
    pub fn new(dialect: Dialect, table: impl Into<String>) -> Self {
        Self {
            dialect,
            table: table.into(),
            columns: Vec::new(),
            values: Vec::new(),
            conflict: Vec::new(),
            updates: Vec::new(),
            do_nothing: false,
        }
    }

    /// The columns an incoming row writes, paired with their values.
    pub fn values(
        mut self,
        columns_values: impl IntoIterator<Item = (impl Into<String>, Value)>,
    ) -> Self {
        for (column, value) in columns_values {
            self.columns.push(column.into());
            self.values.push(value);
        }
        self
    }

    /// The uniqueness key the upsert conflicts on.
    pub fn conflict_on(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.conflict = columns.into_iter().map(Into::into).collect();
        self
    }

    /// An assignment applied when the conflict fires.
    pub fn on_update(mut self, column: impl Into<String>, set: UpsertSet) -> Self {
        self.updates.push((column.into(), set));
        self
    }

    /// Insert the row, but leave any existing row untouched on conflict.
    ///
    /// This is "make sure the row exists", not "write it" — the counterpart to
    /// [`UpsertBuilder::on_update`]. Catching the conflict in SQL rather than as
    /// an error matters inside a transaction: on Postgres a failed statement
    /// aborts the whole transaction, so a caller that swallowed a unique
    /// violation would then fail on its next statement.
    pub fn on_conflict_do_nothing(mut self) -> Self {
        self.do_nothing = true;
        self
    }

    /// Render the statement and its bound parameters.
    pub fn build(self) -> Result<Upsert> {
        if self.columns.is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "upsert requires at least one column".to_string(),
            )));
        }
        if self.do_nothing && !self.updates.is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "upsert cannot both ignore conflicts and update on conflict".to_string(),
            )));
        }
        if self.dialect.supports_on_conflict() && self.conflict.is_empty() && !self.do_nothing {
            return Err(Error::Validation(ValidationError::Invalid(
                "upsert requires a conflict target on this dialect".to_string(),
            )));
        }
        if !self.dialect.supports_on_conflict() && !self.updates.is_empty() {
            // MySQL's `ON DUPLICATE KEY UPDATE` needs no conflict target, but
            // an assignment naming a column absent from the insert list is
            // still rejected; catch it here rather than at the database.
            for (column, _) in &self.updates {
                if !self.columns.contains(column) {
                    return Err(Error::Validation(ValidationError::Invalid(format!(
                        "upsert updates column {column} which is not inserted"
                    ))));
                }
            }
        }

        let d = self.dialect;
        let table = Ident::new(&self.table)?;
        let columns: Vec<Ident> = self
            .columns
            .iter()
            .map(|c| Ident::new(c))
            .collect::<Result<_>>()?;
        let mut params: Vec<Value> = self.values.clone();
        let columns = columns
            .iter()
            .map(|c| c.quote(d))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = vec!["?"; self.columns.len()].join(", ");

        let mut sql = if self.do_nothing && !d.supports_on_conflict() {
            // MySQL has no `ON CONFLICT DO NOTHING`; `INSERT IGNORE` is its
            // spelling, and it has to precede the keyword.
            format!(
                "INSERT IGNORE INTO {} ({columns}) VALUES ({placeholders})",
                table.quote(d)
            )
        } else {
            format!(
                "INSERT INTO {} ({columns}) VALUES ({placeholders})",
                table.quote(d)
            )
        };

        if self.do_nothing && d.supports_on_conflict() {
            let target = self
                .conflict
                .iter()
                .map(|c| Ident::new(c).map(|i| i.quote(d)))
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            sql.push_str(" ON CONFLICT (");
            sql.push_str(&target);
            sql.push_str(") DO NOTHING");
        } else if !self.updates.is_empty() {
            let mut sets = Vec::with_capacity(self.updates.len());
            for (column, set) in &self.updates {
                sets.push(set.render(&Ident::new(column)?, d, &mut params)?);
            }
            let sets = sets.join(", ");
            if d.supports_on_conflict() {
                let target = self
                    .conflict
                    .iter()
                    .map(|c| Ident::new(c).map(|i| i.quote(d)))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                sql.push_str(" ON CONFLICT (");
                sql.push_str(&target);
                sql.push_str(") DO UPDATE SET ");
            } else {
                sql.push_str(" ON DUPLICATE KEY UPDATE ");
            }
            sql.push_str(&sets);
        }

        Ok(Upsert { sql, params })
    }
}
