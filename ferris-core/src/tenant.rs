use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::DatabaseConfig;
use crate::error::{Result, TenantError};

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub database: DatabaseConfig,
    pub redis_url: String,
    pub files_path: PathBuf,
    pub admin_email: String,
    pub admin_password_hash: String,
    /// Whether the tenant is served. Disabled tenants are skipped by host
    /// resolution while keeping their configuration and data.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub enabled_apps: Vec<String>,
    pub wasm_apps: Vec<WasmAppConfig>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmAppConfig {
    pub name: String,
    pub version: String,
    pub wasm_path: PathBuf,
    pub config: serde_json::Value,
    pub permissions: Vec<String>,
}

impl TenantConfig {
    pub fn new(
        name: String,
        host: String,
        database: DatabaseConfig,
        redis_url: String,
        files_path: PathBuf,
        admin_email: String,
        admin_password_hash: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7().to_string(),
            name,
            host,
            database,
            redis_url,
            files_path,
            admin_email,
            admin_password_hash,
            enabled: true,
            enabled_apps: Vec::new(),
            wasm_apps: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn database_url(&self) -> String {
        self.database.connection_url()
    }
}

/// The `Host` values this process answers for, installed once before boot.
///
/// The documented deployment model is **one instance (one database, one
/// files root) per tenant**: requests are never routed to a tenant by their
/// `Host`, and the JWT `tenant` claim is not consulted. `ferris serve --tenant`
/// rewrites the process settings from a [`TenantConfig`] before the listener
/// starts, so `host` resolution here is a refusal list — a request addressed
/// to a name this instance does not serve is misdirected, not a tenant switch.
///
/// The list is process-wide because the listener is: an empty list keeps the
/// historical behaviour and restricts nothing.
static ALLOWED_HOSTS: OnceLock<Vec<String>> = OnceLock::new();

/// Fix the `Host` allow-list for this process.
///
/// Returns `false` when a previous installation already happened. Boot code
/// treats that as an error instead of silently replacing a policy another
/// component decided on.
pub fn install_allowed_hosts(hosts: Vec<String>) -> bool {
    ALLOWED_HOSTS.set(hosts).is_ok()
}

/// The installed allow-list; empty means "do not restrict".
#[must_use]
pub fn allowed_hosts() -> &'static [String] {
    match ALLOWED_HOSTS.get() {
        Some(hosts) => hosts.as_slice(),
        None => &[],
    }
}

/// The bare host of an authority: lower-cased, without userinfo, port or a
/// trailing dot.
///
/// Accepts a full origin (`https://erp.example.com:8443/x`), a `Host` header
/// (`ERP.Example.com:8000`) or a bracketed IPv6 literal. `None` for an empty
/// or structurally unusable value, so a malformed entry can never match.
#[must_use]
pub fn normalize_host(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((_, rest)) = value.split_once("://") {
        value = rest;
    }
    // `split` always yields at least one item; the fallback keeps this total.
    value = value.split('/').next().unwrap_or(value);
    if let Some((_, host)) = value.rsplit_once('@') {
        value = host;
    }
    let host = if let Some(rest) = value.strip_prefix('[') {
        &rest[..rest.find(']')?]
    } else {
        value.split(':').next().unwrap_or(value)
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Whether `host_header` names one of `allowed`.
///
/// An empty `allowed` is handled by the caller as "no restriction"; here it
/// answers `false` (fail closed) so this helper cannot be mistaken for an
/// allow-all matcher.
#[must_use]
pub fn host_is_allowed(host_header: &str, allowed: &[String]) -> bool {
    let Some(host) = normalize_host(host_header) else {
        return false;
    };
    allowed
        .iter()
        .filter_map(|entry| normalize_host(entry))
        .any(|entry| entry == host)
}

/// File-backed registry of tenant configurations.
///
/// Each tenant is stored as `<root>/<id>.json`, so provisioning does not depend
/// on a control database. [`TenantConfig::enabled`] gates host resolution
/// without deleting data.
///
/// This is the crate's one intentional piece of file I/O (see the crate docs):
/// the configurations are the bootstrap data, and a control database to hold
/// them would be a second bootstrap problem.
///
/// # Performance
///
/// Parsed tenants are cached per file signature (mtime + length) and only
/// re-read when that signature changes, so [`TenantManager::resolve_host`] does
/// not re-read and re-parse every tenant file on every request. An edit made by
/// another process is still picked up, because the signature is re-checked on
/// every call.
pub struct TenantManager {
    root: PathBuf,
    cache: RwLock<HashMap<PathBuf, CacheEntry>>,
}

#[derive(Clone)]
struct CacheEntry {
    /// `None` when the platform reports no mtime; then only `len` is compared.
    modified: Option<SystemTime>,
    len: u64,
    tenant: TenantConfig,
}
/// A tenant file that could not be used, and why.
///
/// Produced by [`TenantManager::load`] instead of the file vanishing from the
/// registry with no trace ("no silent success").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTenant {
    pub path: PathBuf,
    pub reason: String,
}

/// The result of reading the tenant directory: the tenants that loaded, plus one
/// diagnostic per file that was skipped.
#[derive(Debug, Clone, Default)]
pub struct TenantLoad {
    pub tenants: Vec<TenantConfig>,
    pub skipped: Vec<SkippedTenant>,
}

/// The signature a cache entry was parsed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    modified: Option<SystemTime>,
    len: u64,
}

fn signature(path: &std::path::Path) -> Option<FileSignature> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

impl TenantManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    fn cache(&self) -> std::sync::RwLockReadGuard<'_, HashMap<PathBuf, CacheEntry>> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }

    fn cache_mut(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<PathBuf, CacheEntry>> {
        self.cache.write().unwrap_or_else(|e| e.into_inner())
    }

    /// `FERRIS_TENANTS_DIR`, else `<config dir>/ferris/tenants`.
    pub fn default_root() -> PathBuf {
        if let Ok(dir) = std::env::var("FERRIS_TENANTS_DIR") {
            return PathBuf::from(dir);
        }
        dirs::config_dir()
            .map(|p| p.join("ferris").join("tenants"))
            .unwrap_or_else(|| PathBuf::from(".ferris/tenants"))
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn config_path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(TenantError::InvalidConfig(format!("invalid tenant id: {id}")).into());
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    /// Create a tenant.
    ///
    /// # Errors
    ///
    /// [`TenantError::InvalidConfig`] when the id is not a plain identifier —
    /// checked here first, because [`exists`](Self::exists) reports an invalid
    /// id as "not found" and the rejection would otherwise only surface from the
    /// filesystem write at the end. [`TenantError::AlreadyExists`] when the id
    /// or the requested host is already taken.
    pub fn create(&self, config: TenantConfig) -> Result<TenantConfig> {
        // The same rule `config_path` enforces, restated at the top so a
        // malformed id fails as a validation error instead of leaking out of a
        // later fs call: `exists()` cannot tell "invalid" from "absent".
        self.config_path(&config.id)?;
        if self.exists(&config.id) {
            return Err(TenantError::AlreadyExists(config.id).into());
        }
        if let Some(existing) = self.get_by_host(&config.host)? {
            if existing.id != config.id {
                return Err(TenantError::AlreadyExists(format!("host: {}", config.host)).into());
            }
        }
        self.write(&config)?;
        Ok(config)
    }

    /// Overwrite an existing tenant's configuration.
    ///
    /// Unlike the previous version this refuses an unknown id instead of
    /// creating a tenant as a side effect of an update — `create` is the only
    /// way in.
    ///
    /// # Errors
    ///
    /// [`TenantError::NotFound`] when no tenant with that id exists.
    pub fn update(&self, config: &TenantConfig) -> Result<()> {
        if !self.exists(&config.id) {
            return Err(TenantError::NotFound(config.id.clone()).into());
        }
        self.write(config)
    }

    pub fn get(&self, id: &str) -> Result<TenantConfig> {
        let path = self.config_path(id)?;
        let data =
            std::fs::read_to_string(&path).map_err(|_| TenantError::NotFound(id.to_string()))?;
        serde_json::from_str(&data).map_err(|e| TenantError::InvalidConfig(e.to_string()).into())
    }

    /// Whether a tenant with this id is stored.
    ///
    /// An *invalid* id (path traversal, empty) reports `false` rather than an
    /// error, because the signature has no way to say which. The id is validated
    /// again — and refused — by every operation that would touch the filesystem,
    /// starting with `config_path`.
    #[must_use]
    pub fn exists(&self, id: &str) -> bool {
        self.get(id).is_ok()
    }

    /// Look a tenant up by id, falling back to its (unique) name.
    pub fn find(&self, id_or_name: &str) -> Result<TenantConfig> {
        if let Ok(tenant) = self.get(id_or_name) {
            return Ok(tenant);
        }
        if let Some(tenant) = self.list()?.into_iter().find(|t| t.name == id_or_name) {
            return Ok(tenant);
        }
        Err(TenantError::NotFound(id_or_name.to_string()).into())
    }

    /// The loadable tenants, sorted by name.
    ///
    /// Files are re-read only when their signature (mtime + length) changed
    /// since the previous call, so a host lookup stops re-reading and re-parsing
    /// every tenant file on every request.
    ///
    /// A file that cannot be read or parsed is skipped here; it does not vanish
    /// silently, because [`TenantManager::load`] returns it in
    /// [`TenantLoad::skipped`]. Callers that surface diagnostics (the `tenant`
    /// CLI, a server at boot) should prefer `load`.
    ///
    /// # Errors
    ///
    /// Fails only when the directory itself cannot be read.
    pub fn list(&self) -> Result<Vec<TenantConfig>> {
        Ok(self.load()?.tenants)
    }

    /// Like [`TenantManager::list`], plus a diagnostic per skipped file.
    ///
    /// # Errors
    ///
    /// Fails only when the directory itself cannot be read.
    pub fn load(&self) -> Result<TenantLoad> {
        if !self.root.is_dir() {
            self.cache_mut().clear();
            return Ok(TenantLoad::default());
        }

        let mut tenants = Vec::new();
        let mut skipped = Vec::new();
        let mut fresh: HashMap<PathBuf, CacheEntry> = HashMap::new();

        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let current = signature(&path);
            if let Some(signature) = current {
                if let Some(entry) = self.cache().get(&path) {
                    if entry.modified == signature.modified && entry.len == signature.len {
                        tenants.push(entry.tenant.clone());
                        fresh.insert(path, entry.clone());
                        continue;
                    }
                }
            }
            match std::fs::read_to_string(&path) {
                Ok(data) => match serde_json::from_str::<TenantConfig>(&data) {
                    Ok(tenant) => {
                        if let Some(signature) = current {
                            fresh.insert(
                                path,
                                CacheEntry {
                                    modified: signature.modified,
                                    len: signature.len,
                                    tenant: tenant.clone(),
                                },
                            );
                        }
                        tenants.push(tenant);
                    }
                    Err(e) => skipped.push(SkippedTenant {
                        path,
                        reason: format!("invalid tenant JSON: {e}"),
                    }),
                },
                Err(e) => skipped.push(SkippedTenant {
                    path,
                    reason: format!("unreadable: {e}"),
                }),
            }
        }

        *self.cache_mut() = fresh;
        tenants.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(TenantLoad { tenants, skipped })
    }

    pub fn get_by_host(&self, host: &str) -> Result<Option<TenantConfig>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|t| t.host.eq_ignore_ascii_case(host)))
    }

    /// The tenant that should serve `host`, skipping disabled tenants.
    pub fn resolve_host(&self, host: &str) -> Result<Option<TenantConfig>> {
        Ok(self.get_by_host(host)?.filter(|t| t.enabled))
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<TenantConfig> {
        let mut config = self.get(id)?;
        config.enabled = enabled;
        config.updated_at = Utc::now();
        self.write(&config)?;
        Ok(config)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.config_path(id)?;
        std::fs::remove_file(&path).map_err(|_| TenantError::NotFound(id.to_string()))?;
        self.cache_mut().remove(&path);
        Ok(())
    }

    fn write(&self, config: &TenantConfig) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        // The directory and the file carry the database password, the Redis URL
        // and the admin hash. A process umask of 022 used to leave both readable
        // by every local user, so the modes are set explicitly instead of
        // inherited.
        restrict_dir(&self.root)?;
        let path = self.config_path(&config.id)?;
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| TenantError::InvalidConfig(e.to_string()))?;
        write_private(&path, &json)?;
        // Refresh the entry explicitly: a same-size write within the same mtime
        // tick would otherwise look unchanged to the signature check, and the
        // caller that just wrote must see its own value.
        if let Some(signature) = signature(&path) {
            self.cache_mut().insert(
                path,
                CacheEntry {
                    modified: signature.modified,
                    len: signature.len,
                    tenant: config.clone(),
                },
            );
        }
        Ok(())
    }
}

impl Default for TenantManager {
    fn default() -> Self {
        Self::new(Self::default_root())
    }
}

/// Make the tenant store readable only by its owner (`0700`).
#[cfg(unix)]
fn restrict_dir(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Windows has no Unix mode bits; the file inherits the ACL of its directory.
#[cfg(not(unix))]
fn restrict_dir(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

/// Write a tenant file readable only by its owner (`0600`).
///
/// The mode is passed at creation time and then re-applied, because `create`
/// does not change the mode of a file that already exists — a tenant file left
/// at `0644` by an older build must be tightened by the next update, not stay
/// world-readable.
#[cfg(unix)]
fn write_private(path: &std::path::Path, contents: &str) -> Result<()> {
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
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(id: &str, host: &str) -> TenantConfig {
        TenantConfig {
            id: id.into(),
            name: id.into(),
            host: host.into(),
            database: DatabaseConfig {
                r#type: crate::config::DatabaseType::Postgres,
                host: "localhost".into(),
                port: 5432,
                name: format!("{id}_db"),
                user: "user".into(),
                password: "pass".into(),
                max_connections: 10,
                min_connections: 2,
                connect_timeout_secs: 10,
                idle_timeout_secs: 300,
                max_lifetime_secs: 1800,
                ssl_mode: crate::config::SslMode::Prefer,
                schema: Some("public".into()),
            },
            redis_url: "redis://localhost".into(),
            files_path: PathBuf::from("/tmp/files"),
            admin_email: "admin@example.com".into(),
            admin_password_hash: "hash".into(),
            enabled: true,
            enabled_apps: vec![],
            wasm_apps: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_tenant_config_database_url() {
        let config = sample_config("test", "test.local");
        let url = config.database_url();
        assert!(url.contains("postgres://"));
        assert!(url.contains("user:pass@localhost:5432/test_db"));
    }

    #[test]
    fn test_tenant_manager_crud_and_host_resolution() {
        let root = std::env::temp_dir().join(format!("ferris-tenants-{}", Uuid::new_v4()));
        let manager = TenantManager::new(&root);

        manager
            .create(sample_config("acme", "acme.example.com"))
            .unwrap();
        assert!(manager.exists("acme"));
        assert_eq!(manager.list().unwrap().len(), 1);

        // Host lookup is case-insensitive and resolves enabled tenants.
        assert_eq!(
            manager
                .resolve_host("ACME.example.com")
                .unwrap()
                .unwrap()
                .id,
            "acme"
        );

        // Duplicate host is rejected.
        assert!(
            manager
                .create(sample_config("other", "acme.example.com"))
                .is_err()
        );

        // Disabled tenants are not resolved.
        manager.set_enabled("acme", false).unwrap();
        assert!(manager.resolve_host("acme.example.com").unwrap().is_none());

        // Path traversal in the id is rejected.
        assert!(manager.get("../etc/passwd").is_err());

        manager.delete("acme").unwrap();
        assert!(!manager.exists("acme"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The tenant file holds `database.password`, `redis_url` and the admin
    /// hash. Both the store and the file must be owner-only, and an update must
    /// tighten a file that an older build left world-readable.
    #[cfg(unix)]
    #[test]
    fn tenant_store_and_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("ferris-tenants-{}", Uuid::new_v4()));
        let manager = TenantManager::new(&root);
        manager
            .create(sample_config("acme", "acme.example.com"))
            .unwrap();

        let dir_mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            dir_mode, 0o700,
            "the tenant store must not be world-readable"
        );

        let path = root.join("acme.json");
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            file_mode, 0o600,
            "the tenant file must not be world-readable"
        );

        // An existing file keeps its mode on a plain write; the update path has
        // to set it again.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        manager.set_enabled("acme", false).unwrap();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `create` validates the id before it consults `exists()`: an invalid id is
    /// an `InvalidConfig`, not an "already exists"/"not found" confusion, and
    /// nothing is written for it.
    #[test]
    fn create_rejects_an_invalid_id_as_a_validation_error() {
        let root = std::env::temp_dir().join(format!("ferris-tenants-{}", Uuid::new_v4()));
        let manager = TenantManager::new(&root);

        let err = manager
            .create(sample_config("../etc/passwd", "evil.example.com"))
            .unwrap_err();
        assert!(
            matches!(err, crate::Error::Tenant(TenantError::InvalidConfig(_))),
            "{err}"
        );
        assert!(
            !root.join("..").join("etc").join("passwd.json").exists(),
            "an invalid id must not write anything"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A corrupt tenant file must not make a tenant disappear without a trace:
    /// the load still succeeds, and the file is reported in `skipped`.
    #[test]
    fn a_malformed_tenant_file_is_reported_not_silently_dropped() {
        let root = std::env::temp_dir().join(format!("ferris-tenants-{}", Uuid::new_v4()));
        let manager = TenantManager::new(&root);
        manager
            .create(sample_config("acme", "acme.example.com"))
            .unwrap();
        std::fs::write(root.join("broken.json"), "{ not json").unwrap();

        let load = manager.load().unwrap();
        assert_eq!(load.tenants.len(), 1);
        assert_eq!(load.skipped.len(), 1);
        assert!(load.skipped[0].path.ends_with("broken.json"));
        assert!(
            load.skipped[0].reason.contains("invalid tenant JSON"),
            "{}",
            load.skipped[0].reason
        );
        // `list()` still returns the loadable tenant, and the diagnostic stayed
        // reachable through `load()`.
        assert_eq!(manager.list().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The cache must not hide an edit made behind the manager's back, which is
    /// why a hit requires the file signature to match.
    #[test]
    fn an_external_edit_is_picked_up_despite_the_cache() {
        let root = std::env::temp_dir().join(format!("ferris-tenants-{}", Uuid::new_v4()));
        let manager = TenantManager::new(&root);
        manager
            .create(sample_config("acme", "acme.example.com"))
            .unwrap();
        // Warm the cache and prove the signature still matches.
        assert_eq!(resolve_name(&manager, "acme.example.com"), "acme");

        let mut changed = sample_config("acme", "acme.example.com");
        changed.name = "Acme Renamed".into();
        // Write the file directly, as another process would.
        std::fs::write(
            root.join("acme.json"),
            serde_json::to_string_pretty(&changed).unwrap(),
        )
        .unwrap();

        assert_eq!(resolve_name(&manager, "acme.example.com"), "Acme Renamed");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn resolve_name(manager: &TenantManager, host: &str) -> String {
        manager
            .resolve_host(host)
            .unwrap()
            .expect("tenant is enabled and resolves")
            .name
    }

    /// An update must not create the tenant it claims to update.
    #[test]
    fn update_of_an_unknown_tenant_is_not_a_create() {
        let root = std::env::temp_dir().join(format!("ferris-tenants-{}", Uuid::new_v4()));
        let manager = TenantManager::new(&root);

        let err = manager
            .update(&sample_config("ghost", "ghost.example.com"))
            .unwrap_err();
        assert!(
            matches!(err, crate::Error::Tenant(TenantError::NotFound(_))),
            "{err:?}"
        );
        assert!(!manager.exists("ghost"));
        assert!(!root.join("ghost.json").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `[server] allowed_hosts` compares names, not spellings: case, a port and
    /// a trailing dot all describe the same host.
    #[test]
    fn a_host_matches_ignoring_case_port_and_trailing_dot() {
        let allowed = vec![
            "erp.example.com".to_string(),
            "https://desk.example.com:8443".to_string(),
        ];
        assert!(host_is_allowed("ERP.EXAMPLE.COM:8000", &allowed));
        assert!(host_is_allowed("erp.example.com.", &allowed));
        assert!(host_is_allowed("desk.example.com", &allowed));
        assert!(host_is_allowed("https://erp.example.com/path", &allowed));
    }

    /// A different name, a malformed value or an empty list is a refusal. The
    /// empty list is only unreachable because the router skips the middleware;
    /// the helper itself must never read as allow-all.
    #[test]
    fn an_unknown_or_malformed_host_is_refused() {
        let allowed = vec!["erp.example.com".to_string()];
        assert!(!host_is_allowed("evil.example.com", &allowed));
        assert!(!host_is_allowed("erp.example.com.evil.test", &allowed));
        assert!(!host_is_allowed("", &allowed));
        assert!(!host_is_allowed("erp.example.com", &[]));
        assert!(normalize_host("   ").is_none());
    }
}
