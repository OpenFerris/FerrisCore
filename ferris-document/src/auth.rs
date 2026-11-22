//! Ambient (task-scoped) user context.
//!
//! REST handlers, realtime connectors and job workers run their futures under
//! [`with_user`] so the metadata engine and the typed CRUD can authorise the
//! operation as the actual caller.
//!
//! The engine is **default-deny**: without an ambient user (or an explicit
//! `allow_anonymous` context) every write operation fails closed. The typed
//! CRUD helpers in [`crate::crud`] read the ambient user synchronously before
//! the first `await`, so the macro-generated `Document::insert/save/delete/...`
//! API keeps its signature while still being authorised.

use ferris_core::User;

tokio::task_local! {
    /// User currently executing the task, set by framework entry points.
    pub static CURRENT_USER: User;
}

/// Run `fut` with `user` as the ambient task user.
///
/// `None` means anonymous — engine operations will be denied (fails closed).
pub async fn with_user<T, F>(user: Option<User>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    if let Some(user) = user {
        CURRENT_USER.scope(user, fut).await
    } else {
        fut.await
    }
}

/// Synchronous read of the current ambient user.
///
/// Safe to call between `await` points (it does not touch the task-local after
/// returning an owned value). Returns `None` when no user is scoped.
pub fn current_user() -> Option<User> {
    CURRENT_USER.try_with(|u| u.clone()).ok()
}
