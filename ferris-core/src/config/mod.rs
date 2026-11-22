//! Configuration model and loading for Ferris.

mod load;
mod models;
#[cfg(test)]
mod tests;

pub use models::{
    AppsConfig, AuthConfig, AzureConfig, Config, DatabaseConfig, DatabaseType, EmailAccountConfig,
    EmailConfig, FeatureFlags, FilesConfig, GcsConfig, JobsConfig, LogFileConfig, LogFormat,
    LogOutput, LoggingConfig, OAuthConfig, OAuthProvider, PkiConfig, QueueConfig, RealtimeConfig,
    Redacted, RedisConfig, S3Config, SchedulerConfig, SearchConfig, SecretsConfig, ServerConfig,
    SslMode, StorageBackend, TelemetryConfig, TlsConfig, TlsMinVersion,
};
