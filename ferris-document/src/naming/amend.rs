//! Amendment names: `NAME-1`, `NAME-2`, …

use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::sql::Ident;
use ferris_db::{Db, Stmt};

/// The next free `name` for an amended document: `NAME-1`, `NAME-2`, …
///
/// Frappe appends a counter to the amended copy
/// (<https://docs.frappe.io/framework/user/en/basics/doctypes/naming>, "Special
/// rules"), and a deleted amendment never reuses its number. The lookup is a
/// `LIKE prefix%` scan scoped to one document's amendments, not a table scan.
///
/// The `LIKE` pattern escapes `!`, `%` and `_` with `!` instead of a backslash:
/// MySQL treats a backslash inside a string literal as an escape character, so
/// `ESCAPE '\'` is not portable while `ESCAPE '!'` is.
pub async fn next_amend_name(db: &Db, doctype: &str, name: &str) -> Result<String> {
    let table = ferris_schema::migration::document_table(doctype)?;
    let quoted = Ident::new(&table)?.quote(db.dialect());
    let prefix = format!("{name}-");
    let escaped = prefix
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    let rows = db
        .fetch_all(&Stmt::new(
            format!("SELECT name FROM {quoted} WHERE name LIKE ? ESCAPE '!'"),
            vec![Value::from(format!("{escaped}%"))],
        ))
        .await?;

    let mut next = 1_u64;
    for row in rows {
        let existing = row.str("name")?;
        let Some(suffix) = existing.strip_prefix(&prefix) else {
            continue;
        };
        if let Ok(number) = suffix.parse::<u64>() {
            next = next.max(number.saturating_add(1));
        }
    }
    // The numeric scan covers the normal case; the explicit existence check
    // covers a collation that matched a differently-cased row, and any literal
    // `NAME-<non-number>` row, so an amendment never collides on the primary key.
    loop {
        let candidate = format!("{prefix}{next}");
        if candidate.len() > 255 {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "the amendment name for `{name}` would exceed 255 characters"
            ))));
        }
        let exists = db
            .fetch_optional(&Stmt::new(
                format!("SELECT name FROM {quoted} WHERE name = ?"),
                vec![Value::from(candidate.clone())],
            ))
            .await?;
        if exists.is_none() {
            return Ok(candidate);
        }
        next = next.saturating_add(1);
    }
}
