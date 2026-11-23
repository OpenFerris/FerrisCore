//! Behaviour of the site layout: creation, host resolution, the two install
//! scopes, upgrades, purge and the owner-only permissions.

use std::path::{Path, PathBuf};

use serde_json::json;
use uuid::Uuid;

use crate::config::{DatabaseType, SslMode};

use super::{APPS_FILE, AppManifest, SiteConfig, SiteManager, is_valid_site_name};

fn temp_manager(name: &str) -> (SiteManager, PathBuf) {
    let root = std::env::temp_dir().join(format!("ferris-site-test-{name}-{}", Uuid::new_v4()));
    let manager = SiteManager::new(root.join("sites"), root.join("apps"));
    (manager, root)
}

fn site(name: &str) -> SiteConfig {
    SiteConfig::new(name)
}

/// Build a minimal but complete app package: manifest, component and the four
/// world directories, with a starter customization and a secret that must not
/// travel to a site.
fn app_package(root: &Path, name: &str) -> PathBuf {
    let package = root.join("package").join(name);
    std::fs::create_dir_all(package.join("world/customizations")).unwrap();
    std::fs::create_dir_all(package.join("world/data")).unwrap();
    std::fs::create_dir_all(package.join("world/files")).unwrap();
    std::fs::create_dir_all(package.join("world/secrets")).unwrap();
    std::fs::write(package.join("world/customizations/ToDo.json"), "{}").unwrap();
    std::fs::write(package.join("world/data/starter.txt"), "starter").unwrap();
    std::fs::write(package.join("world/secrets/token"), "must-not-copy").unwrap();
    std::fs::write(
        package.join("plugin.toml"),
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\n\n[config]\ncurrency = \"EUR\"\nlocale = \"es\"\n"
        ),
    )
    .unwrap();
    std::fs::write(package.join(format!("{name}.wasm")), b"\0asm").unwrap();
    package
}

#[test]
fn creating_a_site_lays_out_every_directory() {
    let (manager, root) = temp_manager("layout");
    manager.create(site("acme")).unwrap();

    let dir = manager.sites_root().join("acme");
    for relative in ["private/files", "public/files", "plugins"] {
        assert!(dir.join(relative).is_dir(), "{relative}");
    }
    assert!(dir.join("site_config.json").is_file());
    assert_eq!(std::fs::read_to_string(dir.join(APPS_FILE)).unwrap(), "");

    let common = manager.sites_root().join("common_site_config.json");
    assert!(common.is_file(), "the shared defaults file must exist");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&common).unwrap()).unwrap();
    assert!(parsed.get("redis_url").is_some());

    // A freshly created site resolves by its own name and points its files at
    // the private root the layout just created.
    let config = manager.get("acme").unwrap();
    assert_eq!(config.host_name, "acme");
    assert_eq!(config.files_path, dir.join("private/files"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn sites_are_listed_and_resolved_by_host() {
    let (manager, root) = temp_manager("resolve");
    let mut erp = site("erp");
    erp.host_name = "erp.example.com".into();
    manager.create(erp).unwrap();
    manager.create(site("desk")).unwrap();

    let names: Vec<String> = manager
        .list()
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, vec!["desk".to_string(), "erp".to_string()]);

    // A port, the case and a trailing dot all name the same site.
    assert_eq!(
        manager
            .resolve_host("ERP.example.com:8443")
            .unwrap()
            .unwrap()
            .name,
        "erp"
    );
    assert_eq!(manager.resolve_host("DESK").unwrap().unwrap().name, "desk");
    assert!(
        manager.resolve_host("erp").unwrap().is_none(),
        "a site answers for its host_name, not for its directory name"
    );
    assert!(
        manager
            .resolve_host("unknown.example.com")
            .unwrap()
            .is_none()
    );

    // Disabled sites are not served; a duplicate host is refused.
    manager.set_enabled("erp", false).unwrap();
    assert!(manager.resolve_host("erp.example.com").unwrap().is_none());
    let mut other = site("other");
    other.host_name = "erp.example.com".into();
    assert!(manager.create(other).is_err(), "duplicate host");
    assert!(manager.create(site("erp")).is_err(), "duplicate name");
    assert!(
        manager.update(&site("ghost")).is_err(),
        "update is not create"
    );

    manager.delete("desk").unwrap();
    assert!(!manager.exists("desk"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_global_app_and_its_instance_carry_the_expected_layout() {
    let (manager, root) = temp_manager("install");
    let package = app_package(&root, "facturacion");
    manager.create(site("acme")).unwrap();

    let global = manager.install_global_app(&package).unwrap();
    assert_eq!(global.name, "facturacion");
    assert_eq!(global.version, "0.1.0");
    assert_eq!(global.config_defaults["currency"], json!("EUR"));
    assert_eq!(manager.global_apps().unwrap(), vec!["facturacion"]);

    let app_dir = manager.apps_root().join("facturacion");
    assert!(app_dir.join("facturacion.wasm").is_file());
    assert!(app_dir.join("plugin.toml").is_file());
    assert!(app_dir.join("assets").is_dir());
    assert!(app_dir.join("world/data/starter.txt").is_file());
    // The template's secret never reaches the catalogue either.
    assert!(!app_dir.join("world/secrets/token").exists());

    let manifest = manager.install_app("acme", "facturacion", false).unwrap();
    assert_eq!(manifest.wasm, app_dir.join("facturacion.wasm"));

    let instance = manager.instance_world("acme", "facturacion").unwrap();
    for relative in ["customizations", "data", "files", "secrets"] {
        assert!(instance.join(relative).is_dir(), "{relative}");
    }
    assert!(
        instance.join("customizations/ToDo.json").is_file(),
        "template copied"
    );
    assert!(
        !instance.join("secrets/token").exists(),
        "no secret is copied"
    );
    assert!(
        instance.join("facturacion.wasm").exists(),
        "component referenced"
    );
    assert!(instance.join("config.toml").is_file());
    assert_eq!(manager.installed_apps("acme").unwrap(), vec!["facturacion"]);
    assert_eq!(manager.get("acme").unwrap().apps, vec!["facturacion"]);

    // A second install without force is refused, and nothing is rewritten.
    assert!(manager.install_app("acme", "facturacion", false).is_err());
    // An app that is not in the catalogue cannot be installed.
    assert!(manager.install_app("acme", "missing", false).is_err());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn instance_config_carries_defaults_and_the_site_override() {
    let (manager, root) = temp_manager("config");
    let package = app_package(&root, "facturacion");
    let mut acme = site("acme");
    acme.plugins
        .insert("facturacion".to_string(), json!({ "locale": "gl" }));
    manager.create(acme).unwrap();
    manager.install_global_app(&package).unwrap();
    manager.install_app("acme", "facturacion", false).unwrap();

    let instance = manager.instance_world("acme", "facturacion").unwrap();
    let text = std::fs::read_to_string(instance.join("config.toml")).unwrap();
    let parsed: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(parsed["ferris"]["site"].as_str(), Some("acme"));
    assert_eq!(parsed["ferris"]["app"].as_str(), Some("facturacion"));
    // The manifest default, overridden by the site's `plugins` block.
    assert_eq!(parsed["config"]["currency"].as_str(), Some("EUR"));
    assert_eq!(parsed["config"]["locale"].as_str(), Some("gl"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn names_are_never_paths() {
    let (manager, root) = temp_manager("traversal");
    manager.create(site("acme")).unwrap();

    for bad in ["../etc", "..", ".", "a/b", "a\\b", "", " with space"] {
        assert!(!is_valid_site_name(bad), "{bad} must be refused");
        assert!(
            manager.site_dir(bad).is_err(),
            "{bad} must not become a path"
        );
        assert!(manager.get(bad).is_err());
    }
    assert!(manager.instance_world("acme", "../evil").is_err());
    assert!(manager.template_world("../evil").is_err());

    // A package that declares a traversal name cannot install anywhere.
    let package = root.join("package").join("evil");
    std::fs::create_dir_all(package.join("world")).unwrap();
    std::fs::write(package.join("plugin.toml"), "name = \"../evil\"\n").unwrap();
    std::fs::write(package.join("../evil.wasm"), b"\0asm").unwrap();
    assert!(AppManifest::read(&package).is_err());
    assert!(!root.join("apps").join("..").join("evil").exists());

    // Nothing was created outside the sites root.
    assert_eq!(manager.list().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn config_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let (manager, root) = temp_manager("perms");
    let package = app_package(&root, "facturacion");
    manager.create(site("acme")).unwrap();
    manager.install_global_app(&package).unwrap();
    manager.install_app("acme", "facturacion", false).unwrap();

    let dir = manager.site_dir("acme").unwrap();
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let config = dir.join("site_config.json");
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // An update must tighten a file an older build left world-readable.
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
    manager.set_enabled("acme", false).unwrap();
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let instance_config = manager
        .instance_world("acme", "facturacion")
        .unwrap()
        .join("config.toml");
    assert_eq!(
        std::fs::metadata(&instance_config)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_forced_upgrade_keeps_the_site_data_and_its_config_edits() {
    let (manager, root) = temp_manager("upgrade");
    let package = app_package(&root, "facturacion");
    manager.create(site("acme")).unwrap();
    manager.install_global_app(&package).unwrap();
    manager.install_app("acme", "facturacion", false).unwrap();

    let instance = manager.instance_world("acme", "facturacion").unwrap();
    std::fs::write(instance.join("data/site-own.txt"), "keep me").unwrap();
    std::fs::write(instance.join("secrets/token"), "site secret").unwrap();
    let config_path = instance.join("config.toml");
    let mut config: toml::Table =
        toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["config"]["currency"] = toml::Value::String("USD".into());
    std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

    // The new template drops `locale` and rewrites the customization.
    let template = manager.template_world("facturacion").unwrap();
    std::fs::write(template.join("customizations/ToDo.json"), "{\"v\":2}").unwrap();
    std::fs::write(template.join("data/starter.txt"), "template v2").unwrap();

    manager.install_app("acme", "facturacion", true).unwrap();

    assert_eq!(
        std::fs::read_to_string(instance.join("data/site-own.txt")).unwrap(),
        "keep me"
    );
    assert_eq!(
        std::fs::read_to_string(instance.join("secrets/token")).unwrap(),
        "site secret"
    );
    assert_eq!(
        std::fs::read_to_string(instance.join("customizations/ToDo.json")).unwrap(),
        "{\"v\":2}",
        "the template's customizations are replaced"
    );
    let parsed: toml::Table =
        toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        parsed["config"]["currency"].as_str(),
        Some("USD"),
        "the operator's per-site value survives"
    );
    assert_eq!(parsed["config"]["locale"].as_str(), Some("es"));
    assert_eq!(
        std::fs::read_to_string(instance.join("data/starter.txt")).unwrap(),
        "starter",
        "instance data is not replaced by the template"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn uninstall_clears_apps_txt_and_purges_only_when_asked() {
    let (manager, root) = temp_manager("uninstall");
    let package = app_package(&root, "facturacion");
    manager.create(site("acme")).unwrap();
    manager.install_global_app(&package).unwrap();
    manager.install_app("acme", "facturacion", false).unwrap();
    let instance = manager.instance_world("acme", "facturacion").unwrap();

    manager.uninstall_app("acme", "facturacion", false).unwrap();
    assert!(manager.installed_apps("acme").unwrap().is_empty());
    assert!(manager.get("acme").unwrap().apps.is_empty());
    assert!(instance.is_dir(), "without purge the data stays");

    // A later reinstall reuses the kept directory without a forced upgrade.
    manager
        .install_app("acme", "facturacion", false)
        .unwrap_err();
    manager.install_app("acme", "facturacion", true).unwrap();
    manager.uninstall_app("acme", "facturacion", true).unwrap();
    assert!(!instance.exists(), "purge removes the instance");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn site_config_round_trips_the_flat_frappe_keys() {
    let mut config = SiteConfig::new("acme");
    config.database.name = "acme_db".into();
    config.database.user = "acme".into();
    config.database.password = "s3cret".into();
    config.database.r#type = DatabaseType::MySQL;

    let json = config.to_json().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    for key in [
        "db_name",
        "db_type",
        "db_host",
        "db_port",
        "db_user",
        "db_password",
        "db_ssl_mode",
    ] {
        assert!(parsed.get(key).is_some(), "missing flat key {key}");
    }
    assert_eq!(parsed["db_type"], json!("mysql"));

    let back = SiteConfig::from_json(&json, "acme").unwrap();
    assert_eq!(back.database.name, "acme_db");
    assert_eq!(back.database.r#type, DatabaseType::MySQL);
    assert_eq!(back.database.ssl_mode, SslMode::Prefer);
    assert_eq!(back.database.password, "s3cret");

    // A minimal file (a Frappe site, or one written by hand) still loads.
    let minimal = SiteConfig::from_json("{\"db_name\": \"erp\"}", "erp").unwrap();
    assert_eq!(minimal.name, "erp");
    assert_eq!(minimal.host_name, "erp");
    assert!(minimal.enabled);
    assert_eq!(minimal.database.name, "erp");
    assert_eq!(minimal.database.port, 5432);
    assert_eq!(minimal.redis_url, super::config::DEFAULT_REDIS_URL);

    // `mariadb` is Frappe's spelling for the MySQL dialect; an unknown type is
    // an error, never a silent Postgres fallback.
    let mariadb = SiteConfig::from_json("{\"name\": \"erp\", \"db_type\": \"mariadb\"}", "erp")
        .expect("mariadb maps to mysql");
    assert_eq!(mariadb.database.r#type, DatabaseType::MySQL);
    assert!(SiteConfig::from_json("{\"db_type\": \"oracle\"}", "erp").is_err());
}

/// A site copies the component and pins the version it installed: replacing
/// the catalogue must not change what the site serves until `force`.
#[test]
fn the_instance_is_a_copy_pinned_to_the_installed_version() {
    let (manager, root) = temp_manager("copy");
    manager.create(site("acme")).unwrap();
    let package = app_package(&root, "facturacion");
    std::fs::write(package.join("facturacion.wasm"), b"version-one").unwrap();
    manager.install_global_app(&package).unwrap();
    manager.install_app("acme", "facturacion", false).unwrap();

    let instance = manager
        .sites_root()
        .join("acme/plugins/facturacion/facturacion.wasm");
    assert!(instance.is_file(), "the instance must carry the component");
    assert!(
        !std::fs::symlink_metadata(&instance)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the component must be a copy, not a symlink"
    );
    assert_eq!(std::fs::read(&instance).unwrap(), b"version-one");
    assert_eq!(
        manager
            .installed_app_version("acme", "facturacion")
            .unwrap(),
        Some("0.1.0".to_string())
    );

    // Moving the catalogue forward does not touch the pinned copy.
    std::fs::write(
        manager.apps_root().join("facturacion/facturacion.wasm"),
        b"version-two",
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&instance).unwrap(),
        b"version-one",
        "the site keeps serving its pinned component"
    );

    // An explicit upgrade copies the new component.
    manager.install_app("acme", "facturacion", true).unwrap();
    assert_eq!(std::fs::read(&instance).unwrap(), b"version-two");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_old_site_config_loads_with_the_new_defaults() {
    let json = r#"{"name":"acme","db_type":"sqlite","db_name":"acme"}"#;
    let config = SiteConfig::from_json(json, "acme").unwrap();
    assert_eq!(config.default_locale, "en");
    assert!(config.supported_locales.contains(&"es".to_string()));
    assert!(config.app_versions.is_empty());
}

/// A site installed before the copy model has a symlink to the catalogue.
/// Booting or migrating it must turn that link into the pinned copy.
#[cfg(unix)]
#[test]
fn a_legacy_symlink_is_materialized_into_a_copy() {
    let (manager, root) = temp_manager("materialize");
    manager.create(site("acme")).unwrap();
    let package = app_package(&root, "facturacion");
    manager.install_global_app(&package).unwrap();
    manager.install_app("acme", "facturacion", false).unwrap();

    let instance = manager
        .sites_root()
        .join("acme/plugins/facturacion/facturacion.wasm");
    std::fs::remove_file(&instance).unwrap();
    std::os::unix::fs::symlink(
        manager.apps_root().join("facturacion/facturacion.wasm"),
        &instance,
    )
    .unwrap();

    // A pre-record install has no version yet.
    let mut config = manager.get("acme").unwrap();
    config.app_versions.clear();
    manager.update(&config).unwrap();

    assert_eq!(manager.materialize_instance_components("acme").unwrap(), 1);
    assert!(
        !std::fs::symlink_metadata(&instance)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link must become a real copy"
    );
    assert_eq!(
        manager
            .installed_app_version("acme", "facturacion")
            .unwrap(),
        Some("0.1.0".to_string())
    );
    let _ = std::fs::remove_dir_all(&root);
}
