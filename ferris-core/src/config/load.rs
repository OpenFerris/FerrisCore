use super::models::{
    AppsConfig, AuthConfig, Config, DatabaseConfig, DatabaseType, EmailConfig, FeatureFlags,
    FilesConfig, JobsConfig, LogFormat, LogOutput, LoggingConfig, OAuthConfig, QueueConfig,
    RealtimeConfig, RedisConfig, SchedulerConfig, SearchConfig, SecretsConfig, ServerConfig,
    SslMode, StorageBackend, TelemetryConfig, default_roles,
};
use crate::error::{ConfigError, Result};
use figment::Figment;
use figment::providers::{Env, Format, Toml, Yaml};
use std::path::PathBuf;

impl Config {
    /// Load and merge the configuration for this process's config directory.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::config_dir()?)
    }

    /// Load and merge the configuration files in `dir`.
    ///
    /// Precedence, lowest to highest: `ferris.toml`, then `ferris.yaml`, then
    /// the `FERRIS_*` environment (with `__` as the nesting separator).
    /// Figment's later `merge` wins. A **missing** file yields an empty
    /// provider rather than an error, which is why an absent directory is
    /// reported as [`ConfigError::NotFound`] here instead of surfacing as a
    /// wall of `extract()`'s "missing field" messages.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] when `dir` does not exist, and
    /// [`ConfigError::Parse`] when the merged providers do not describe a valid
    /// configuration.
    pub fn load_from(dir: &std::path::Path) -> Result<Self> {
        if !dir.is_dir() {
            return Err(
                ConfigError::NotFound(format!("config directory: {}", dir.display())).into(),
            );
        }
        Self::figment_for(dir)
            .extract()
            .map_err(|e| ConfigError::Parse(e).into())
    }

    /// The merged provider tree for `dir`, in precedence order. Split out so a
    /// test can assert the order on the merged providers themselves.
    ///
    /// The file providers are **not** `nested()`: that figment modifier reads a
    /// document's top-level tables as *profiles*, so the `[database]` / `[server]`
    /// layout this project documents (and `ferris new` scaffolds) would land under
    /// a profile called `database` and every field would look missing. Ferris has
    /// one flat configuration file, not per-profile sections.
    pub(super) fn figment_for(dir: &std::path::Path) -> Figment {
        Figment::new()
            .merge(Toml::file(dir.join("ferris.toml")))
            .merge(Yaml::file(dir.join("ferris.yaml")))
            .merge(Env::prefixed("FERRIS_").split("__"))
    }

    pub fn config_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("FERRIS_CONFIG_DIR") {
            return Ok(PathBuf::from(dir));
        }

        dirs::config_dir()
            .map(|p| p.join("ferris"))
            .ok_or_else(|| ConfigError::NotFound("config directory".into()).into())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
            redis: RedisConfig::default(),
            server: ServerConfig::default(),
            auth: AuthConfig::default(),
            files: FilesConfig::default(),
            email: EmailConfig::default(),
            search: SearchConfig::default(),
            realtime: RealtimeConfig::default(),
            jobs: JobsConfig::default(),
            scheduler: SchedulerConfig::default(),
            retention: super::models::RetentionConfig::default(),
            logging: LoggingConfig::default(),
            telemetry: TelemetryConfig::default(),
            apps: AppsConfig::default(),
            features: FeatureFlags::default(),
            secrets: SecretsConfig::default(),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            r#type: DatabaseType::Postgres,
            host: "localhost".into(),
            port: 5432,
            name: "ferris".into(),
            user: "ferris".into(),
            password: "ferris".into(),
            max_connections: 20,
            min_connections: 5,
            connect_timeout_secs: 10,
            idle_timeout_secs: 600,
            max_lifetime_secs: 1800,
            ssl_mode: SslMode::Prefer,
            schema: Some("public".into()),
        }
    }
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".into(),
            max_connections: 20,
            connection_timeout_secs: 5,
            command_timeout_secs: 30,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8000,
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            request_timeout_secs: 60,
            body_limit_mb: 50,
            enable_cors: true,
            cors_origins: vec!["*".into()],
            trusted_proxies: vec![],
            tls: None,
            pki: None,
            external_url: None,
            roles: default_roles(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt_secret: None,
            jwt_expiry_hours: 24,
            refresh_expiry_days: 30,
            password_min_length: 8,
            password_require_uppercase: true,
            password_require_lowercase: true,
            password_require_numbers: true,
            password_require_symbols: false,
            max_login_attempts: 5,
            lockout_duration_minutes: 15,
            mfa_enabled: false,
            mfa_issuer: "Ferris".into(),
            session_timeout_minutes: 60,
            max_sessions_per_user: 5,
            oauth: OAuthConfig::default(),
        }
    }
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self { providers: vec![] }
    }
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::Local,
            local_path: PathBuf::from("./files"),
            s3: None,
            azure: None,
            gcs: None,
            cdn_url: None,
            max_file_size_mb: 50,
            allowed_extensions: vec![],
            image_optimization: true,
            image_quality: 85,
        }
    }
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            default_account: "default".into(),
            accounts: vec![],
            queue_concurrency: 10,
            retry_attempts: 3,
            retry_delay_secs: 60,
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "http://localhost:7700".into(),
            api_key: None,
            index_prefix: "ferris".into(),
            batch_size: 1000,
        }
    }
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "0.0.0.0".into(),
            port: 8001,
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            ping_interval_secs: 25,
            ping_timeout_secs: 60,
            max_connections_per_worker: 10000,
            redis_channel: "ferris:realtime".into(),
        }
    }
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_queue: "default".into(),
            queues: vec![
                QueueConfig {
                    name: "default".into(),
                    priority: 0,
                    concurrency: 10,
                    timeout_secs: 300,
                },
                QueueConfig {
                    name: "short".into(),
                    priority: 10,
                    concurrency: 20,
                    timeout_secs: 30,
                },
                QueueConfig {
                    name: "long".into(),
                    priority: -10,
                    concurrency: 5,
                    timeout_secs: 3600,
                },
            ],
            worker_concurrency: 10,
            poll_interval_ms: 1000,
            default_timeout_secs: 300,
            max_retries: 3,
            retry_backoff_base_secs: 60,
            dead_letter_queue: "dead_letter".into(),
        }
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timezone: "UTC".into(),
            tick_interval_secs: 60,
            max_concurrent_jobs: 100,
            history_retention_days: 30,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Json,
            output: LogOutput::Stdout,
            file: None,
        }
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            otlp_endpoint: "http://localhost:4317".into(),
            service_name: "ferris".into(),
            sample_rate: 0.1,
        }
    }
}

impl Default for AppsConfig {
    fn default() -> Self {
        Self {
            enabled: vec![],
            // `ferris build` and `ferris deploy` both put the component here
            // (`plugins/<package>.wasm`), so the server reads the same directory
            // the CLI writes instead of a name nobody produced.
            wasm_dir: PathBuf::from("./plugins"),
            plugin_tls_port: None,
            plugin_tls_ports: std::collections::BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::models::EmailAccountConfig;

    /// `[apps]` configures the app directories the host loads; it is not a
    /// capability grant (declaring a capability is the grant).

    /// SRV9 regression: existing account configs that never mention STARTTLS
    /// keep working (both switches default to `false`), and the switches are
    /// actually deserialized when present.
    #[test]
    fn email_starttls_switches_default_to_false_and_are_read_when_present() {
        let legacy = serde_json::json!({
            "name": "mail",
            "email": "noreply@example.com",
            "smtp_host": "smtp.example.com",
            "smtp_port": 465,
            "smtp_username": "noreply",
            "smtp_password": "secret",
            "smtp_tls": true,
            "imap_tls": true,
        });
        let account: EmailAccountConfig =
            serde_json::from_value(legacy).expect("legacy account config parses");
        assert!(!account.smtp_starttls);
        assert!(!account.imap_starttls);

        let upgraded = serde_json::json!({
            "name": "mail",
            "email": "noreply@example.com",
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "smtp_username": "noreply",
            "smtp_password": "secret",
            "smtp_tls": false,
            "smtp_starttls": true,
            "imap_tls": false,
            "imap_starttls": true,
        });
        let account: EmailAccountConfig =
            serde_json::from_value(upgraded).expect("STARTTLS account config parses");
        assert!(account.smtp_starttls);
        assert!(account.imap_starttls);
    }
}
