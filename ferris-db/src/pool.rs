//! Pool creation.
//!
//! [`create_pool`] is the only place a database connection pool is built in the
//! framework. It applies the configured SSL mode, creates a missing SQLite
//! file and installs the migration bookkeeping table. The thirteen other
//! constructions the framework had each skipped some of that — notably the SSL
//! mode, which a tenant pool silently ignored.

use crate::dialect::Dialect;
use crate::exec::{Db, open_pool};
use ferris_core::{DatabaseConfig, DatabaseType, Error, Result};

/// Create a pool from `config`, ready to use.
pub async fn create_pool(config: &DatabaseConfig) -> Result<Db> {
    let db = Db::connect(config).await?;
    crate::migration::ensure_bookkeeping(&db).await?;
    Ok(db)
}

/// The connection URL for `config`, with the SSL mode applied.
///
/// Exposed so the CLI can report the URL it would connect with without opening
/// a socket.
pub fn database_url(config: &DatabaseConfig) -> String {
    match config.r#type {
        DatabaseType::Postgres | DatabaseType::MySQL => {
            let dialect = Dialect::from_database_type(config.r#type);
            let mut url = config.connection_url();
            if let Some(value) = dialect.ssl_param(config.ssl_mode) {
                let separator = if url.contains('?') { '&' } else { '?' };
                url.push(separator);
                url.push_str(dialect.ssl_param_key());
                url.push('=');
                url.push_str(value);
            }
            url
        }
        DatabaseType::SQLite => {
            // `:memory:` must use sqlx's special `sqlite::memory:` URL,
            // otherwise a file literally named `:memory:` is created.
            // `connection_url()` already covers that case; the file variant
            // adds `mode=rwc` so a missing database file is created.
            if config.name.is_empty() || config.name == ":memory:" {
                "sqlite::memory:".to_string()
            } else {
                format!("sqlite://{}?mode=rwc", config.name)
            }
        }
    }
}

/// Open a pool without running migrations.
///
/// For callers that manage schema themselves, such as tests and tools.
pub async fn open_unmanaged(config: &DatabaseConfig) -> Result<Db> {
    validate(config)?;
    Ok(Db::new(open_pool(config).await?, config))
}

/// A fresh in-memory SQLite database.
///
/// Tests across the workspace need somewhere to put tables and rows; building
/// the config by hand in each one used to be the only way, and the copies
/// drifted. One connection is the right ceiling for `:memory:` — a second would
/// see a different, empty database.
pub async fn memory_db() -> Result<Db> {
    open_unmanaged(&DatabaseConfig {
        r#type: DatabaseType::SQLite,
        name: ":memory:".to_string(),
        max_connections: 1,
        min_connections: 0,
        ..DatabaseConfig::default()
    })
    .await
}

/// Reject a configuration that cannot be served, before any socket is opened.
///
/// A `min_connections` above `max_connections` is not an error in sqlx — it
/// silently clamps — so it is worth catching here rather than debugging a pool
/// that never reaches its configured floor.
pub fn validate(config: &DatabaseConfig) -> Result<()> {
    if config.max_connections == 0 {
        return Err(Error::Config(ferris_core::error::ConfigError::Invalid {
            key: "database.max_connections".to_string(),
            msg: "must be greater than 0".to_string(),
        }));
    }
    if config.min_connections > config.max_connections {
        return Err(Error::Config(ferris_core::error::ConfigError::Invalid {
            key: "database.min_connections".to_string(),
            msg: format!(
                "{} exceeds database.max_connections ({})",
                config.min_connections, config.max_connections
            ),
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
