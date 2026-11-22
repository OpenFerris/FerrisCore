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
fn escape_string_literal() {
    assert_eq!(Dialect::Postgres.escape_string_literal("it's"), "it''s");
    assert_eq!(Dialect::MySQL.escape_string_literal("a\\b"), "a\\\\b");
    assert!(
        !Dialect::Postgres
            .escape_string_literal("a\\b")
            .contains("\\\\")
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
