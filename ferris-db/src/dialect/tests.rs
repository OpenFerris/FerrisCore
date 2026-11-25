//! Unit tests for the dialect authority.
//!
//! Restored from the inline `#[cfg(test)] mod tests` moved into `tests/`
//! by commit 2013f3d.

use super::*;

#[test]
fn mysql_quotes_with_backticks_so_reserved_words_survive() {
    // `"user"` is a string literal on MySQL unless ANSI_QUOTES is on,
    // which is not the default.
    assert_eq!(Dialect::MySQL.quote_ident("user").unwrap(), "`user`");
    assert_eq!(Dialect::MySQL.quote_ident("key").unwrap(), "`key`");
    assert_eq!(Dialect::MySQL.quote_ident("default").unwrap(), "`default`");
    assert_eq!(Dialect::Postgres.quote_ident("user").unwrap(), "\"user\"");
}

#[test]
fn quote_ident_rejects_injection_and_qualified_names() {
    assert!(Dialect::Postgres.quote_ident("bad name").is_err());
    assert!(Dialect::Postgres.quote_ident("a;DROP").is_err());
    assert!(Dialect::Postgres.quote_ident("").is_err());
    assert!(Dialect::Postgres.quote_ident("tab.name").is_err());
    assert_eq!(Dialect::Postgres.try_quote_ident("tab.name"), None);
    assert_eq!(
        Dialect::Postgres.try_quote_ident("tabname").as_deref(),
        Some("\"tabname\"")
    );
}

#[test]
fn quoting_preserves_case_so_migration_and_runtime_agree() {
    // The unquoted form folds to `tabtodo` on Postgres, which is why every
    // runtime query must quote too or it addresses a different table.
    assert_eq!(
        Dialect::Postgres.quote_ident("tabToDo").unwrap(),
        "\"tabToDo\""
    );
}

#[test]
fn ssl_param() {
    assert_eq!(
        Dialect::Postgres.ssl_param(SslMode::VerifyFull),
        Some("verify-full")
    );
    assert_eq!(Dialect::MySQL.ssl_param(SslMode::Require), Some("REQUIRED"));
    assert_eq!(Dialect::SQLite.ssl_param(SslMode::Require), None);
    assert_eq!(Dialect::MySQL.ssl_param_key(), "ssl-mode");
    assert_eq!(Dialect::Postgres.ssl_param_key(), "sslmode");
}

#[test]
fn dialect_from_url_matches_the_scheme() {
    assert_eq!(Dialect::from_url("postgres://x/y"), Dialect::Postgres);
    assert_eq!(Dialect::from_url("mysql://x/y"), Dialect::MySQL);
    assert_eq!(Dialect::from_url("mariadb://x/y"), Dialect::MySQL);
    assert_eq!(Dialect::from_url("sqlite::memory:"), Dialect::SQLite);
    assert_eq!(Dialect::from_url("sqlite://test.db"), Dialect::SQLite);
}

#[test]
fn capability_flags() {
    assert!(Dialect::Postgres.supports_ilike());
    assert!(!Dialect::MySQL.supports_ilike());
    assert!(!Dialect::SQLite.supports_ilike());
    assert!(!Dialect::MySQL.supports_on_conflict());
    assert!(Dialect::SQLite.supports_on_conflict());
    assert!(!Dialect::MySQL.supports_create_index_if_not_exists());
    assert!(!Dialect::MySQL.allows_default_on_text());
    assert!(Dialect::SQLite.allows_default_on_text());
}

#[test]
fn qualified_ident_validation() {
    assert!(is_qualified_ident("tabToDo"));
    assert!(is_qualified_ident("tabToDo.name"));
    assert!(!is_qualified_ident("tabToDo."));
    assert!(!is_qualified_ident(".name"));
    assert!(!is_qualified_ident("tab.*"));
    assert!(!is_qualified_ident(""));
}

#[test]
fn postgres_placeholders_are_numbered_in_order() {
    let rendered = Dialect::Postgres.render_placeholders("SELECT * FROM t WHERE a = ? AND b = ?");
    assert_eq!(rendered.as_ref(), "SELECT * FROM t WHERE a = $1 AND b = $2");
}

#[test]
fn placeholders_inside_literals_identifiers_and_comments_are_left_alone() {
    let sql = "SELECT '?', \"?\", `?` FROM t /* ? */ -- ?\n WHERE a = ?";
    let rendered = Dialect::Postgres.render_placeholders(sql);
    assert_eq!(rendered.matches("$1").count(), 1);
    assert!(rendered.contains("'?'"));
    assert!(rendered.contains("\"?\""));
    assert!(rendered.contains("/* ? */"));
    assert!(rendered.contains("-- ?"));
    assert!(rendered.ends_with("a = $1"));
}

#[test]
fn a_doubled_quote_does_not_close_a_literal() {
    let rendered = Dialect::Postgres.render_placeholders("SELECT 'it''s ?' WHERE a = ?");
    assert_eq!(rendered.as_ref(), "SELECT 'it''s ?' WHERE a = $1");
}

#[test]
fn dollar_quoted_bodies_are_not_rewritten() {
    let rendered = Dialect::Postgres.render_placeholders("SELECT $$?$$, $tag$?$tag$, ?");
    assert_eq!(rendered.as_ref(), "SELECT $$?$$, $tag$?$tag$, $1");
}

#[test]
fn other_dialects_keep_the_question_mark() {
    assert_eq!(
        Dialect::MySQL.render_placeholders("a = ?").as_ref(),
        "a = ?"
    );
    assert_eq!(
        Dialect::SQLite.render_placeholders("a = ?").as_ref(),
        "a = ?"
    );
}
