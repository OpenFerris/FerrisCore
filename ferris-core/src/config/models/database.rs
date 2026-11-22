//! Database connection settings.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::redact::Redacted;

#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub r#type: DatabaseType,
    pub host: String,
    pub port: u16,
    pub name: String,
    pub user: String,
    pub password: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u64,
    pub idle_timeout_secs: u64,
    pub max_lifetime_secs: u64,
    pub ssl_mode: SslMode,
    pub schema: Option<String>,
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("type", &self.r#type)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("name", &self.name)
            .field("user", &self.user)
            .field("password", &Redacted)
            .field("max_connections", &self.max_connections)
            .field("min_connections", &self.min_connections)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("idle_timeout_secs", &self.idle_timeout_secs)
            .field("max_lifetime_secs", &self.max_lifetime_secs)
            .field("ssl_mode", &self.ssl_mode)
            .field("schema", &self.schema)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    Postgres,
    MySQL,
    SQLite,
}

/// TLS verification mode for the database connection.
///
/// The spellings are the ones libpq and MySQL document: `verify-ca` and
/// `verify-full`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    #[serde(rename = "verify-ca")]
    VerifyCa,
    #[serde(rename = "verify-full")]
    VerifyFull,
}

impl DatabaseConfig {
    /// Build a connection URL with the **user, password and database name
    /// percent-encoded**, so credentials containing `@`, `:`, `/`, `?`, `#`,
    /// … cannot break out of the URL or inject connection parameters.
    ///
    /// Produces the raw scheme+credentials+host URL for Postgres/MySQL (SSL
    /// query parameters are appended by the caller, see `ferris-db`) and the
    /// sqlx special form for in-memory SQLite.
    pub fn connection_url(&self) -> String {
        match self.r#type {
            DatabaseType::Postgres => format!(
                "postgres://{}:{}@{}:{}/{}",
                percent_encode(&self.user),
                percent_encode(&self.password),
                self.host,
                self.port,
                percent_encode(&self.name)
            ),
            DatabaseType::MySQL => format!(
                "mysql://{}:{}@{}:{}/{}",
                percent_encode(&self.user),
                percent_encode(&self.password),
                self.host,
                self.port,
                percent_encode(&self.name)
            ),
            DatabaseType::SQLite => {
                if self.name.is_empty() || self.name == ":memory:" {
                    "sqlite::memory:".to_string()
                } else if self.name.contains("://") || self.name.starts_with("file:") {
                    // Already a URL (`sqlite://…`, `file:…`): use it unchanged
                    // instead of producing `sqlite://sqlite://…`.
                    self.name.clone()
                } else {
                    format!("sqlite://{}", self.name)
                }
            }
        }
    }
}

/// Percent-encode a single URL component (RFC 3986). Kept private to the crate
/// so every URL builder shares one implementation; unreserved characters
/// (`A-Za-z0-9-._~`) pass through untouched.
///
/// The hex digits come from a table and are pushed straight into the buffer:
/// the previous `push_str(&format!("%{b:02X}"))` allocated a `String` per
/// escaped byte.
fn percent_encode(component: &str) -> String {
    const HEX: [u8; 16] = *b"0123456789ABCDEF";
    let mut out = String::with_capacity(component.len());
    for b in component.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => {
                // Both indices are nibbles, so they are provably < 16.
                out.push('%');
                out.push(HEX[usize::from(b >> 4)] as char);
                out.push(HEX[usize::from(b & 0x0F)] as char);
            }
        }
    }
    out
}
