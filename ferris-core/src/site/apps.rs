//! Installing an app into the global catalogue and into a site.
//!
//! Two scopes, two operations. [`SiteManager::install_global_app`] copies a
//! package into `apps/<app>/`; [`SiteManager::install_app`] copies that
//! catalogue's `world/` template into `sites/<site>/plugins/<app>/` and keeps
//! whatever the site already has. The rules that make an upgrade safe live in
//! `copy_world_template`: secrets are never copied, and the site's `data/` and
//! `files/` are only written on a first install.

use std::path::Path;

use crate::plugins::{WORLD_DIRS, is_valid_app_name};
use crate::{ConfigError, Error, Result, ValidationError};

use super::config::SiteConfig;
use super::fsx::{
    copy_tree, create_private_dir, relative_path, symlink_file, write_private, write_private_atomic,
};
use super::manager::{APPS_FILE, ASSETS_DIR, CONFIG_FILE, SiteManager};
use super::manifest::{AppManifest, PLUGIN_MANIFEST, TEMPLATE_WORLD};

impl SiteManager {
    /// The apps present in the global catalogue, sorted.
    ///
    /// # Errors
    ///
    /// Only when the apps directory cannot be read.
    pub fn global_apps(&self) -> Result<Vec<String>> {
        if !self.apps_root.is_dir() {
            return Ok(Vec::new());
        }
        let mut apps = Vec::new();
        for entry in std::fs::read_dir(&self.apps_root)? {
            let entry = entry?;
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_valid_app_name(&name) && entry.path().join(PLUGIN_MANIFEST).is_file() {
                apps.push(name);
            }
        }
        apps.sort();
        Ok(apps)
    }

    /// The manifest of an installed global app.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe name, plus the errors of
    /// [`AppManifest::read`] when the app is not installed.
    pub fn app_manifest(&self, app: &str) -> Result<AppManifest> {
        AppManifest::read(&self.global_app_dir(app)?)
    }

    /// Install (or re-install) an app package into the global catalogue.
    ///
    /// The package is validated first, then copied: the component, `plugin.toml`,
    /// the `world/` template (whose `secrets/` is created empty — those belong to
    /// a site) and `assets/` when present. Only regular files are copied, so a
    /// symlink inside the package cannot pull in a file from outside it.
    ///
    /// Re-installing replaces the template in place. Site instances are copies
    /// and are not touched; only `install_app` upgrades them.
    ///
    /// # Errors
    ///
    /// The validation errors of [`AppManifest::read`], plus any I/O failure.
    pub fn install_global_app(&self, package: &Path) -> Result<AppManifest> {
        let source = AppManifest::read(package)?;
        let dest = self.global_app_dir(&source.name)?;
        create_private_dir(&dest)?;

        let wasm = dest.join(format!("{}.wasm", source.name));
        std::fs::copy(&source.wasm, &wasm)?;
        std::fs::copy(package.join(PLUGIN_MANIFEST), dest.join(PLUGIN_MANIFEST))?;

        let world = dest.join(TEMPLATE_WORLD);
        if world.is_dir() {
            std::fs::remove_dir_all(&world)?;
        }
        copy_world_template(&source.template_world, &world, false)?;

        let assets = dest.join(ASSETS_DIR);
        let package_assets = package.join(ASSETS_DIR);
        if package_assets.is_dir() {
            if assets.is_dir() {
                std::fs::remove_dir_all(&assets)?;
            }
            copy_tree(&package_assets, &assets)?;
        } else {
            create_private_dir(&assets)?;
        }

        Ok(AppManifest {
            name: source.name,
            version: source.version,
            wasm,
            template_world: world,
            config_defaults: source.config_defaults,
        })
    }

    /// Install `app` into `site`: copy the template `world/` to the instance
    /// scope, write `config.toml`, link the component and record the app in
    /// `apps.txt`.
    ///
    /// The instance is what the guest mounts, so it is the only copy that gets
    /// written. Its `secrets/` is always created empty and its `data/` and
    /// `files/` are never overwritten once they exist: they are the site's, and
    /// an upgrade must not destroy them. `config.toml` is merged rather than
    /// replaced, so a value the operator edited for this site survives an
    /// upgrade while new defaults still arrive.
    ///
    /// # Errors
    ///
    /// The `NotFound` variant of [`Error::File`] when the site or the global app
    /// is missing, `Constraint` of [`Error::Validation`] when the app is already
    /// installed and `force` is false, and any I/O failure.
    pub fn install_app(&self, site: &str, app: &str, force: bool) -> Result<AppManifest> {
        let site_config = self.get(site)?;
        let instance = self.instance_world(site, app)?;
        let manifest = self.app_manifest(app)?;
        let existed = instance.is_dir();
        if existed && !force {
            return Err(Error::Validation(ValidationError::Constraint(format!(
                "app `{app}` is already installed in site `{site}`; pass force to upgrade it"
            ))));
        }

        create_private_dir(&instance)?;
        copy_world_template(&manifest.template_world, &instance, existed)?;

        let config_path = instance.join(CONFIG_FILE);
        let previous = if existed && config_path.is_file() {
            Some(std::fs::read_to_string(&config_path)?)
        } else {
            None
        };
        let overrides = site_config.plugins.get(app);
        let text = render_instance_config(site, &manifest, overrides, previous.as_deref())?;
        write_private(&config_path, &text)?;

        link_component(&manifest.wasm, &instance, app)?;

        let mut apps = self.read_apps(site)?;
        if !apps.iter().any(|name| name == app) {
            apps.push(app.to_string());
        }
        self.write_apps(site, &apps)?;
        self.sync_site_apps(&site_config, apps)?;
        Ok(manifest)
    }

    /// Remove `app` from `site`.
    ///
    /// The app always leaves `apps.txt`, so the next boot stops loading it.
    /// `purge` also deletes the instance directory (its data, files and
    /// secrets); without it the directory stays for a later reinstall.
    ///
    /// # Errors
    ///
    /// The `NotFound` variant of [`Error::File`] when the site does not exist,
    /// and any I/O failure. Uninstalling an app that is not listed is not an
    /// error.
    pub fn uninstall_app(&self, site: &str, app: &str, purge: bool) -> Result<()> {
        let site_config = self.get(site)?;
        let instance = self.instance_world(site, app)?;
        let apps: Vec<String> = self
            .read_apps(site)?
            .into_iter()
            .filter(|name| name != app)
            .collect();
        self.write_apps(site, &apps)?;
        self.sync_site_apps(&site_config, apps)?;
        if purge && instance.is_dir() {
            std::fs::remove_dir_all(&instance)?;
        }
        Ok(())
    }

    /// The apps installed in `site`, in install order.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when a line is not a usable app name (a corrupt
    /// `apps.txt` must be fixed, not silently half-read).
    pub fn installed_apps(&self, site: &str) -> Result<Vec<String>> {
        self.read_apps(site)
    }

    fn read_apps(&self, site: &str) -> Result<Vec<String>> {
        let path = self.site_dir(site)?.join(APPS_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut apps: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if !is_valid_app_name(line) {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "{}: `{line}` is not a usable app name",
                    path.display()
                ))));
            }
            if !apps.iter().any(|name| name == line) {
                apps.push(line.to_string());
            }
        }
        Ok(apps)
    }

    fn write_apps(&self, site: &str, apps: &[String]) -> Result<()> {
        let path = self.site_dir(site)?.join(APPS_FILE);
        let mut contents = String::new();
        for app in apps {
            contents.push_str(app);
            contents.push('\n');
        }
        write_private_atomic(&path, &contents)
    }

    /// Keep `site_config.json`'s `apps` in step with `apps.txt`, the file the
    /// boot path reads.
    fn sync_site_apps(&self, config: &SiteConfig, apps: Vec<String>) -> Result<()> {
        if config.apps == apps {
            return Ok(());
        }
        let mut config = config.clone();
        config.apps = apps;
        self.update(&config)?;
        Ok(())
    }
}

/// Copy a world template into a destination, applying the two rules that make an
/// upgrade safe.
///
/// * `secrets/` content is never copied — the template has none and the site's
///   own secrets must not be replaced. The directory is created empty.
/// * when `keep_instance_data` is set, `data/` and `files/` are left untouched;
///   `customizations/` is replaced because it *is* the template's output.
fn copy_world_template(src: &Path, dst: &Path, keep_instance_data: bool) -> Result<()> {
    create_private_dir(dst)?;
    for dir in WORLD_DIRS {
        create_private_dir(&dst.join(dir))?;
    }
    let mut entries = std::fs::read_dir(src)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if entry.file_type()?.is_symlink() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let target = dst.join(&name);
        match name.as_str() {
            // The instance's secrets are its own; the template never ships any.
            "secrets" => {}
            // An upgrade must not touch the site's data and blobs.
            "data" | "files" if keep_instance_data => {}
            // Customizations are the template's output, so they are replaced.
            "customizations" => {
                if target.is_dir() {
                    std::fs::remove_dir_all(&target)?;
                }
                copy_tree(&entry.path(), &target)?;
            }
            _ => copy_tree(&entry.path(), &target)?,
        }
    }
    Ok(())
}

/// Point `instance/<app>.wasm` at the global component, copying when the
/// filesystem refuses a symlink.
fn link_component(global: &Path, instance: &Path, app: &str) -> Result<()> {
    let link = instance.join(format!("{app}.wasm"));
    if std::fs::symlink_metadata(&link).is_ok() {
        std::fs::remove_file(&link)?;
    }
    if let Ok(relative) = relative_path(instance, global)
        && symlink_file(&relative, &link).is_ok()
    {
        return Ok(());
    }
    std::fs::copy(global, &link)?;
    Ok(())
}

/// Build the instance `config.toml`.
///
/// Precedence, lowest first: the manifest's `[config]`, the site's `plugins`
/// override for this app, and the values already in the instance file — the
/// operator's per-site edit is the most specific one and must survive an
/// upgrade.
fn render_instance_config(
    site: &str,
    manifest: &AppManifest,
    overrides: Option<&serde_json::Value>,
    previous: Option<&str>,
) -> Result<String> {
    let mut config = toml::Table::new();
    for (key, value) in &manifest.config_defaults {
        if let Some(value) = json_to_toml(value) {
            config.insert(key.clone(), value);
        }
    }
    if let Some(serde_json::Value::Object(map)) = overrides {
        for (key, value) in map {
            if let Some(value) = json_to_toml(value) {
                config.insert(key.clone(), value);
            }
        }
    }
    if let Some(previous) = previous {
        let table: toml::Table = toml::from_str(previous).map_err(|e| {
            Error::Config(ConfigError::Invalid {
                key: CONFIG_FILE.to_string(),
                msg: format!("{e} (site `{site}`, app `{}`)", manifest.name),
            })
        })?;
        if let Some(previous_config) = table.get("config").and_then(toml::Value::as_table) {
            for (key, value) in previous_config {
                config.insert(key.clone(), value.clone());
            }
        }
    }

    let mut ferris = toml::Table::new();
    ferris.insert("site".to_string(), toml::Value::String(site.to_string()));
    ferris.insert(
        "app".to_string(),
        toml::Value::String(manifest.name.clone()),
    );
    ferris.insert(
        "version".to_string(),
        toml::Value::String(manifest.version.clone()),
    );
    let mut root = toml::Table::new();
    root.insert("ferris".to_string(), toml::Value::Table(ferris));
    if !config.is_empty() {
        root.insert("config".to_string(), toml::Value::Table(config));
    }
    toml::to_string_pretty(&root).map_err(|e| {
        Error::Config(ConfigError::Invalid {
            key: CONFIG_FILE.to_string(),
            msg: format!("cannot encode the instance config: {e}"),
        })
    })
}

/// TOML has no `null`, so a default declared as JSON `null` is dropped instead
/// of failing the whole install.
fn json_to_toml(value: &serde_json::Value) -> Option<toml::Value> {
    toml::Value::try_from(value).ok()
}
