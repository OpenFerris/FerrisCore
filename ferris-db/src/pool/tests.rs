//! Unit tests for pool construction.
//!
//! Restored from the inline `#[cfg(test)] mod tests` moved into `tests/`
//! by commit 2013f3d.

use super::*;
use crate::Stmt;
use ferris_core::config::SslMode;

fn config(r#type: DatabaseType) -> DatabaseConfig {
    let (host, port) = match r#type {
        DatabaseType::Postgres => ("localhost", 5432),
        DatabaseType::MySQL => ("localhost", 3306),
        DatabaseType::SQLite => ("", 0),
    };
    DatabaseConfig {
        r#type,
        host: host.into(),
        port,
        name: "testdb".into(),
        user: "user".into(),
        password: "pass".into(),
        ssl_mode: SslMode::Prefer,
        schema: None,
        ..DatabaseConfig::default()
    }
}

#[test]
fn postgres_url_carries_credentials_and_ssl_mode() {
    let url = database_url(&config(DatabaseType::Postgres));
    assert!(url.starts_with("postgres://"));
    assert!(url.contains("user:pass@localhost:5432/testdb"), "{url}");
    assert!(url.ends_with("?sslmode=prefer"), "{url}");
}

#[test]
fn mysql_url_uses_the_ssl_mode_key_mysql_understands() {
    let url = database_url(&config(DatabaseType::MySQL));
    assert!(url.starts_with("mysql://"), "{url}");
    assert!(url.contains("user:pass@localhost:3306/testdb"), "{url}");
    // `sslmode` is not a MySQL parameter; it must be `ssl-mode`.
    assert!(url.ends_with("?ssl-mode=PREFERRED"), "{url}");
}

#[test]
fn sqlite_memory_does_not_become_a_file() {
    let mut cfg = config(DatabaseType::SQLite);
    cfg.name = ":memory:".into();
    assert_eq!(database_url(&cfg), "sqlite::memory:");

    cfg.name = String::new();
    assert_eq!(database_url(&cfg), "sqlite::memory:");
}

#[test]
fn a_missing_sqlite_file_is_created() {
    let mut cfg = config(DatabaseType::SQLite);
    cfg.name = "test.db".into();
    // `mode=rwc` is what makes a non-existent file appear.
    assert_eq!(database_url(&cfg), "sqlite://test.db?mode=rwc");
}

#[test]
fn sqlite_never_gets_an_ssl_parameter() {
    let mut cfg = config(DatabaseType::SQLite);
    cfg.name = "test.db".into();
    cfg.ssl_mode = SslMode::VerifyFull;
    assert!(!database_url(&cfg).contains("ssl"));
}

#[test]
fn invalid_pool_sizes_are_rejected_before_connecting() {
    let mut cfg = config(DatabaseType::Postgres);
    cfg.max_connections = 0;
    assert!(validate(&cfg).is_err());

    let mut cfg = config(DatabaseType::Postgres);
    cfg.max_connections = 2;
    cfg.min_connections = 5;
    assert!(validate(&cfg).is_err());

    let mut cfg = config(DatabaseType::Postgres);
    cfg.max_connections = 5;
    cfg.min_connections = 5;
    assert!(validate(&cfg).is_ok());
}

#[tokio::test]
async fn an_in_memory_pool_is_usable_and_reports_its_dialect() {
    let mut cfg = config(DatabaseType::SQLite);
    cfg.name = ":memory:".into();
    let db = open_unmanaged(&cfg).await.expect("pool opens");
    assert_eq!(db.dialect(), Dialect::SQLite);
    db.health_check().await.expect("ping succeeds");
}

/// The SQLite connection applies the pragmas that keep a write-heavy install
/// from serializing on the default rollback journal and a 2 MiB page cache.
#[tokio::test]
async fn sqlite_connection_pragmas_are_applied() {
    let db = memory_db().await.unwrap();
    let foreign_keys = db
        .fetch_one(&Stmt::raw("PRAGMA foreign_keys"))
        .await
        .unwrap();
    assert_eq!(foreign_keys.opt_i64("foreign_keys"), Some(1));
    let busy = db
        .fetch_one(&Stmt::raw("PRAGMA busy_timeout"))
        .await
        .unwrap();
    assert_eq!(busy.opt_i64("timeout"), Some(5000));
    let cache = db.fetch_one(&Stmt::raw("PRAGMA cache_size")).await.unwrap();
    assert_eq!(cache.opt_i64("cache_size"), Some(-16000));
}

/// A file-backed SQLite database opens in WAL mode, not the rollback journal
/// that serializes every reader behind the writer.
#[tokio::test]
async fn a_sqlite_file_uses_wal() {
    let path = std::env::temp_dir().join(format!(
        "ferris-wal-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos())
    ));
    let config = DatabaseConfig {
        r#type: DatabaseType::SQLite,
        name: path.to_string_lossy().into_owned(),
        max_connections: 2,
        min_connections: 0,
        ..DatabaseConfig::default()
    };
    let db = open_unmanaged(&config).await.expect("the file opens");
    let mode = db
        .fetch_one(&Stmt::raw("PRAGMA journal_mode"))
        .await
        .expect("journal_mode");
    assert_eq!(mode.opt_str("journal_mode").as_deref(), Some("wal"));
    db.close().await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
