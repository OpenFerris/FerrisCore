//! Engine tests, split by concern.
//!
//! The suite was one 2460-line file, which the repository's size rule
//! (`AGENTS.md` §4) does not allow — a `#[cfg(test)]` module is explicitly not
//! an exception. It is a folder now; this file holds the shared prelude and the
//! fixtures every group needs (`engine`, the two users, `field`, `vault_meta`,
//! `install_test_hasher`), and re-exports the framework types so each group only
//! writes `use super::*;` instead of a per-file import block that drifts.
//!
//! Groups: [`crud`] (insert/update/delete/`apply`), [`list`] (paging and the SQL
//! pushdown), [`permissions`] (the authorization matrix), [`fields`] (permlevel
//! and password masking), [`single`] (Single DocTypes), [`naming`] (name
//! strategies), [`runtime`] (workflow gate, version history, shares).

use super::support::*;
use super::*;

// The framework types the groups use. `pub(crate) use` so `use super::*;` in a
// group file picks them up; a glob import never raises `unused_imports`, which
// is why the groups stay warning-free without a per-file header.
pub(crate) use crate::list_sql;
pub(crate) use ferris_core::hooks::{
    DocStatus, DocumentHook, HookContext, HookPoint, HookRegistry,
};
pub(crate) use ferris_core::permissions::Permission;
pub(crate) use ferris_core::{DocumentError, Error, SecretError, ValidationError, Value};
pub(crate) use ferris_db::Stmt;
pub(crate) use ferris_schema::DoctypeMeta;
pub(crate) use std::collections::HashMap;

mod crud;
mod fields;
mod list;
mod naming;
mod permissions;
mod runtime;
mod search;
mod single;
mod trash;
mod typed;

async fn engine() -> DocumentEngine {
    register_common_doctypes();
    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    DocumentEngine::new(db)
}

/// Register the ad-hoc DocTypes the engine tests drive.
///
/// Columns are the only source of truth (ADR 0001), so an unregistered name
/// cannot store a declared field. These names are test fixtures, not part of
/// `ferris-schema`'s catalogue; registering them once per process gives them
/// the columns their bodies use. Idempotent.
pub(crate) fn register_common_doctypes() {
    let simple = |name: &str, fields: &[&str]| DoctypeMeta {
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|field| field_meta(field, "Data", 0))
            .collect(),
        ..Default::default()
    };
    ferris_schema::register_doctype(DoctypeMeta {
        name: "ToDo".to_string(),
        is_submittable: true,
        fields: vec![
            field_meta("title", "Data", 0),
            field_meta("status", "Data", 0),
            field_meta("description", "Text", 0),
            field_meta("edited", "Data", 0),
            field_meta("first", "Data", 0),
            field_meta("last", "Data", 0),
            field_meta("urgent", "Check", 0),
            field_meta("code", "Data", 0),
            field_meta("changed", "Data", 0),
            field_meta("loot", "Data", 0),
            field_meta("hijacked", "Data", 0),
            field_meta("note", "Data", 0),
            ferris_schema::FieldMeta {
                fieldname: "paid_amount".to_string(),
                fieldtype: "Currency".to_string(),
                allow_on_submit: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    for (name, fields) in [
        ("Note", &["title", "body"][..]),
        ("NestedSealProbe", &["title", "sealed"][..]),
        ("Shared", &["title"][..]),
        ("ListShared", &["title"][..]),
        ("History", &["title"][..]),
        ("Versioned", &["title"][..]),
        ("Masked", &["title", "secret", "password"][..]),
        ("AuthorizeFirstProbe", &["title"][..]),
        ("AuthorizeFirstUpdateProbe", &["title"][..]),
        ("SuppressProbe", &["title"][..]),
    ] {
        ferris_schema::register_doctype(simple(name, fields));
    }
}

fn field_meta(fieldname: &str, fieldtype: &str, permlevel: u8) -> ferris_schema::FieldMeta {
    ferris_schema::FieldMeta {
        fieldname: fieldname.to_string(),
        fieldtype: fieldtype.to_string(),
        permlevel,
        ..Default::default()
    }
}

fn sysadmin() -> ferris_core::User {
    let mut user = ferris_core::User::system_user("admin@local".to_string(), "Admin".to_string());
    // `User::new` mints a UUID for `id`; pin it so the assertions below can
    // name the owner. `plain_user` already did this.
    user.id = "admin@local".to_string();
    user.roles = vec!["System Manager".to_string()];
    user
}

fn plain_user() -> ferris_core::User {
    let mut user = ferris_core::User::system_user("alice@local".to_string(), "Alice".to_string());
    user.id = "alice@local".to_string();
    user
}

fn denied(err: &Error) -> bool {
    matches!(
        err,
        Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
    )
}

fn field(fieldname: &str, fieldtype: &str, permlevel: u8) -> ferris_schema::FieldMeta {
    ferris_schema::FieldMeta {
        fieldname: fieldname.to_string(),
        fieldtype: fieldtype.to_string(),
        permlevel,
        ..Default::default()
    }
}

/// A `title` any caller may read/write, an `admin_note` at level 1, and a
/// level-0 `Password`.
fn vault_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "Vault".to_string(),
        fields: vec![
            field("title", "Data", 0),
            field("admin_note", "Data", 1),
            field("password", "Password", 0),
        ],
        ..Default::default()
    }
}

/// A deterministic stand-in for Argon2; `ferris-auth` installs the real hasher
/// at boot, which this crate (by design) does not depend on.
fn install_test_hasher() {
    ferris_core::password::set_password_hasher(|plain| Ok(format!("hashed:{plain}")));
}

/// A fixed 32-byte AES-256-GCM key for `Secret`-field tests.
pub(crate) const TEST_SECRET_KEY: &str =
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

/// Install the AEAD keyring the `Secret` write path requires.
///
/// Idempotent (the first ring wins, the same material re-installs): every test
/// wants this one key, and `OnceLock` cannot be reset between tests.
fn install_test_keyring() {
    let key = ferris_core::secret::SecretKey::parse(TEST_SECRET_KEY).expect("32-byte test key");
    let _ = ferris_core::secret::install_keyring(ferris_core::secret::SecretKeyring::new(
        key,
        Vec::new(),
    ));
}

/// `vault_meta` plus a `Secret` field, so the two credential types can be
/// asserted side by side.
fn secret_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "VaultS".to_string(),
        fields: vec![
            field("title", "Data", 0),
            field("password", "Password", 0),
            field("client_secret", "Secret", 0),
            field("admin_secret", "Secret", 1),
        ],
        ..Default::default()
    }
}

/// A process-wide `User Permission` source for tests.
///
/// The source is global and the tests that swap it must not run in parallel, so
/// the guard both installs the rules and holds the serialisation lock until it
/// is dropped. `read` and `permissions` share it, which is why it lives here
/// rather than inside one test module.
pub(crate) mod user_permission_source {
    use ferris_core::permissions::{
        UserPermissionRule, UserPermissionRules, UserPermissionSource,
        clear_user_permission_source, set_user_permission_source,
    };
    use std::sync::{Arc, Mutex, MutexGuard};

    struct Static(Vec<UserPermissionRule>);

    impl UserPermissionSource for Static {
        fn rules_for(&self, user: &str) -> UserPermissionRules {
            UserPermissionRules::from(
                self.0
                    .iter()
                    .filter(|rule| rule.user == user)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }
    }

    static SERIALISE: Mutex<()> = Mutex::new(());

    pub(crate) struct Installed {
        _guard: MutexGuard<'static, ()>,
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            clear_user_permission_source();
        }
    }

    pub(crate) fn install(rules: Vec<UserPermissionRule>) -> Installed {
        let guard = SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_user_permission_source(Arc::new(Static(rules)));
        Installed { _guard: guard }
    }
}
