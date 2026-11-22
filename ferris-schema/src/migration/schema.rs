//! The shared physical definition of a `tab<DocType>` table.
//!
//! Every DDL path — the migration runner in [`super::runner`] and the document
//! engine's lazy `ensure_table` — derives its table name, columns and indexes
//! from here, so the two can never disagree about what a DocType table is.

use ferris_core::Result;
use ferris_db::sql::{Column, Ident};

/// The physical table backing a DocType.
///
/// Validated as an identifier so it can be quoted into SQL, and derived the
/// same way by every caller — `ToDo` and `To Do` both give `tabToDo`.
pub fn document_table(doctype: &str) -> Result<String> {
    let name: String = doctype.chars().filter(|c| !c.is_whitespace()).collect();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Invalid(format!("Invalid DocType name: {doctype}")),
        ));
    }
    let table = Ident::new(&format!("tab{name}"))?;
    Ok(table.as_str().to_string())
}

/// The columns every `tab<DocType>` table has.
///
/// `data` holds the document as JSON: the engine reads and writes fields through
/// it, so a per-field column would be a second source of truth that nothing
/// keeps in step. `owner`, `docstatus`, `creation` and `modified` are duplicated
/// out of `data` because they are the columns lists sort and filter on.
pub fn document_columns() -> Vec<Column> {
    vec![
        Column::new("name", "VARCHAR(255)").not_null().primary_key(),
        Column::new("owner", "VARCHAR(255)"),
        Column::new("docstatus", "SMALLINT").not_null().default("0"),
        Column::new("creation", "BIGINT").not_null(),
        Column::new("modified", "BIGINT").not_null(),
        Column::new("data", "TEXT").not_null(),
    ]
}

/// The version a DocType's table migration is recorded under.
///
/// M2, then D4/S7: `generate_version` first hashed a DocType name into a
/// 1M bucket, so two DocTypes could share a version and the runner's "strictly
/// increasing" guarantee silently dropped the second table's DDL. The first
/// replacement was a monotonic counter, which fixed the collision but tied each
/// version to the registry *set* — adding or removing any DocType shifted later
/// versions and broke the checksum guard. The version below is derived from the
/// name alone and collision-checked in the runner.
///
/// Derived **only** from the DocType name, so it is stable regardless of which
/// other DocTypes are registered (see
/// [`MigrationRunner::add_schema_migrations`](super::MigrationRunner::add_schema_migrations)
/// for why a positional counter is a trap). The value lives above `1_000_000`,
/// the base reserved for hand-authored core migrations, and below `u32::MAX`.
///
/// FNV-1a over the name, folded into the reserved span. The span is ~10^9 wide,
/// so the birthday-bound collision probability over a realistic registry is
/// negligible — and the runner still *checks* for one and fails loudly rather
/// than letting two tables share a version.
pub fn schema_version(doctype: &str) -> u32 {
    /// First version handed to a generated schema migration. Hand-authored core
    /// migrations sit below this, so the two spaces never overlap.
    const BASE: u32 = 1_000_000;
    /// How many generated versions are available above [`BASE`].
    const SPAN: u32 = u32::MAX - BASE;

    // FNV-1a, 32-bit: trivial, dependency-free and stable across platforms and
    // Rust versions (unlike `DefaultHasher`, which is unspecified and may
    // change between releases — exactly what a persisted version must never do).
    let mut hash: u32 = 0x811c_9dc5;
    for byte in doctype.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    BASE + 1 + hash % (SPAN - 1)
}

/// The indexes every document table needs, as `(logical name, columns, unique)`.
///
/// The names are *logical*: [`ferris_db::sql::ensure_index`] qualifies them
/// with the table before rendering (`idx_<table>_owner`). Postgres and SQLite
/// keep index names in one schema-wide namespace, so the old fixed
/// `idx_owner`/`idx_modified`/`idx_docstatus` made `CREATE INDEX IF NOT EXISTS`
/// index only the first DocType table and silently skip every other one
/// (SCH-03 / PERF-16).
pub fn document_indexes() -> Vec<(&'static str, &'static [&'static str], bool)> {
    vec![
        ("idx_owner", &["owner"], false),
        ("idx_modified", &["modified"], false),
        ("idx_docstatus", &["docstatus"], false),
    ]
}

/// Whether `field` is backed by a real column of a `tab<DocType>` table *and*
/// holds one document field, so a declared index may name it as a column.
///
/// Derived from [`document_columns`] so an extra column added there is
/// automatically indexable here. `data` is deliberately the one exception: it
/// *is* a column, but it holds the whole document rather than a single field,
/// so a `meta.indexes` entry naming it is meaningless.
pub fn is_indexable_column(field: &str) -> bool {
    field != "data" && document_columns().iter().any(|c| c.name == field)
}

/// How a [`crate::registry::IndexMeta`] entry materializes against the
/// blob-backed table.
///
/// `meta.indexes` is the DocType's own declaration, and until now neither the
/// lazy engine nor the migration runner used it for DDL: lazily-created tables
/// only got [`document_indexes`], and `ferris migrate` runs in a process
/// without plugins. The two DDL paths now share this classification so a
/// table created by the engine and one created by a migration stay the same
/// table (the invariant [`document_columns`] already protects).
pub enum DeclaredIndex<'a> {
    /// Every named field is a column: a plain [`ferris_db::sql::ensure_index`].
    Columns,
    /// One field with no column: the value lives in `data`, and the index must
    /// be a JSON expression index over it
    /// ([`ferris_db::sql::ensure_json_index`]).
    JsonField(&'a str),
    /// A composite that names a field without a column. There is no portable
    /// spelling (MySQL would need one generated column per field), so it is
    /// skipped — the same as today's status quo, where `meta.indexes` was
    /// ignored altogether.
    Unsupported,
}

/// Classify one declared index. See [`DeclaredIndex`].
pub fn classify_index(index: &crate::registry::IndexMeta) -> DeclaredIndex<'_> {
    if index.fields.is_empty() {
        return DeclaredIndex::Unsupported;
    }
    if index.fields.iter().all(|f| is_indexable_column(f)) {
        return DeclaredIndex::Columns;
    }
    match index.fields.as_slice() {
        [only] => DeclaredIndex::JsonField(only),
        _ => DeclaredIndex::Unsupported,
    }
}
