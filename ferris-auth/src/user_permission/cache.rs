//! Background freshness for the `User Permission` snapshot.
//!
//! The engine hooks in [`super::register_hooks`] refresh the snapshot after a
//! write this process made. A rule written by **another replica** never runs
//! those hooks, so without a periodic reload the snapshot would stay stale
//! until the next local edit or a restart. [`spawn_background_refresh`] closes
//! that window at boot; [`refresh_if_stale`] exposes the same bounded-staleness
//! check to a caller that wants to force a reload before a privileged read.
//!
//! Both are best-effort: a failed reload logs and keeps the previous snapshot,
//! because stale rules keep restricting access while a cleared snapshot would
//! silently widen it.

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use ferris_db::Db;
use tokio::task::JoinHandle;

use super::refresh;

/// How often the background task reloads the snapshot.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// When the snapshot was last rebuilt successfully. Monotonic and
/// process-local: it only bounds how often this replica queries again.
static LAST_REFRESH: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));

/// The task spawned by the last [`spawn_background_refresh`] call.
static BACKGROUND: LazyLock<Mutex<Option<JoinHandle<()>>>> = LazyLock::new(|| Mutex::new(None));

/// Record a successful rebuild. Called by [`super::refresh`] only after the new
/// snapshot is published, so a failed load keeps the previous timestamp.
pub(super) fn mark_refreshed() {
    *LAST_REFRESH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
}

/// Forget the timestamp together with the snapshot it describes.
pub(super) fn forget_refreshed() {
    *LAST_REFRESH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Reload the snapshot unless it was rebuilt less than `max_age` ago.
///
/// Returns whether a reload ran. A caller with a privileged operation on the
/// request path can pass the interval it is willing to tolerate, so the reload
/// costs one query at most once per window instead of one per request.
///
/// # Errors
///
/// Whatever [`refresh`] returns; the previous snapshot is kept.
#[must_use = "the caller decides whether a reload was needed"]
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
pub async fn refresh_if_stale(db: &Db, max_age: Duration) -> ferris_core::Result<bool> {
    let is_fresh = LAST_REFRESH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some_and(|at| at.elapsed() < max_age);
    if is_fresh {
        return Ok(false);
    }
    refresh(db).await?;
    Ok(true)
}

/// Spawn the periodic reload, or do nothing when there is no runtime.
///
/// `install` calls this after a successful load. A test or an embedder that
/// installs the store without a Tokio runtime is a no-op instead of a panic:
/// the hooks and [`refresh_if_stale`] still work, only the timer does not.
///
/// The task replaces the one a previous install spawned, so a role-permission
/// edit (which re-installs) does not stack reloads.
pub fn spawn_background_refresh(db: &Db) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::debug!(
            "user-permission: no Tokio runtime; the rule cache will only refresh on demand"
        );
        return;
    };
    let db = db.clone();
    let task = handle.spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick is immediate and `install` already loaded the rules,
        // so the first useful reload is one interval away.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = refresh(&db).await {
                tracing::warn!(
                    "user-permission: background refresh failed; keeping the previous snapshot: {error}"
                );
            }
        }
    });
    let mut slot = BACKGROUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(previous) = slot.replace(task) {
        previous.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_permission::tests::guard;

    /// A fresh snapshot is not loaded twice inside the window, and a stale one
    /// is reloaded once.
    #[tokio::test]
    async fn refresh_if_stale_reloads_at_most_once_per_window() {
        let _guard = guard();
        let db = ferris_db::memory_db().await.unwrap();

        assert!(
            refresh_if_stale(&db, REFRESH_INTERVAL).await.unwrap(),
            "an unmarked snapshot must be loaded"
        );
        assert!(
            !refresh_if_stale(&db, REFRESH_INTERVAL).await.unwrap(),
            "a snapshot younger than the window must not be loaded again"
        );
        assert!(
            refresh_if_stale(&db, Duration::ZERO).await.unwrap(),
            "a zero window always reloads"
        );
    }

    /// Without a runtime the spawn is a no-op, not a panic: boot-time install
    /// and tests without Tokio must keep working.
    #[test]
    fn spawning_without_a_runtime_is_a_no_op() {
        let _guard = guard();
        let db = {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(ferris_db::memory_db()).unwrap()
        };
        assert!(tokio::runtime::Handle::try_current().is_err());
        spawn_background_refresh(&db);
    }
}
