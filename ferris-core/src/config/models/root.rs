//! The merged configuration root (`ferris.toml`).

use serde::{Deserialize, Serialize};

use super::apps::AppsConfig;
use super::auth::AuthConfig;
use super::database::DatabaseConfig;
use super::email::EmailConfig;
use super::features::{FeatureFlags, SecretsConfig};
use super::files::FilesConfig;
use super::observability::{LoggingConfig, TelemetryConfig};
use super::runtime::{
    JobsConfig, RealtimeConfig, RedisConfig, RetentionConfig, SchedulerConfig, SearchConfig,
};
use super::server::ServerConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub files: FilesConfig,
    pub email: EmailConfig,
    pub search: SearchConfig,
    pub realtime: RealtimeConfig,
    pub jobs: JobsConfig,
    pub scheduler: SchedulerConfig,
    /// Soft-delete retention and legal hold (`[retention]`). The engine also
    /// reads the `FERRIS_RETENTION__…` spelling for tests and tools; a server
    /// boot projects this typed section into the same policy.
    #[serde(default)]
    pub retention: RetentionConfig,
    pub logging: LoggingConfig,
    pub telemetry: TelemetryConfig,
    pub apps: AppsConfig,
    /// Install-wide feature switches (`[features]`). Missing sections default to
    /// "everything on", which is what a config written before this table existed
    /// means.
    #[serde(default)]
    pub features: FeatureFlags,
    /// Key material for recoverable `Secret` fields (H3). Absent means those
    /// fields can be read (masked) but not written or revealed, never a
    /// placeholder key.
    #[serde(default)]
    pub secrets: SecretsConfig,
}
