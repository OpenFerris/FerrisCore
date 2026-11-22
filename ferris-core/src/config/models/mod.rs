//! Configuration model: the merged `ferris.toml` and one section per domain.
//!
//! Every secret-bearing struct keeps a hand-written `Debug` impl so `{:?}`,
//! `dbg!` and test failure messages can never print a credential (§B4); the
//! marker that makes that mechanical is [`redact`]. Sections are declared next
//! to their own types and re-exported here, so `crate::config::X` and
//! `crate::config::models::X` keep resolving to the same item.

mod apps;
mod auth;
mod database;
mod email;
mod features;
mod files;
mod observability;
mod redact;
mod root;
mod runtime;
mod server;

pub use apps::AppsConfig;
pub use auth::{AuthConfig, OAuthConfig, OAuthProvider};
pub use database::{DatabaseConfig, DatabaseType, SslMode};
pub use email::{EmailAccountConfig, EmailConfig};
pub use features::{FeatureFlags, SecretsConfig};
pub use files::{AzureConfig, FilesConfig, GcsConfig, S3Config, StorageBackend};
pub use observability::{LogFileConfig, LogFormat, LogOutput, LoggingConfig, TelemetryConfig};
pub use redact::Redacted;
pub use root::Config;
pub use runtime::{
    JobsConfig, QueueConfig, RealtimeConfig, RedisConfig, RetentionConfig, SchedulerConfig,
    SearchConfig,
};
pub use server::{PkiConfig, ServerConfig, TlsConfig, TlsMinVersion};

pub(super) use server::default_roles;
