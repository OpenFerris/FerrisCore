//! The persisted form of a site.
//!
//! A site is Frappe's unit of deployment: one directory under `sites/` holding
//! its own database credentials, files root and plugin instances. Its
//! `site_config.json` is edited by operators and read by `ferris serve --site`
//! before any engine type exists, so the file keeps the **flat** Frappe keys
//! (`db_name`, `db_type`, `db_host`, `db_port`, `db_user`, `db_password`,
//! `db_ssl_mode`) next to the Ferris ones. The nested [`DatabaseConfig`] is
//! rebuilt on load from those keys; the tunables it also carries (pool sizes,
//! timeouts) are not part of the Frappe shape and stay at their defaults.
//!
//! Every field of the wire form is `#[serde(default)]`: a file written by
//! Frappe, by a smaller earlier build or by hand must still load, and a missing
//! key must produce the documented default instead of a parse error.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{DatabaseConfig, DatabaseType, SslMode};
use crate::{ConfigError, Error, Result, ValidationError};

use super::is_valid_site_name;

const fn default_enabled() -> bool {
    true
}

/// The Redis a new site points at, and the value written to
/// `common_site_config.json` so a deployment can override it once.
pub(super) const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

/// The locale a site serves when it names none, and the one locale the binary
/// is guaranteed to have a catalogue for.
pub const DEFAULT_LOCALE: &str = "en";

fn default_locale() -> String {
    DEFAULT_LOCALE.to_string()
}

fn default_supported_locales() -> Vec<String> {
    // Both catalogues the binary embeds. A site with no `[i18n]` policy is
    // still useful in Spanish; the HTML falls back to English for any missing
    // key, so advertising both costs nothing.
    vec![DEFAULT_LOCALE.to_string(), "es".to_string()]
}

/// Trim, drop blanks and duplicates, and never return an empty policy.
fn normalize_locales(locales: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::with_capacity(locales.len());
    for locale in locales {
        let locale = locale.trim();
        if !locale.is_empty() && !normalized.iter().any(|seen| seen == locale) {
            normalized.push(locale.to_string());
        }
    }
    if normalized.is_empty() {
        default_supported_locales()
    } else {
        normalized
    }
}

/// A site's configuration, as `sites/<site>/site_config.json` stores it.
#[derive(Debug, Clone)]
pub struct SiteConfig {
    /// Directory name under `sites/`, and the name `--site` takes.
    pub name: String,
    /// The `Host` this site answers for. Empty means "same as `name`".
    pub host_name: String,
    /// Whether the site is served. A disabled site keeps its files and data.
    pub enabled: bool,
    pub database: DatabaseConfig,
    pub redis_url: String,
    /// The files root for this site; empty means `<site>/private/files`.
    pub files_path: PathBuf,
    /// Apps installed in this site, mirroring `apps.txt`.
    pub apps: Vec<String>,
    /// Per-site overrides for an app's `[config]` defaults, keyed by app name.
    pub plugins: BTreeMap<String, serde_json::Value>,
    /// Version of each installed app, captured from the catalogue at the
    /// moment its component was copied into this site.
    ///
    /// The instance owns a real copy of the wasm, so this map is the record of
    /// what that copy is: replacing `apps/<app>/<app>.wasm` does not change it,
    /// and `install_app(..., force = true)` is the only writer.
    pub app_versions: BTreeMap<String, String>,
    /// Locale served when the client sends no usable `Accept-Language`.
    pub default_locale: String,
    /// Locales this site advertises through `GET /api/i18n`.
    pub supported_locales: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SiteConfig {
    /// A site with the documented defaults: Postgres on localhost, `db_name`
    /// derived from the site name, the shared Redis, no app installed.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let now = Utc::now();
        Self {
            host_name: name.clone(),
            database: default_database(&name),
            name,
            enabled: true,
            redis_url: DEFAULT_REDIS_URL.to_string(),
            files_path: PathBuf::new(),
            apps: Vec::new(),
            plugins: BTreeMap::new(),
            app_versions: BTreeMap::new(),
            default_locale: DEFAULT_LOCALE.to_string(),
            supported_locales: default_supported_locales(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Parse a `site_config.json`.
    ///
    /// `name_hint` is the directory name; it fills `name` when the file omits it
    /// (the common case for a file copied from another site).
    ///
    /// # Errors
    ///
    /// [`Error::Serialization`] when the JSON does not parse, and
    /// [`Error::Config`] when a flat key holds a value this build does not
    /// implement (an unknown `db_type` or `db_ssl_mode`) — reported instead of
    /// silently falling back to Postgres.
    pub fn from_json(json: &str, name_hint: &str) -> Result<Self> {
        let file: SiteFile = serde_json::from_str(json)?;
        let name = if file.name.trim().is_empty() {
            name_hint.to_string()
        } else {
            file.name
        };
        if !is_valid_site_name(&name) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "invalid site name `{name}`: use letters, digits, `.`, `-` or `_`"
            ))));
        }
        let r#type = parse_db_type(&file.db_type)?;
        let db_name = if file.db_name.trim().is_empty() {
            database_name(&name)
        } else {
            file.db_name
        };
        let database = DatabaseConfig {
            r#type,
            host: non_empty(file.db_host, "127.0.0.1"),
            port: if file.db_port == 0 {
                default_port(r#type)
            } else {
                file.db_port
            },
            name: db_name.clone(),
            user: non_empty(file.db_user, &db_name),
            password: file.db_password,
            max_connections: 10,
            min_connections: 2,
            connect_timeout_secs: 10,
            idle_timeout_secs: 300,
            max_lifetime_secs: 1800,
            ssl_mode: parse_ssl_mode(&file.db_ssl_mode)?,
            schema: None,
        };
        Ok(Self {
            host_name: if file.host_name.trim().is_empty() {
                name.clone()
            } else {
                file.host_name
            },
            name,
            enabled: file.enabled,
            database,
            redis_url: non_empty(file.redis_url, DEFAULT_REDIS_URL),
            files_path: PathBuf::from(file.files_path),
            apps: file.apps,
            plugins: file.plugins,
            app_versions: file.app_versions,
            default_locale: non_empty(file.default_locale, DEFAULT_LOCALE),
            supported_locales: normalize_locales(file.supported_locales),
            created_at: file.created_at.unwrap_or_else(Utc::now),
            updated_at: file.updated_at.unwrap_or_else(Utc::now),
        })
    }

    /// Render the JSON written to `site_config.json`.
    ///
    /// # Errors
    ///
    /// [`Error::Serialization`] only when a `plugins` override cannot be
    /// encoded (it is already JSON, so this is a defect, not user input).
    pub fn to_json(&self) -> Result<String> {
        let file = SiteFile {
            name: self.name.clone(),
            host_name: self.host_name.clone(),
            enabled: self.enabled,
            redis_url: self.redis_url.clone(),
            files_path: self.files_path.to_string_lossy().into_owned(),
            apps: self.apps.clone(),
            plugins: self.plugins.clone(),
            app_versions: self.app_versions.clone(),
            default_locale: self.default_locale.clone(),
            supported_locales: self.supported_locales.clone(),
            created_at: Some(self.created_at),
            updated_at: Some(self.updated_at),
            db_name: self.database.name.clone(),
            db_type: db_type_str(self.database.r#type).to_string(),
            db_host: self.database.host.clone(),
            db_port: self.database.port,
            db_user: self.database.user.clone(),
            db_password: self.database.password.clone(),
            db_ssl_mode: ssl_mode_str(self.database.ssl_mode).to_string(),
        };
        Ok(serde_json::to_string_pretty(&file)?)
    }
}

/// The on-disk shape: Ferris keys plus Frappe's flat database keys.
#[derive(Debug, Serialize, Deserialize)]
struct SiteFile {
    // `name` is kept so a file can move with its directory; the directory name
    // is authoritative and fills it when the key is absent.
    #[serde(default)]
    name: String,
    #[serde(default)]
    host_name: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    redis_url: String,
    #[serde(default)]
    files_path: String,
    #[serde(default)]
    apps: Vec<String>,
    #[serde(default)]
    plugins: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    app_versions: BTreeMap<String, String>,
    #[serde(default = "default_locale")]
    default_locale: String,
    #[serde(default = "default_supported_locales")]
    supported_locales: Vec<String>,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
    // Frappe-compatible flat database keys.
    #[serde(default)]
    db_name: String,
    #[serde(default)]
    db_type: String,
    #[serde(default)]
    db_host: String,
    #[serde(default)]
    db_port: u16,
    #[serde(default)]
    db_user: String,
    #[serde(default)]
    db_password: String,
    #[serde(default)]
    db_ssl_mode: String,
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

/// The database and user a new site gets: the site name with the characters a
/// SQL identifier refuses replaced.
fn database_name(site: &str) -> String {
    site.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

const fn default_port(r#type: DatabaseType) -> u16 {
    match r#type {
        DatabaseType::Postgres => 5432,
        DatabaseType::MySQL => 3306,
        DatabaseType::SQLite => 0,
    }
}

fn default_database(site: &str) -> DatabaseConfig {
    let name = database_name(site);
    DatabaseConfig {
        r#type: DatabaseType::Postgres,
        host: "127.0.0.1".to_string(),
        port: 5432,
        name: name.clone(),
        user: name,
        password: String::new(),
        max_connections: 10,
        min_connections: 2,
        connect_timeout_secs: 10,
        idle_timeout_secs: 300,
        max_lifetime_secs: 1800,
        ssl_mode: SslMode::Prefer,
        schema: None,
    }
}

const fn db_type_str(r#type: DatabaseType) -> &'static str {
    match r#type {
        DatabaseType::Postgres => "postgres",
        DatabaseType::MySQL => "mysql",
        DatabaseType::SQLite => "sqlite",
    }
}

/// Parse `db_type`, accepting Frappe's spellings (`mariadb`) next to ours.
///
/// An empty value is a file written by this framework before the key existed,
/// so it defaults to Postgres; anything else unknown is refused.
fn parse_db_type(raw: &str) -> Result<DatabaseType> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "" | "postgres" | "postgresql" => Ok(DatabaseType::Postgres),
        "mysql" | "mariadb" => Ok(DatabaseType::MySQL),
        "sqlite" | "sqlite3" => Ok(DatabaseType::SQLite),
        other => Err(Error::Config(ConfigError::Invalid {
            key: "db_type".to_string(),
            msg: format!("unknown database type `{other}`"),
        })),
    }
}

const fn ssl_mode_str(mode: SslMode) -> &'static str {
    match mode {
        SslMode::Disable => "disable",
        SslMode::Allow => "allow",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    }
}

fn parse_ssl_mode(raw: &str) -> Result<SslMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "prefer" => Ok(SslMode::Prefer),
        "disable" => Ok(SslMode::Disable),
        "allow" => Ok(SslMode::Allow),
        "require" => Ok(SslMode::Require),
        "verify-ca" => Ok(SslMode::VerifyCa),
        "verify-full" => Ok(SslMode::VerifyFull),
        other => Err(Error::Config(ConfigError::Invalid {
            key: "db_ssl_mode".to_string(),
            msg: format!("unknown ssl mode `{other}`"),
        })),
    }
}
