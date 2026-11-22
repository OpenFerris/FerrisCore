//! The on-disk **world** of a plugin.
//!
//! A plugin is not only a `.wasm` file: it owns a directory, `plugins/<app>/`,
//! where everything that belongs to it lives — its exported customizations, its
//! data, the files it stores and its secrets. The directory is the unit of
//! isolation: a guest that is given a filesystem sees that root (and nothing
//! else), and an export writes inside it instead of into the framework's data.
//!
//! The layout is fixed so the CLI, the server and the guest agree:
//!
//! ```text
//! plugins/<app>/
//!   customizations/   exported Custom Field / Property Setter payloads
//!   data/             the plugin's own files
//!   files/            blobs it attaches
//!   secrets/          secret material (never served)
//! ```
//!
//! The name is validated here, once: it becomes a path component, so a `..`, a
//! separator or an absolute path must never reach the filesystem.

use std::path::{Component, Path, PathBuf};

use crate::{Error, Result, ValidationError};

/// The sub-directories every world carries.
pub const WORLD_DIRS: [&str; 4] = ["customizations", "data", "files", "secrets"];

/// Whether `app` is a usable plugin name (and therefore a safe path segment).
///
/// Plugin names are crate names: ASCII letters, digits, `-` and `_`. Anything
/// else (a dot, a slash, a space) is refused instead of sanitized, because a
/// silent rewrite would make two plugins collide.
#[must_use]
pub fn is_valid_app_name(app: &str) -> bool {
    let app = app.trim();
    !app.is_empty()
        && app.len() <= 64
        && app
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && app != "."
        && app != ".."
}

/// The directory that holds every plugin, from the environment or the project.
///
/// `FERRIS_PLUGINS_DIR` wins (a deployment can keep worlds outside the project);
/// otherwise `<project>/plugins`, and `./plugins` as the last resort. The CLI
/// runs with the project as its working directory, so this matches the
/// `[apps] wasm_dir` default without a second configuration source.
#[must_use]
pub fn plugins_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("FERRIS_PLUGINS_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("FERRIS_PROJECT_DIR") {
        return PathBuf::from(dir).join("plugins");
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("plugins")
}

/// `plugins/<app>` for a validated name.
///
/// # Errors
///
/// [`ValidationError::Invalid`] when the name is not a safe path segment.
pub fn world_root(app: &str) -> Result<PathBuf> {
    world_root_at(&plugins_root(), app)
}

/// [`world_root`] against an explicit plugins directory (for tests and tools).
///
/// # Errors
///
/// [`ValidationError::Invalid`] when the name is not a safe path segment.
pub fn world_root_at(plugins: &Path, app: &str) -> Result<PathBuf> {
    if !is_valid_app_name(app) {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "invalid app name `{app}`: use letters, digits, `-` or `_`"
        ))));
    }
    Ok(plugins.join(app.trim()))
}

/// Create the world (and its sub-directories) if it does not exist.
///
/// # Errors
///
/// A validation error for an unsafe name, or a storage error when the
/// directories cannot be created.
pub fn ensure_world(app: &str) -> Result<PathBuf> {
    ensure_world_at(&plugins_root(), app)
}

/// [`ensure_world`] against an explicit plugins directory.
///
/// # Errors
///
/// Same as [`ensure_world`].
pub fn ensure_world_at(plugins: &Path, app: &str) -> Result<PathBuf> {
    let world = world_root_at(plugins, app)?;
    for relative in WORLD_DIRS {
        let dir = world.join(relative);
        std::fs::create_dir_all(&dir).map_err(|e| {
            Error::File(crate::FileError::Storage(format!(
                "cannot create {}: {e}",
                dir.display()
            )))
        })?;
    }
    Ok(world)
}

/// A path **inside** `world`, refusing anything that escapes it.
///
/// `relative` is a caller-built path (`customizations/<doctype>.json`). It must
/// stay relative and must not contain `..`; the join is then checked again as a
/// whole so a future caller cannot bypass the rule.
///
/// # Errors
///
/// [`ValidationError::Invalid`] when the relative path is absolute, empty or
/// contains a parent component.
pub fn world_path(world: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "invalid path inside a plugin world: {}",
            relative.display()
        ))));
    }
    let joined = world.join(relative);
    let base = world.components().count();
    let inside = joined.components().take(base).eq(world.components());
    if inside {
        Ok(joined)
    } else {
        Err(Error::Validation(ValidationError::Invalid(format!(
            "path escapes the plugin world: {}",
            relative.display()
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("ferris-world-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn app_names_are_safe_path_segments() {
        for valid in ["test", "my-app", "a_b", "App2"] {
            assert!(is_valid_app_name(valid), "{valid}");
        }
        for invalid in ["", " ", "..", ".", "a/b", "a\\b", "a b", "a.b", "../x"] {
            assert!(!is_valid_app_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn a_world_is_created_with_every_subdirectory() {
        let root = temp_root("create");
        let world = ensure_world_at(&root, "demo").expect("world");
        assert_eq!(world, root.join("demo"));
        for dir in WORLD_DIRS {
            assert!(world.join(dir).is_dir(), "{dir}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_invalid_name_never_becomes_a_path() {
        let root = temp_root("invalid");
        assert!(world_root_at(&root, "../escape").is_err());
        assert!(ensure_world_at(&root, "a/b").is_err());
        assert!(!root.exists(), "nothing may be created for an invalid name");
    }

    #[test]
    fn world_paths_cannot_escape() {
        let world = Path::new("/srv/plugins/demo");
        assert_eq!(
            world_path(world, Path::new("customizations/ToDo.json")).unwrap(),
            PathBuf::from("/srv/plugins/demo/customizations/ToDo.json")
        );
        assert!(world_path(world, Path::new("../other/secret")).is_err());
        assert!(world_path(world, Path::new("/etc/passwd")).is_err());
        assert!(world_path(world, Path::new("")).is_err());
    }
}
