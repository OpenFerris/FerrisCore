//! Runtime services: redis, search, realtime, jobs and the scheduler.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::redact::Redacted;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    pub url: String,
    pub max_connections: u32,
    pub connection_timeout_secs: u64,
    pub command_timeout_secs: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub enabled: bool,
    pub host: String,
    pub api_key: Option<String>,
    pub index_prefix: String,
    pub batch_size: usize,
}

impl fmt::Debug for SearchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchConfig")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("api_key", &self.api_key.as_ref().map(|_| Redacted))
            .field("index_prefix", &self.index_prefix)
            .field("batch_size", &self.batch_size)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub ping_interval_secs: u64,
    pub ping_timeout_secs: u64,
    pub max_connections_per_worker: usize,
    pub redis_channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobsConfig {
    pub enabled: bool,
    pub default_queue: String,
    pub queues: Vec<QueueConfig>,
    pub worker_concurrency: usize,
    pub poll_interval_ms: u64,
    pub default_timeout_secs: u64,
    pub max_retries: u32,
    pub retry_backoff_base_secs: u64,
    pub dead_letter_queue: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    pub name: String,
    pub priority: i32,
    pub concurrency: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    pub enabled: bool,
    pub timezone: String,
    pub tick_interval_secs: u64,
    pub max_concurrent_jobs: usize,
    pub history_retention_days: u32,
}

/// `[retention]`: how long soft-deleted documents are kept and whether a legal
/// hold forbids destroying them.
///
/// `deleted_documents_days = 0` keeps tombstones forever (the default);
/// `legal_hold = true` blocks both explicit purges and automatic retention.
/// The engine reads the same two switches from the environment
/// (`FERRIS_RETENTION__…`); this type is the typed declaration the config root
/// will carry, kept next to the other runtime sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // not yet a field of `Config`; the engine reads the env spelling today
pub struct RetentionConfig {
    pub deleted_documents_days: u32,
    pub legal_hold: bool,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            deleted_documents_days: 0,
            legal_hold: false,
        }
    }
}
