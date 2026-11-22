//! Shared plumbing for the stores that persist through the document engine.
//!
//! The framework's own tables (`DocPerm`, `Session`, `API Key`, and the `User`
//! credentials) are documents like any other: the engine owns their lifecycle
//! and is the single source of truth for reads and writes. The stores are
//! *trusted framework callers* — they mint sessions and issue keys on behalf of
//! a request that has no authenticated user yet — so they act as `System
//! Manager`, the one principal the engine lets reach every document and every
//! field level. That is also why they never re-implement validation, hooks or
//! field redaction: the engine does it, once.

use ferris_core::User;
use ferris_core::permissions::SYSTEM_MANAGER_ROLE;
use ferris_document::EngineContext;
use ferris_schema::DoctypeMeta;

/// The identity the framework's stores act as.
///
/// `System Manager` is the documented bypass in
/// [`ferris_core::permissions::check_permission`] and
/// [`ferris_core::permissions::has_permlevel_access`], so a store reaches the
/// `Password` fields (the `User` hash, the API secret) it has to verify against
/// and writes rows no ordinary caller may.
#[must_use]
pub(crate) fn system_user() -> User {
    let mut user = User::system_user("Administrator".to_string(), "Administrator".to_string());
    user.roles = vec![SYSTEM_MANAGER_ROLE.to_string()];
    user
}

/// An engine context for `meta`, acting as the framework.
///
/// The naming series is left empty: every store supplies the document `name`
/// itself (a user's email, a session id, an API key's `(doctype, role,
/// permlevel)` key), so the engine's series generator is never consulted.
#[must_use]
pub(crate) fn system_context(meta: &DoctypeMeta) -> EngineContext<'_> {
    EngineContext::new(meta.name.clone(), "")
        .with_meta(meta)
        .with_user(&system_user())
        // Login must compare the submitted password with the stored Argon2
        // hash. This is the one trusted context that opts out of the read
        // mask; every request-derived context keeps it on.
        .reveal_password_hash()
}
