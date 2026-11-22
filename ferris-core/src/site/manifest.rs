//! The global app catalogue and its manifest.
//!
//! `apps/<app>/` is the *global* scope: installed once, and the source every site
//! copies its `world/` from. Its `plugin.toml` is the only place the app's name,
//! version and configuration defaults are declared, so both installing it and
//! installing it into a site read the same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::plugins::is_valid_app_name;
use crate::{ConfigError, Error, FileError, Result, ValidationError};

/// The manifest every app package and every installed app must carry.
pub const PLUGIN_MANIFEST: &str = "plugin.toml";

/// The template world inside a package, copied into a site on install.
pub const TEMPLATE_WORLD: &str = "world";

/// What `plugin.toml` declares, with the paths resolved against the package.
///
/// ```toml
/// name = "facturacion"
/// version = "0.1.0"
///
/// [config]
/// currency = "EUR"
/// ```
#[derive(Debug, Clone)]
pub struct AppManifest {
    pub name: String,
    pub version: String,
    /// `<package>/<name>.wasm`.
    pub wasm: PathBuf,
    /// `<package>/world`.
    pub template_world: PathBuf,
    /// Defaults for the instance's `[config]` table.
    pub config_defaults: BTreeMap<String, serde_json::Value>,
}

/// The subset of `plugin.toml` this phase reads. Unknown keys are ignored on
/// purpose: a manifest may declare capabilities and routes that the host, not
/// this catalogue, consumes.
#[derive(Debug, Deserialize)]
struct ManifestFile {
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    config: BTreeMap<String, serde_json::Value>,
}

fn default_version() -> String {
    "0.0.0".to_string()
}

impl AppManifest {
    /// Read and validate the package at `dir`.
    ///
    /// Validation is all-or-nothing because every later step trusts the result:
    /// the name must be a safe path segment (it becomes `apps/<name>/`), the
    /// component `<name>.wasm` must exist, and `world/` must be a directory.
    ///
    /// # Errors
    ///
    /// [`Error::File`] when `plugin.toml`, the wasm or `world/` is missing,
    /// [`Error::Config`] when the manifest does not parse, and
    /// [`Error::Validation`] for an unusable name.
    pub fn read(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join(PLUGIN_MANIFEST);
        let text = std::fs::read_to_string(&manifest_path)?;
        let file: ManifestFile = toml::from_str(&text).map_err(|e| {
            Error::Config(ConfigError::Invalid {
                key: manifest_path.display().to_string(),
                msg: e.to_string(),
            })
        })?;
        if !is_valid_app_name(&file.name) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "invalid app name `{}` in {}: use letters, digits, `-` or `_`",
                file.name,
                manifest_path.display()
            ))));
        }
        let wasm = dir.join(format!("{}.wasm", file.name));
        if !wasm.is_file() {
            return Err(Error::File(FileError::NotFound(format!(
                "{} (declared by {})",
                wasm.display(),
                manifest_path.display()
            ))));
        }
        let template_world = dir.join(TEMPLATE_WORLD);
        if !template_world.is_dir() {
            return Err(Error::File(FileError::NotFound(format!(
                "{} (the app's world template)",
                template_world.display()
            ))));
        }
        Ok(Self {
            name: file.name,
            version: file.version,
            wasm,
            template_world,
            config_defaults: file.config,
        })
    }
}
