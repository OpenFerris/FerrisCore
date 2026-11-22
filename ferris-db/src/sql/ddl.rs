//! Portable DDL: `CREATE TABLE`, indexes and `ALTER TABLE ... ADD COLUMN`.
//!
//! Oracle MySQL rejects `CREATE INDEX IF NOT EXISTS` and a `DEFAULT` on a
//! `TEXT`/`BLOB` column, so a table definition cannot be one template: the
//! dialect decides whether an existing object is tolerated in SQL or by the
//! caller (see [`crate::exec::ensure_index`]) and whether a text column has to
//! be widened. Centralising it here is what lets every other crate keep
//! calling one builder instead of branching on the backend.

use crate::dialect::Dialect;
use crate::sql::Ident;
use ferris_core::{Error, Result, ValidationError};

/// One column in a [`create_table`] definition.
///
/// The name is validated by [`create_table`], not here, so a table definition
/// reads as data rather than as a chain of `Result`s.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub ty: String,
    pub not_null: bool,
    /// Already-rendered SQL literal, or a bare keyword such as `0` / `NULL`.
    pub default: Option<String>,
    /// Part of the table's primary key. See [`Column::primary_key`].
    pub primary_key: bool,
    /// The database assigns this column. See [`Column::auto_increment`].
    pub auto_increment: bool,
}

impl Column {
    pub fn new(name: impl Into<String>, ty: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ty: ty.into(),
            not_null: false,
            default: None,
            primary_key: false,
            auto_increment: false,
        }
    }

    pub fn not_null(mut self) -> Self {
        self.not_null = true;
        self
    }

    pub fn default(mut self, literal: impl Into<String>) -> Self {
        self.default = Some(literal.into());
        self
    }

    /// Put this column in the table's primary key.
    ///
    /// Marking two or more columns makes a composite key, which is how the
    /// framework's `(doctype, name)` and `(doctype, docname, version)` tables
    /// are keyed. A primary key is not optional decoration: an
    /// [`crate::sql::UpsertBuilder`] conflict target must name a unique
    /// constraint, so a table upserted into needs one declared here rather than
    /// bolted on with a separate statement.
    pub fn primary_key(mut self) -> Self {
        self.primary_key = true;
        self
    }

    /// Let the database assign this column's value.
    ///
    /// Implies [`Column::primary_key`] — a surrogate key with no uniqueness
    /// constraint behind it is not a key. Use it instead of computing the next
    /// id in Rust, which races two writers onto the same value.
    ///
    /// The three backends spell this differently and `create_table` renders
    /// each: `SERIAL PRIMARY KEY` on Postgres, `AUTO_INCREMENT` on MySQL, and an
    /// inline `INTEGER PRIMARY KEY AUTOINCREMENT` on SQLite — where an inline
    /// declaration is required for the column to become a `rowid` alias, which
    /// is why this case is rendered outside the table-constraint form.
    pub fn auto_increment(mut self) -> Self {
        self.auto_increment = true;
        self.primary_key = true;
        self
    }
}

/// `CREATE TABLE IF NOT EXISTS` statement for `table`.
///
/// On MySQL a `TEXT`/`BLOB` column cannot carry a `DEFAULT`, so such a column
/// is widened to `VARCHAR(255)` when it has one; without that the statement is
/// rejected outright by the server.
pub fn create_table(dialect: Dialect, table: &str, columns: &[Column]) -> Result<String> {
    if columns.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "table {table} has no columns"
        ))));
    }
    let table = Ident::new(table)?;

    let mut defs = Vec::with_capacity(columns.len() + 1);
    let mut names: Vec<&str> = Vec::with_capacity(columns.len());
    let mut surrogate: Option<&str> = None;
    for column in columns {
        let name = Ident::new(&column.name)?;
        if names.contains(&column.name.as_str()) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "table {} declares column {} twice",
                table.as_str(),
                name.as_str()
            ))));
        }
        names.push(&column.name);

        if column.auto_increment {
            if surrogate.is_some() {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "table {} declares more than one auto-increment column",
                    table.as_str()
                ))));
            }
            if !is_integer_type(&column.ty) {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "table {} auto-increments non-integer column {}",
                    table.as_str(),
                    name.as_str()
                ))));
            }
            surrogate = Some(&column.name);
            defs.push(format!(
                "{} {}",
                name.quote(dialect),
                surrogate_type(dialect, &column.ty)
            ));
            continue;
        }

        let mut ty = column.ty.clone();
        if column.default.is_some() && !dialect.allows_default_on_text() && is_text_type(&ty) {
            ty = "VARCHAR(255)".to_string();
        }
        let mut def = format!("{} {}", name.quote(dialect), ty);
        if column.not_null {
            def.push_str(" NOT NULL");
        }
        if let Some(default) = &column.default {
            def.push_str(" DEFAULT ");
            def.push_str(default);
        }
        defs.push(def);
    }

    // A composite key cannot be written inline, so the primary key always goes
    // in as a table constraint; that spelling is also valid for a single
    // column, which keeps the two cases from diverging. The one exception is
    // the surrogate key, whose SQLite form only works inline.
    let key: Vec<String> = columns
        .iter()
        .filter(|c| c.primary_key && !c.auto_increment)
        .map(|c| Ident::new(&c.name).map(|i| i.quote(dialect)))
        .collect::<Result<_>>()?;
    match surrogate {
        Some(_) => {}
        None if key.is_empty() => {}
        None => defs.push(format!("PRIMARY KEY ({})", key.join(", "))),
    }

    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        table.quote(dialect),
        defs.join(", ")
    ))
}

/// The declared type of a surrogate key, including the clause that makes the
/// database fill it in.
fn surrogate_type(dialect: Dialect, ty: &str) -> String {
    match dialect {
        // `SERIAL` implies NOT NULL and a sequence; there is no separate clause.
        Dialect::Postgres => "SERIAL PRIMARY KEY".to_string(),
        Dialect::MySQL => {
            format!(
                "{} NOT NULL AUTO_INCREMENT PRIMARY KEY",
                ty.to_ascii_uppercase()
            )
        }
        // The column must be declared exactly `INTEGER PRIMARY KEY AUTOINCREMENT`
        // to become a `rowid` alias; any other spelling makes it a plain
        // NOT NULL column with no default, and the insert fails.
        Dialect::SQLite => "INTEGER PRIMARY KEY AUTOINCREMENT".to_string(),
    }
}

fn is_integer_type(ty: &str) -> bool {
    let upper = ty.trim().to_ascii_uppercase();
    upper.contains("INT") && !upper.contains("POINT")
}

fn is_text_type(ty: &str) -> bool {
    let upper = ty.trim().to_ascii_uppercase();
    upper.starts_with("TEXT") || upper.starts_with("BLOB")
}

/// The physical name an index is created under, qualified with its table.
///
/// Postgres and SQLite keep index names in the schema namespace rather than
/// under the table, so the same short name on two `tab<DocType>` tables
/// collides and `IF NOT EXISTS` silently leaves every table but the first
/// unindexed. Qualifying the name with the table fixes that on every backend;
/// MySQL scopes names per table, where the prefix is redundant but harmless.
///
/// A name already qualified as `idx_<table>_…` is returned unchanged, so a
/// caller may pass either form and repeated DDL stays idempotent.
fn qualified_index_name(table: &Ident, name: &str) -> Result<String> {
    let name = Ident::new(name)?;
    let name = name.as_str();
    let prefix = format!("idx_{}_", table.as_str());
    if name.starts_with(&prefix) {
        return Ok(name.to_string());
    }
    // Drop the redundant `idx_` of a caller-supplied short name so
    // `idx_owner` becomes `idx_tabToDo_owner`, not `idx_tabToDo_idx_owner`.
    let short = name.strip_prefix("idx_").unwrap_or(name);
    Ok(format!("{prefix}{short}"))
}

/// The statement that creates `name` on `table` if it is absent.
///
/// On MySQL — where `CREATE INDEX IF NOT EXISTS` does not exist — this returns
/// a plain `CREATE INDEX`; the caller is expected to run it through
/// [`crate::exec::ensure_index`], which tolerates the duplicate-name error.
/// On Postgres and SQLite the statement is already idempotent.
///
/// The rendered name always carries the table (see [`qualified_index_name`]),
/// because a schema-wide index namespace makes a fixed name index only the
/// first table that asks for it.
pub fn ensure_index(
    dialect: Dialect,
    name: &str,
    table: &str,
    columns: &[&str],
    unique: bool,
) -> Result<String> {
    if columns.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "index {name} has no columns"
        ))));
    }
    let table = Ident::new(table)?;
    let name = qualified_index_name(&table, name)?;
    let cols = columns
        .iter()
        .map(|c| Ident::new(c).map(|i| i.quote(dialect)))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    let unique = if unique { "UNIQUE " } else { "" };
    let guard = if dialect.supports_create_index_if_not_exists() {
        "IF NOT EXISTS "
    } else {
        ""
    };
    Ok(format!(
        "CREATE {unique}INDEX {guard}{} ON {} ({cols})",
        Ident::new(&name)?.quote(dialect),
        table.quote(dialect)
    ))
}

/// The generated column that mirrors `data.field` on MySQL.
///
/// MySQL is the one backend of the three that cannot index a *function* of a
/// column directly for this payload, so the value of `data.field` is
/// materialised into a column that the index (and every `WHERE` over it) can
/// name. The name is derived deterministically from the field, validated as an
/// identifier, and shared between [`json_field_target`] and
/// [`ensure_json_index`] so the DDL and the queries it serves can never drift.
///
/// The prefix `g_` is reserved for this mechanism: a document field is read
/// from the JSON blob and is never a physical column, so `g_<field>` cannot
/// collide with user data.
pub fn json_field_column(field: &str) -> Result<String> {
    let field = Ident::new(field)?;
    Ok(format!("g_{}", field.as_str()))
}

/// The SQL expression that yields the plain text value of `data.field`.
///
/// This is the value the Rust filter path compares against, so every dialect
/// spelling here must strip JSON quoting: `json_extract` on SQLite and
/// `->>` on Postgres already return the unquoted text, and MySQL's generated
/// column is declared `BINARY` so a comparison is byte-for-byte and
/// case-sensitive (the framework's exact match, not a collation-aware one).
///
/// It is also the exact expression the expression index is created over, so a
/// `WHERE` built from this can be served by the index built by
/// [`ensure_json_index`].
///
/// # Errors
/// Returns the identifier error if `field` is not a valid column identifier —
/// a field named that way could never exist in the `data` blob either.
pub fn json_field_target(dialect: Dialect, field: &str) -> Result<String> {
    let field = Ident::new(field)?;
    let name = field.as_str();
    Ok(match dialect {
        // `json_extract` returns the JSON value; for a JSON string that is the
        // unquoted text, which is what the Rust path compares.
        Dialect::SQLite => {
            let data = Ident::new("data")?.quote(dialect);
            format!("json_extract({data}, '$.{name}')")
        }
        // `->>` returns text. Postgres has no `json_extract`, and an
        // expression index over the operator needs the whole expression
        // parenthesised, which is why the target carries its own parens.
        Dialect::Postgres => {
            let data = Ident::new("data")?.quote(dialect);
            format!("({data}::jsonb ->> '{name}')")
        }
        // The generated column IS the value on MySQL; naming it lets the
        // optimizer use its index and keeps the comparison binary.
        Dialect::MySQL => Ident::new(&json_field_column(name)?)?.quote(dialect),
    })
}

/// The statements that index the exact value of `data.field`, per dialect.
///
/// Document fields live in the `data` JSON blob, so a declared index over such
/// a field is not an index over a column. The three backends answer differently:
///
/// * **SQLite / Postgres** — an *expression index* over `json_extract` /
///   `data::jsonb ->>`, in one idempotent statement.
/// * **MySQL 8** — a **generated column** mirroring the value plus an index on
///   it (expression indexes over `JSON_EXTRACT` are not reliable there). The
///   column is `VARBINARY(255)`: binary so the comparison is case-sensitive,
///   bounded so the index has a key length. Values longer than 255 bytes are
///   compared by their first 255 bytes on MySQL — the documented limit of this
///   mechanism.
///
/// Callers run these through [`crate::exec::ensure_json_index`], which tolerates
/// the duplicate-index *and* duplicate-column errors that make lazy re-creation
/// idempotent on every dialect.
///
/// # Errors
/// Returns the first identifier-quoting failure for `name`, `table` or `field`.
pub fn ensure_json_index(
    dialect: Dialect,
    name: &str,
    table: &str,
    field: &str,
    unique: bool,
) -> Result<Vec<String>> {
    let table = Ident::new(table)?;
    let name = qualified_index_name(&table, name)?;
    let index = Ident::new(&name)?;
    let unique = if unique { "UNIQUE " } else { "" };
    match dialect {
        Dialect::SQLite | Dialect::Postgres => {
            let guard = if dialect.supports_create_index_if_not_exists() {
                "IF NOT EXISTS "
            } else {
                ""
            };
            let target = json_field_target(dialect, field)?;
            Ok(vec![format!(
                "CREATE {unique}INDEX {guard}{} ON {} ({target})",
                index.quote(dialect),
                table.quote(dialect)
            )])
        }
        Dialect::MySQL => {
            let column = Ident::new(&json_field_column(field)?)?;
            let data = Ident::new("data")?;
            let expr = format!(
                "BINARY json_unquote(json_extract({}, '$.{}'))",
                data.quote(dialect),
                Ident::new(field)?.as_str()
            );
            let alter = format!(
                "ALTER TABLE {} ADD COLUMN {} VARBINARY(255) \
                 GENERATED ALWAYS AS ({expr}) VIRTUAL",
                table.quote(dialect),
                column.quote(dialect)
            );
            let create = format!(
                "CREATE {unique}INDEX {} ON {} ({})",
                index.quote(dialect),
                table.quote(dialect),
                column.quote(dialect)
            );
            Ok(vec![alter, create])
        }
    }
}

/// The statement that adds `column` to `table`.
///
/// There is no portable `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` (MySQL took
/// it out of 8.0 and SQLite has no such syntax), so the caller runs this
/// through [`crate::exec::ensure_column`], which tolerates the
/// duplicate-column error and makes the migration idempotent across boots.
///
/// # Errors
/// Returns the first identifier-quoting failure for `table` or `column`.
#[allow(clippy::result_large_err)] // the crate-wide `Error` type is intentionally unchanged
pub fn add_column(dialect: Dialect, table: &str, column: &Column) -> Result<String> {
    let table = Ident::new(table)?;
    let name = Ident::new(&column.name)?;

    // Same widening `create_table` applies: MySQL rejects a `DEFAULT` on TEXT.
    let mut ty = column.ty.clone();
    if column.default.is_some() && !dialect.allows_default_on_text() && is_text_type(&ty) {
        ty = "VARCHAR(255)".to_string();
    }
    let mut def = format!("{} {}", name.quote(dialect), ty);
    if column.not_null {
        def.push_str(" NOT NULL");
    }
    if let Some(default) = &column.default {
        def.push_str(" DEFAULT ");
        def.push_str(default);
    }
    Ok(format!(
        "ALTER TABLE {} ADD COLUMN {def}",
        table.quote(dialect)
    ))
}
