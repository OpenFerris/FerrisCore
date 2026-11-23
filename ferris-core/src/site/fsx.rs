//! Filesystem primitives shared by the site manager.
//!
//! Two policies live here so every call site applies the same one:
//!
//! * **private by default** — a site directory carries `db_password` and its
//!   plugin `config.toml` carries app credentials, so directories are forced to
//!   `0700` and config files to `0600` instead of inheriting the process umask
//!   (`022` would leave them readable by every local user). This mirrors the
//!   tenant store, whose helpers are private to its module.
//! * **never follow a symlink out of a package** — an app package is data from
//!   outside the project, so a symlink inside it must not become a way to copy
//!   `/etc` into `apps/`. Only regular files and directories are copied.

use std::path::{Path, PathBuf};

use crate::{Error, FileError, Result};

/// Create `dir` (and its parents) and make it owner-only.
pub(super) fn create_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    restrict_dir(dir)
}

/// Tighten an existing directory to `0700`.
pub(super) fn restrict_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        // Windows has no Unix mode bits; the directory inherits its ACL.
        let _ = dir;
    }
    Ok(())
}

/// Write `contents`, readable only by its owner.
///
/// The mode is applied at creation *and* afterwards, because opening an
/// existing file neither creates it nor changes its mode: a `site_config.json`
/// left at `0644` by an older build must be tightened by the next write.
pub(super) fn write_private(path: &Path, contents: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)?;
    }
    Ok(())
}

/// Write `contents` to `path` through a sibling temporary file and a rename.
///
/// `apps.txt` is read by another process (the server booting a site) while the
/// CLI edits it; a half-written file would make an app disappear. The rename is
/// atomic on the same filesystem, which is why the temporary file is a sibling.
pub(super) fn write_private_atomic(path: &Path, contents: &str) -> Result<()> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    write_private(&tmp, contents)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::File(FileError::Storage(format!(
            "cannot replace {}: {e}",
            path.display()
        )))
    })
}

/// Copy a file or directory tree, refusing to follow a symlink.
///
/// A symlink found in the source is skipped: the copy must stay a copy of what
/// the package contains, never of wherever a link points.
pub(super) fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(src)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(());
    }
    if file_type.is_dir() {
        create_private_dir(dst)?;
        let mut entries = std::fs::read_dir(src)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(())
}
