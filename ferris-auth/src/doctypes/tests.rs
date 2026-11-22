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

/// The `DocPerm` and `Session` rows the stores persist are now engine
/// documents, and the session's CSRF token is a credential.
#[test]
fn docperm_and_session_are_declared_with_properties() {
    register_doctypes();

    let docperm = get_doctype("DocPerm").expect("DocPerm registered");
    assert!(docperm.fields.iter().any(|f| f.fieldname == "permlevel"));
    assert!(docperm.fields.iter().any(|f| f.fieldname == "if_owner"));
    assert!(
        docperm.hidden,
        "DocPerm is infrastructure, not a workspace item"
    );

    let session = get_doctype("Session").expect("Session registered");
    assert!(
        session.hidden,
        "Session is infrastructure, not a workspace item"
    );
    let csrf = session
        .fields
        .iter()
        .find(|f| f.fieldname == "csrf_token")
        .expect("csrf_token field");
    assert_eq!(csrf.permlevel, 1);
    assert!(csrf.hidden);
}

/// `API Key` is a Ferris extension (Frappe keeps keys on `User`); its secret
/// is a `Password` credential at `permlevel` 1 and the table is hidden.
#[test]
fn api_key_secret_is_a_password_field_admin_only() {
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
    assert_eq!(secret.fieldtype, "Password");
    assert_eq!(secret.permlevel, 1);
    assert!(secret.hidden);
}
