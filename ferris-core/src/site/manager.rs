//! Filesystem layout of `sites/` and the global `apps/` catalogue.
//!
//! ```text
//! sites/
//!   common_site_config.json      deployment-wide defaults
//!   <site>/
//!     site_config.json           the site's own config (0600: it has the password)
//!     apps.txt                   installed apps, one per line
//!     plugins/<app>/             the INSTANCE world the guest mounts at /world
//!     private/files/  public/files/
//! apps/
//!   <app>/
//!     <app>.wasm  world/  plugin.toml  assets/
//! ```
//!
//! The split is the whole point: `apps/<app>/world` is the **template** and is
//! never written by a running site; `sites/<site>/plugins/<app>` is the
//! **instance**, a copy of that template plus the site's own data, files and
//! secrets. Upgrading an app copies the template again and keeps the instance
//! data, which is why the two scopes cannot share a directory.
//!
//! Everything synchronous, like [`crate::TenantManager`]: this is bootstrap I/O,
//! so it must not run on an async request path.

use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::plugins::is_valid_app_name;
use crate::{Error, FileError, Result, ValidationError};

use super::config::{DEFAULT_REDIS_URL, SiteConfig};
use super::fsx::{create_private_dir, write_private};
use super::manifest::TEMPLATE_WORLD;

/// Per-site configuration file (holds the database password: `0600`).
pub const SITE_CONFIG_FILE: &str = "site_config.json";

/// Deployment-wide defaults shared by every site.
pub const COMMON_CONFIG_FILE: &str = "common_site_config.json";

/// The apps installed in a site, one name per line.
pub const APPS_FILE: &str = "apps.txt";

/// An instance's own configuration (`[ferris]` + `[config]`).
pub const CONFIG_FILE: &str = "config.toml";

/// Static assets shipped next to an app component.
pub const ASSETS_DIR: &str = "assets";

/// The app's own translation catalogues (`locales/<tag>.toml`).
///
/// The locales are declared in the WIT descriptor; the messages travel in the
/// package, next to `plugin.toml`, so a translator can update them without
/// rebuilding the component. `site install-app` copies the directory into the
/// instance, where the host merges it into the site's catalogue set at boot.
pub const LOCALES_DIR: &str = "locales";

/// A site's private root: files, the SQLite database and anything the site must
/// not publish. `0700`.
pub const PRIVATE_DIR: &str = "private";

/// Files neither the site nor its plugins may publish.
const PRIVATE_FILES: &str = "private/files";

/// Files the framework may serve.
const PUBLIC_FILES: &str = "public/files";

/// Whether `site` is a usable directory name under `sites/`.
///
/// Sites are more permissive than apps because Frappe site names carry dots
/// (`erp.example.com`). A leading dot, `..` or a separator is refused: the name
/// becomes a path component, and it is never sanitized, because a silent rewrite
/// would make two sites collide.
#[must_use]
pub fn is_valid_site_name(site: &str) -> bool {
    let name = site.trim();
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// The registry of sites and the app catalogue they install from.
pub struct SiteManager {
    pub(super) sites_root: PathBuf,
    pub(super) apps_root: PathBuf,
}

impl SiteManager {
    #[must_use]
    pub fn new(sites_root: impl Into<PathBuf>, apps_root: impl Into<PathBuf>) -> Self {
        Self {
            sites_root: sites_root.into(),
            apps_root: apps_root.into(),
        }
    }

    /// `FERRIS_SITES_DIR`/`FERRIS_APPS_DIR`, else `<project>/sites` and
    /// `<project>/apps` with the project from `FERRIS_PROJECT_DIR` or the cwd.
    ///
    /// The CLI runs with the project as its working directory, so this matches
    /// the documented layout without a second configuration source.
    fn env_roots() -> (PathBuf, PathBuf) {
        let project = std::env::var_os("FERRIS_PROJECT_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let sites = std::env::var_os("FERRIS_SITES_DIR")
            .map_or_else(|| project.join("sites"), PathBuf::from);
        let apps =
            std::env::var_os("FERRIS_APPS_DIR").map_or_else(|| project.join("apps"), PathBuf::from);
        (sites, apps)
    }

    #[must_use]
    pub fn sites_root(&self) -> &Path {
        &self.sites_root
    }

    #[must_use]
    pub fn apps_root(&self) -> &Path {
        &self.apps_root
    }

    /// `sites/<site>`.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when the name is not a safe path segment.
    pub fn site_dir(&self, site: &str) -> Result<PathBuf> {
        site_segment(&self.sites_root, site)
    }

    /// `sites/<site>/private`, the site's own (unpublished) root.
    ///
    /// # Errors
    ///
    /// An invalid site name.
    pub fn private_dir(&self, site: &str) -> Result<PathBuf> {
        Ok(self.site_dir(site)?.join(PRIVATE_DIR))
    }

    /// Create `sites/<site>/private` (idempotent, `0700`) and return it.
    ///
    /// `site new` needs it before the SQLite file is opened, and the resolver
    /// uses it to adopt a database left at the bench root by an older layout.
    ///
    /// # Errors
    ///
    /// An invalid site name or an I/O failure.
    pub fn ensure_private_dir(&self, site: &str) -> Result<PathBuf> {
        let dir = self.private_dir(site)?;
        create_private_dir(&dir)?;
        Ok(dir)
    }

    /// `sites/<site>/plugins`, the instance scope.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when the site name is not a safe path segment.
    pub fn plugins_root(&self, site: &str) -> Result<PathBuf> {
        Ok(self.site_dir(site)?.join("plugins"))
    }

    /// `sites/<site>/plugins/<app>`, the directory a guest mounts at `/world`.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when either name is not a safe path segment.
    pub fn instance_world(&self, site: &str, app: &str) -> Result<PathBuf> {
        let plugins = self.plugins_root(site)?;
        plugin_segment(&plugins, app)
    }

    /// `apps/<app>/world`, the template a site copies on install.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when the app name is not a safe path segment.
    pub fn template_world(&self, app: &str) -> Result<PathBuf> {
        Ok(self.global_app_dir(app)?.join(TEMPLATE_WORLD))
    }

    /// `apps/<app>`.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when the app name is not a safe path segment.
    pub fn global_app_dir(&self, app: &str) -> Result<PathBuf> {
        plugin_segment(&self.apps_root, app)
    }

    /// Create the bench's fixed directories and the shared site defaults.
    ///
    /// Idempotent: `ferris init` may run over a folder that already has part of
    /// the layout, and creating the first site calls the same helpers.
    ///
    /// # Errors
    ///
    /// An I/O failure under `apps/` or `sites/`.
    pub fn init_bench(&self) -> Result<()> {
        create_private_dir(&self.apps_root)?;
        create_private_dir(&self.sites_root)?;
        self.ensure_common_config()
    }

    /// Every loadable site, sorted by name.
    ///
    /// A directory without `site_config.json` is not a site (that is how
    /// `common_site_config.json` and a stray directory are skipped). A site whose
    /// file is corrupt is skipped here so one broken site cannot take the whole
    /// host down; [`SiteManager::get`] reports the parse error for that name.
    ///
    /// # Errors
    ///
    /// Only when the sites directory itself cannot be read.
    pub fn list(&self) -> Result<Vec<SiteConfig>> {
        if !self.sites_root.is_dir() {
            return Ok(Vec::new());
        }
        let mut sites = Vec::new();
        for entry in std::fs::read_dir(&self.sites_root)? {
            let entry = entry?;
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_valid_site_name(&name) {
                continue;
            }
            if let Ok(config) = self.get(&name) {
                sites.push(config);
            }
        }
        sites.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(sites)
    }

    /// Read `sites/<site>/site_config.json`.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe name, [`Error::File`] when the site or
    /// its config file does not exist, and the parse errors of
    /// [`SiteConfig::from_json`].
    pub fn get(&self, site: &str) -> Result<SiteConfig> {
        let path = self.site_dir(site)?.join(SITE_CONFIG_FILE);
        let text = std::fs::read_to_string(&path)?;
        SiteConfig::from_json(&text, site.trim())
    }

    /// Whether a site directory with a config file exists.
    #[must_use]
    pub fn exists(&self, site: &str) -> bool {
        self.site_dir(site)
            .is_ok_and(|dir| dir.join(SITE_CONFIG_FILE).is_file())
    }

    /// The enabled site whose `host_name` matches `host`.
    ///
    /// The comparison goes through [`crate::tenant::normalize_host`], so case, a
    /// port and a trailing dot all name the same site, and a malformed value
    /// matches nothing (fail closed).
    ///
    /// # Errors
    ///
    /// Propagates a failure to list the sites directory.
    pub fn resolve_host(&self, host: &str) -> Result<Option<SiteConfig>> {
        let Some(host) = crate::tenant::normalize_host(host) else {
            return Ok(None);
        };
        Ok(self.list()?.into_iter().find(|site| {
            site.enabled
                && crate::tenant::normalize_host(&site.host_name).as_deref() == Some(host.as_str())
        }))
    }

    /// Create the site layout and write its config.
    ///
    /// The layout is created in one go — the private and public files roots and
    /// the instance scope — because a site that exists but cannot serve files is
    /// a half-created site. An empty `files_path` becomes `<site>/private/files`
    /// and an empty `host_name` becomes the site name, so a freshly created site
    /// already resolves by its own name.
    ///
    /// # Errors
    ///
    /// The `Invalid` variant of [`Error::Validation`] for an unsafe name or a
    /// `host_name` another site already serves, and `Constraint` when the site
    /// already exists.
    pub fn create(&self, mut config: SiteConfig) -> Result<SiteConfig> {
        let dir = self.site_dir(&config.name)?;
        if dir.join(SITE_CONFIG_FILE).is_file() {
            return Err(already_exists(&config.name));
        }
        self.ensure_host_unique(&config)?;
        if config.files_path.as_os_str().is_empty() {
            config.files_path = dir.join(PRIVATE_FILES);
        }
        if config.host_name.trim().is_empty() {
            let name = config.name.clone();
            config.host_name = name;
        }
        config.created_at = Utc::now();
        config.updated_at = config.created_at;

        create_private_dir(&self.sites_root)?;
        create_private_dir(&dir)?;
        create_private_dir(&dir.join(PRIVATE_FILES))?;
        create_private_dir(&dir.join(PUBLIC_FILES))?;
        create_private_dir(&dir.join("plugins"))?;
        self.ensure_common_config()?;
        write_private(&dir.join(APPS_FILE), "")?;
        self.write_config(&config)?;
        Ok(config)
    }

    /// Overwrite an existing site's configuration.
    ///
    /// # Errors
    ///
    /// The `NotFound` variant of [`Error::File`] when the site does not exist
    /// (an update must not create one), plus the validation errors of
    /// [`SiteManager::create`].
    pub fn update(&self, config: &SiteConfig) -> Result<SiteConfig> {
        if !self.exists(&config.name) {
            return Err(not_found(&config.name));
        }
        self.ensure_host_unique(config)?;
        let mut config = config.clone();
        if config.host_name.trim().is_empty() {
            let name = config.name.clone();
            config.host_name = name;
        }
        config.updated_at = Utc::now();
        self.write_config(&config)?;
        Ok(config)
    }

    /// Delete a site and everything under it.
    ///
    /// # Errors
    ///
    /// The `NotFound` variant of [`Error::File`] when the site does not exist.
    pub fn delete(&self, site: &str) -> Result<()> {
        let dir = self.site_dir(site)?;
        if !dir.join(SITE_CONFIG_FILE).is_file() {
            return Err(not_found(site));
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Enable or disable a site without touching its data.
    ///
    /// # Errors
    ///
    /// [`SiteManager::get`] errors, and the write failure of
    /// [`SiteManager::update`].
    pub fn set_enabled(&self, site: &str, enabled: bool) -> Result<SiteConfig> {
        let mut config = self.get(site)?;
        config.enabled = enabled;
        self.update(&config)
    }

    fn write_config(&self, config: &SiteConfig) -> Result<()> {
        let dir = self.site_dir(&config.name)?;
        let path = dir.join(SITE_CONFIG_FILE);
        write_private(&path, &config.to_json()?)?;
        // The directory is created before the file on the fresh path; tightening
        // it again covers a site directory left at `0755` by an older build.
        super::fsx::restrict_dir(&dir)?;
        Ok(())
    }

    fn ensure_common_config(&self) -> Result<()> {
        let path = self.sites_root.join(COMMON_CONFIG_FILE);
        if path.is_file() {
            return Ok(());
        }
        let defaults = serde_json::json!({ "redis_url": DEFAULT_REDIS_URL });
        write_private(&path, &serde_json::to_string_pretty(&defaults)?)
    }

    /// A duplicate `host_name` would make [`resolve_host`](Self::resolve_host)
    /// answer with an arbitrary one of two sites, so it is refused.
    fn ensure_host_unique(&self, config: &SiteConfig) -> Result<()> {
        let Some(host) = crate::tenant::normalize_host(&config.host_name) else {
            return Ok(());
        };
        for other in self.list()? {
            if other.name == config.name {
                continue;
            }
            if crate::tenant::normalize_host(&other.host_name).as_deref() == Some(host.as_str()) {
                return Err(Error::Validation(ValidationError::Constraint(format!(
                    "host `{host}` is already served by site `{}`",
                    other.name
                ))));
            }
        }
        Ok(())
    }
}

impl Default for SiteManager {
    fn default() -> Self {
        let (sites, apps) = Self::env_roots();
        Self::new(sites, apps)
    }
}

fn site_segment(base: &Path, site: &str) -> Result<PathBuf> {
    if !is_valid_site_name(site) {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "invalid site name `{site}`: use letters, digits, `.`, `-` or `_`"
        ))));
    }
    contained(base, site.trim())
}

fn plugin_segment(base: &Path, app: &str) -> Result<PathBuf> {
    if !is_valid_app_name(app) {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "invalid app name `{app}`: use letters, digits, `-` or `_`"
        ))));
    }
    contained(base, app.trim())
}

/// Join a validated single component and prove the result stays inside `base`.
///
/// The charset check already forbids separators; the containment check is the
/// same second line of defence as [`crate::plugins::world_path`], so a future
/// caller cannot bypass the policy by relaxing a name validator.
fn contained(base: &Path, name: &str) -> Result<PathBuf> {
    crate::plugins::world_path(base, Path::new(name))
}

fn not_found(site: &str) -> Error {
    Error::File(FileError::NotFound(format!("site `{site}` does not exist")))
}

fn already_exists(site: &str) -> Error {
    Error::Validation(ValidationError::Constraint(format!(
        "site `{site}` already exists"
    )))
}
