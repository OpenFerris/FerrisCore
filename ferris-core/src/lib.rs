//! Core types, traits and errors for Ferris — the bottom crate of the graph.
//!
//! # The one piece of I/O
//!
//! Everything here is pure: values, permissions, hooks, themes, the error
//! taxonomy. The single exception is reading the *bootstrap* files, which cannot
//! live anywhere else because every layer above needs them first:
//!
//! * [`Config::load`] / [`Config::load_from`] merge `ferris.toml`/`ferris.yaml`
//!   plus `FERRIS_*` env;
//! * [`TenantManager`] lists `<root>/<id>.json` tenant configs.
//!
//! Both are synchronous `std::fs`, so callers must not run them from an async
//! request path; the tenant registry caches parsed files by signature (mtime +
//! length) so a host lookup does not re-read every file.

pub mod audit;
pub mod config;
pub mod error;
pub mod hooks;
pub mod host_features;
pub mod i18n;
pub mod identity;
pub mod notifications;
pub mod password;
pub mod permissions;
pub mod plugins;
pub mod ports;
mod registry;
pub mod secret;
pub mod site;
pub mod tenant;
pub mod themes;
pub mod ui_extensions;
pub mod user;
pub mod value;
pub mod workspaces;

pub use config::{
    AppsConfig, AuthConfig, AzureConfig, Config, DatabaseConfig, DatabaseType, EmailAccountConfig,
    EmailConfig, FeatureFlags, FilesConfig, GcsConfig, JobsConfig, LogFileConfig, LogFormat,
    LogOutput, LoggingConfig, OAuthConfig, OAuthProvider, PkiConfig, QueueConfig, RealtimeConfig,
    RedisConfig, S3Config, SchedulerConfig, SearchConfig, SecretsConfig, ServerConfig, SslMode,
    StorageBackend, TelemetryConfig, TlsConfig, TlsMinVersion,
};
pub use error::{
    AuthError, AuthzError, ConfigError, DatabaseError, DocumentError, EmailError, Error, FileError,
    JobError, MigrationError, RealtimeError, Result, SchemaError, SearchError, SecretError,
    SerializationError, ValidationError, WorkflowError,
};
pub use hooks::{DocStatus, DocumentHook, Hook, HookContext, HookPoint, HookRegistry};
pub use i18n::{Catalog, I18n, Locale, negotiate};
pub use identity::ClientIdentity;
pub use permissions::{
    ParsePermissionError, Permission, PermissionEngine, PermissionRule, Resource,
    caller_principals, principals_for,
};
pub use site::{AppManifest, SiteConfig, SiteManager, is_valid_site_name};
pub use tenant::{SkippedTenant, TenantConfig, TenantLoad, TenantManager, WasmAppConfig};
pub use user::{CreateUserInput, UpdateUserInput, User, UserFilters, UserRole, UserType};
pub use value::{Map, Value};

// The `value!` macro is exported via #[macro_export] and available as
// crate::value!; users can import it with `use ferris_core::value;`.
