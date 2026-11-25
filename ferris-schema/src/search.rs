//! Per-engine full-text index behind `ListQuery::search`.
//!
//! The document engine stores every field in one JSON blob, so a search used to
//! be a full table scan decoded in Rust. This module is the other half of the
//! replacement: one global table, `_ferris_search`, with a `(doctype, name)`
//! key and the document's text in `body`, indexed by whatever the backend
//! offers natively.
//!
//! | Dialect | Table | Index | Match |
//! |---|---|---|---|
//! | SQLite | FTS5 virtual table (`unicode61`) | built in | `MATCH` |
//! | Postgres | ordinary table | GIN over `to_tsvector('simple', body)` | `@@ plainto_tsquery` |
//! | MySQL | ordinary table | `FULLTEXT(body)` | `MATCH … AGAINST … IN BOOLEAN MODE` |
//!
//! The prefix is reserved: `_ferris_search` is not a `DocType` and no `tab…`
//! naming rule can produce it. The statements here are only *built*; running
//! them is the document engine's business, because it owns the failure policy
//! (an unavailable index means the Rust scan remains, never a failed write).
//!
//! **Semantics.** Native full-text matches *tokens*, not substrings: a search
//! for `alpha` finds a document containing the word `alpha`, not one containing
//! `alphabet`. SQLite and MySQL are given the term as a quoted phrase so a value
//! like `p-1` (several tokens) still matches the name `P-1` and punctuation
//! cannot turn into query syntax. Postgres takes the raw term through
//! `plainto_tsquery`, which already treats it as plain text and ANDs its lexemes.

use ferris_core::Value;
use ferris_db::{Dialect, Stmt};

/// The global search table. Reserved: not a `DocType`, never `tab`-prefixed.
pub const TABLE: &str = "_ferris_search";

/// DDL that creates the table and its text index, for `dialect`.
///
/// Each string is one script; Postgres needs two statements (table, then the
/// GIN index), the other backends need one. Every statement is idempotent, so
/// running the whole list on a migrated database is a no-op.
#[must_use]
pub fn ensure_statements(dialect: Dialect) -> Vec<String> {
    match dialect {
        Dialect::SQLite => vec![format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {TABLE} \
             USING fts5(doctype, name, body, tokenize='unicode61')"
        )],
        Dialect::Postgres => vec![
            format!(
                "CREATE TABLE IF NOT EXISTS {TABLE} (\
                 doctype VARCHAR(255) NOT NULL, \
                 name VARCHAR(255) NOT NULL, \
                 body TEXT NOT NULL, \
                 PRIMARY KEY (doctype, name))"
            ),
            format!(
                "CREATE INDEX IF NOT EXISTS {TABLE}_ft \
                 ON {TABLE} USING GIN (to_tsvector('simple', body))"
            ),
        ],
        Dialect::MySQL => vec![format!(
            "CREATE TABLE IF NOT EXISTS {TABLE} (\
             doctype VARCHAR(255) NOT NULL, \
             name VARCHAR(255) NOT NULL, \
             body LONGTEXT NOT NULL, \
             PRIMARY KEY (doctype, name), \
             FULLTEXT KEY {TABLE}_ft (body)) ENGINE=InnoDB"
        )],
    }
}

/// Statements that replace `(doctype, name)`'s body with `body`.
///
/// SQLite answers with two statements because an FTS5 table can be deleted from
/// by `WHERE` but not upserted; the others have a native upsert. The caller runs
/// them in order on the same connection.
#[must_use]
pub fn upsert(dialect: Dialect, doctype: &str, name: &str, body: &str) -> Vec<Stmt> {
    let doc = || Value::from(doctype);
    let doc_name = || Value::from(name);
    match dialect {
        Dialect::SQLite => vec![
            Stmt::new(
                format!("DELETE FROM {TABLE} WHERE doctype = ? AND name = ?"),
                vec![doc(), doc_name()],
            ),
            Stmt::new(
                format!("INSERT INTO {TABLE} (doctype, name, body) VALUES (?, ?, ?)"),
                vec![doc(), doc_name(), Value::from(body)],
            ),
        ],
        Dialect::Postgres => vec![Stmt::new(
            format!(
                "INSERT INTO {TABLE} (doctype, name, body) VALUES (?, ?, ?) \
                 ON CONFLICT (doctype, name) DO UPDATE SET body = EXCLUDED.body"
            ),
            vec![doc(), doc_name(), Value::from(body)],
        )],
        Dialect::MySQL => vec![Stmt::new(
            format!(
                "INSERT INTO {TABLE} (doctype, name, body) VALUES (?, ?, ?) \
                 ON DUPLICATE KEY UPDATE body = VALUES(body)"
            ),
            vec![doc(), doc_name(), Value::from(body)],
        )],
    }
}

/// Statement that drops `(doctype, name)` from the index.
#[must_use]
pub fn remove(_dialect: Dialect, doctype: &str, name: &str) -> Stmt {
    Stmt::new(
        format!("DELETE FROM {TABLE} WHERE doctype = ? AND name = ?"),
        vec![Value::from(doctype), Value::from(name)],
    )
}

/// Statement selecting the `name` column of every matching document.
///
/// The term is bound, never interpolated. A malformed term that the backend
/// still rejects is an error the caller turns into the Rust scan fallback.
#[must_use]
pub fn search(dialect: Dialect, doctype: &str, term: &str) -> Stmt {
    match dialect {
        Dialect::SQLite => Stmt::new(
            format!("SELECT name FROM {TABLE} WHERE doctype = ? AND {TABLE} MATCH ?"),
            vec![Value::from(doctype), Value::from(phrase(term))],
        ),
        Dialect::Postgres => Stmt::new(
            format!(
                "SELECT name FROM {TABLE} WHERE doctype = ? \
                 AND to_tsvector('simple', body) @@ plainto_tsquery('simple', ?)"
            ),
            vec![Value::from(doctype), Value::from(term)],
        ),
        Dialect::MySQL => Stmt::new(
            format!(
                "SELECT name FROM {TABLE} WHERE doctype = ? \
                 AND MATCH(body) AGAINST (? IN BOOLEAN MODE)"
            ),
            vec![Value::from(doctype), Value::from(phrase(term))],
        ),
    }
}

/// A double-quoted match phrase with the characters that would end it removed.
///
/// SQLite (`FTS5`) and MySQL (boolean mode) parse the match string as a query
/// language, so a raw term such as `p-1` would be read as `p` minus `1` and a
/// stray quote would be a syntax error. Quoting keeps a multi-token value
/// together as an adjacent phrase; dropping quotes and backslashes keeps the
/// phrase well formed.
fn phrase(term: &str) -> String {
    let cleaned: String = term.chars().filter(|c| *c != '"' && *c != '\\').collect();
    format!("\"{cleaned}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_dialect_gets_its_own_table_and_upsert() {
        let sqlite = ensure_statements(Dialect::SQLite);
        assert_eq!(sqlite.len(), 1);
        assert!(sqlite[0].contains("USING fts5"));
        assert!(sqlite[0].contains("tokenize='unicode61'"));

        let pg = ensure_statements(Dialect::Postgres);
        assert_eq!(pg.len(), 2, "table and GIN index are separate statements");
        assert!(pg[0].contains("PRIMARY KEY (doctype, name)"));
        assert!(pg[1].contains("USING GIN (to_tsvector('simple', body))"));

        let mysql = ensure_statements(Dialect::MySQL);
        assert_eq!(mysql.len(), 1);
        assert!(mysql[0].contains("FULLTEXT KEY"));
        assert!(mysql[0].contains("ENGINE=InnoDB"));

        assert_eq!(
            upsert(Dialect::SQLite, "T", "n", "b").len(),
            2,
            "FTS5 deletes then inserts"
        );
        assert_eq!(upsert(Dialect::Postgres, "T", "n", "b").len(), 1);
        assert_eq!(upsert(Dialect::MySQL, "T", "n", "b").len(), 1);
        assert!(
            upsert(Dialect::MySQL, "T", "n", "b")[0]
                .sql()
                .contains("ON DUPLICATE KEY UPDATE")
        );
        assert!(
            upsert(Dialect::Postgres, "T", "n", "b")[0]
                .sql()
                .contains("ON CONFLICT")
        );
    }

    #[test]
    fn search_binds_the_term_and_keeps_punctuation_inside_a_phrase() {
        let sqlite = search(Dialect::SQLite, "T", "p-1");
        assert!(sqlite.sql().contains("MATCH ?"));
        assert_eq!(sqlite.params().len(), 2);
        assert!(matches!(&sqlite.params()[1], Value::String(s) if s == "\"p-1\""));

        let pg = search(Dialect::Postgres, "T", "p-1");
        assert!(pg.sql().contains("plainto_tsquery('simple', ?)"));

        let mysql = search(Dialect::MySQL, "T", "p-1");
        assert!(mysql.sql().contains("IN BOOLEAN MODE"));
        assert!(matches!(&mysql.params()[1], Value::String(s) if s == "\"p-1\""));
    }

    #[test]
    fn a_quote_cannot_break_out_of_the_phrase() {
        let stmt = search(Dialect::SQLite, "T", "a\" b\\c");
        assert!(matches!(&stmt.params()[1], Value::String(s) if s == "\"a bc\""));
    }
}
