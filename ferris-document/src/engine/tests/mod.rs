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
mod single;
mod trash;

async fn engine() -> DocumentEngine {
    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    DocumentEngine::new(db)
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
