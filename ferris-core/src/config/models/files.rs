//! Object storage backends and upload limits.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use super::redact::Redacted;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesConfig {
    pub backend: StorageBackend,
    pub local_path: PathBuf,
    pub s3: Option<S3Config>,
    pub azure: Option<AzureConfig>,
    pub gcs: Option<GcsConfig>,
    pub cdn_url: Option<String>,
    pub max_file_size_mb: u64,
    pub allowed_extensions: Vec<String>,
    pub image_optimization: bool,
    pub image_quality: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Local,
    S3,
    Azure,
    Gcs,
    Memory,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &Redacted)
            .field("force_path_style", &self.force_path_style)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AzureConfig {
    pub account: String,
    pub container: String,
    pub access_key: String,
}

impl fmt::Debug for AzureConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureConfig")
            .field("account", &self.account)
            .field("container", &self.container)
            .field("access_key", &Redacted)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcsConfig {
    pub bucket: String,
    pub credentials_path: PathBuf,
}
