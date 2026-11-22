//! Shared fixtures for the authorization tests.

use ferris_auth::permissions::{
    PermissionEngine, SkippedRule, global_engine_ref, set_global_engine,
};

use super::super::{PolicySource, install_policies};

/// A test DocType.
///
/// `hidden: true` because the schema registry is process-wide and has no
/// deregister: a visible fixture would appear in the Desk sidebar. It is not
/// enough for the permission engine to ignore it — `auto_modules` in
/// `desk.rs` reads the same registry and builds the workspace out of it, so
/// an unhidden fixture fails a test in another module.
pub(super) fn meta(
    name: &str,
    perms: Vec<ferris_schema::PermissionRule>,
) -> ferris_schema::DoctypeMeta {
    ferris_schema::DoctypeMeta {
        name: name.to_string(),
        module: "Test".to_string(),
        permissions: perms,
        hidden: true,
        ..Default::default()
    }
}

pub(super) fn rule(
    role: &str,
    read: bool,
    write: bool,
    delete: bool,
) -> ferris_schema::PermissionRule {
    ferris_schema::PermissionRule {
        role: role.to_string(),
        read,
        write,
        delete,
        ..Default::default()
    }
}

/// Serialise the tests in this module against each other and restore the
/// engine afterwards.
///
/// The engine is process-wide, so these tests would otherwise see each
/// other's installs. The lock here is a *separate* mutex, not the engine's:
/// `install_policies` takes the engine's own write lock, so holding that one
/// across the body would deadlock the test against itself.
static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Holds the module lock and restores the engine on drop.
///
/// A guard rather than a closure wrapper, because the tests that go through
/// the document engine are `async` and need the scope to live across an
/// `.await`. A wrapper that restored on return would restore *before* the
/// future was ever polled, which is exactly the bug this shape exists to
/// avoid.
pub(super) struct EngineScope {
    _serialised: std::sync::MutexGuard<'static, ()>,
    previous: PermissionEngine,
}

impl Drop for EngineScope {
    fn drop(&mut self) {
        set_global_engine(self.previous.clone());
    }
}

pub(super) fn engine_scope() -> EngineScope {
    let serialised = SERIALISE.lock().unwrap_or_else(|e| e.into_inner());
    let previous = global_engine_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    EngineScope {
        _serialised: serialised,
        previous,
    }
}

pub(super) fn with_engine<R>(f: impl FnOnce() -> R) -> R {
    let _scope = engine_scope();
    f()
}

/// Register a DocType and install, reporting only what this registration added.
///
/// `list_doctypes_metas` walks the process-wide registry, which has no
/// deregister and which other tests in this binary write to concurrently — the
/// `panel` and `store` tests call `register_core_doctypes` on every request path
/// they exercise. So an absolute count is meaningless here, and even a
/// before/after delta over *every* DocType races with another thread's
/// registration.
///
/// What is safe is the delta over statements and skipped rules, because the only
/// registrations carrying `permissions` are this module's and the core DocTypes',
/// and the core ones are present in both halves of the delta.
pub(super) fn install_after(name: &str, perms: Vec<ferris_schema::PermissionRule>) -> AddedRules {
    let before = install_policies(&PolicySource::default());
    ferris_schema::register_doctype(meta(name, perms));
    let after = install_policies(&PolicySource::default());
    AddedRules {
        statements: after.statements - before.statements,
        skipped: after
            .skipped
            .iter()
            .filter(|s| !before.skipped.contains(s))
            .cloned()
            .collect(),
    }
}

/// What one registration contributed. See [`install_after`] on why the
/// registry-wide counts are absent.
#[derive(Debug, Default)]
pub(super) struct AddedRules {
    pub(super) statements: usize,
    pub(super) skipped: Vec<SkippedRule>,
}

pub(super) fn service_user() -> ferris_auth::User {
    let mut user =
        ferris_auth::User::system_user("system@ferris.local".to_string(), "System".to_string());
    user.roles = vec!["System Manager".to_string()];
    user
}

pub(super) fn user(id: &str, roles: &[&str]) -> ferris_auth::User {
    ferris_auth::User {
        id: id.to_string(),
        roles: roles.iter().map(|r| r.to_string()).collect(),
        ..Default::default()
    }
}

pub(super) fn doc(doctype: &str, name: &str, owner: &str) -> ferris_auth::permissions::Resource {
    ferris_auth::permissions::Resource::new(doctype, name).with_owner(owner)
}

pub(super) fn shared_event(title: &str) -> ferris_core::Value {
    ferris_core::Value::from_json(serde_json::json!({
        "title": title,
        "starts_on": "2030-01-01T09:00:00Z",
    }))
}
