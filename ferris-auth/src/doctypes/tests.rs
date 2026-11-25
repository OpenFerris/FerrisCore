//! Tests for the Users-section DocType declarations.

use super::*;
use ferris_schema::get_doctype;

#[test]
fn every_doctype_registers_with_fields() {
    register_doctypes();
    for meta in doctype_metas() {
        let got = get_doctype(&meta.name).expect("registered");
        assert_eq!(got.module, MODULE, "{}", meta.name);
        assert!(!got.fields.is_empty(), "{} needs fields", meta.name);
    }
}

#[test]
fn no_users_doctype_is_a_single() {
    assert!(doctype_metas().iter().all(|m| !m.is_single));
}

#[test]
fn user_and_role_are_declared_for_the_workspace() {
    register_doctypes();
    assert!(get_doctype("User").is_some());
    assert!(get_doctype("Role").is_some());
}

#[test]
fn register_is_idempotent() {
    register_doctypes();
    register_doctypes();
    assert!(get_doctype("User Permission").is_some());
    assert!(get_doctype("Activity Log").is_some());
}

/// A credential must be a `Password` field so the engine hashes it on write
/// and masks it on read, and the fields only an administrator may see must
/// carry `permlevel = 1`.
#[test]
fn user_credentials_are_password_fields_and_admin_only() {
    let meta = user_meta();
    let field = |name: &str| {
        meta.fields
            .iter()
            .find(|f| f.fieldname == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
    };

    assert_eq!(field("password").fieldtype, "Password");
    assert!(field("password").hidden);

    // `api_secret` is hashed like a password (verified by hashing the
    // presented value). `mfa_secret` must stay reversible — TOTP needs the
    // seed — so it is a hidden administrator-only field, not `Password`.
    assert_eq!(field("api_secret").fieldtype, "Password");
    assert_eq!(field("api_secret").permlevel, 1);
    assert!(field("api_secret").hidden);

    for name in ["mfa_secret", "mfa_backup_codes"] {
        assert_ne!(
            field(name).fieldtype,
            "Password",
            "{name} must stay readable"
        );
        assert_eq!(field(name).permlevel, 1, "{name}");
        assert!(field(name).hidden, "{name}");
    }
    for name in ["api_key", "reset_password_key", "locked_until"] {
        assert_eq!(field(name).permlevel, 1, "{name}");
        assert!(field(name).hidden, "{name}");
    }
}

/// `DocPerm` is still an engine document. `Session` is **not**: sessions live
/// in `tab_session_store` and are owned by `tower-sessions`, so declaring the
/// old DocType would be a second store.
#[test]
fn docperm_is_declared_and_the_old_session_doctype_is_gone() {
    register_doctypes();

    let docperm = get_doctype("DocPerm").expect("DocPerm registered");
    assert!(docperm.fields.iter().any(|f| f.fieldname == "permlevel"));
    assert!(docperm.fields.iter().any(|f| f.fieldname == "if_owner"));
    assert!(
        docperm.hidden,
        "DocPerm is infrastructure, not a workspace item"
    );

    assert!(
        get_doctype("Session").is_none(),
        "the custom Session DocType must not come back alongside tab_session_store"
    );
}

/// Roles are the `User.roles` JSON array; no child table grants a role. The
/// `Has Role` table is kept only as the `Workspace.roles` target, so it must be
/// declared (the registry and the reserved-name set expect it) and must grant
/// nothing.
#[test]
fn user_roles_json_is_the_only_role_grant() {
    register_doctypes();
    let meta = user_meta();
    let roles = meta
        .fields
        .iter()
        .find(|f| f.fieldname == "roles")
        .expect("roles field");
    assert_eq!(roles.fieldtype, "Small Text");
    assert!(
        !meta
            .fields
            .iter()
            .any(|f| f.fieldtype == "Table" || f.fieldtype == "Table MultiSelect"),
        "no User child table may grant a role"
    );

    let has_role = get_doctype("Has Role").expect("Has Role is the Workspace child table");
    assert!(has_role.is_child_table);
    assert!(
        has_role.permissions.is_empty(),
        "a child table carries no permissions, and Has Role must not grant access"
    );
}

/// `Access Log` is written by the host (`ferris-files` emits the event and the
/// server's sink inserts the row), so it stays declared and visible rather than
/// being pruned as dead metadata.
#[test]
fn access_log_is_declared_for_the_host_writer() {
    register_doctypes();
    let meta = get_doctype("Access Log").expect("Access Log registered");
    assert!(meta.read_only);
    assert!(
        !meta.hidden,
        "Access Log is a live workspace log, not hidden infrastructure"
    );
}

/// `API Key` is a Ferris extension (Frappe keeps keys on `User`); its secret is
/// a SHA-256 digest stored as `Data` at `permlevel` 1, `key` is unique, and the
/// table is hidden.
#[test]
fn api_key_secret_is_a_hidden_data_digest_admin_only() {
    let meta = api_key_meta();
    assert!(
        meta.hidden,
        "API Key is infrastructure, not a workspace item"
    );
    let secret = meta
        .fields
        .iter()
        .find(|f| f.fieldname == "secret_hash")
        .expect("secret_hash field");
    assert_eq!(
        secret.fieldtype, "Data",
        "the manager writes a digest; a Password field would hash it again"
    );
    assert!(secret.read_only);
    assert!(secret.reqd);
    assert_eq!(secret.permlevel, 1);
    assert!(secret.hidden);

    assert!(
        meta.indexes
            .iter()
            .any(|index| index.unique && index.fields == ["key"]),
        "the public key identifier must be unique"
    );
}
