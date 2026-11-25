//! Login rate limiting and the persisted account lockout.
//!
//! [`RateLimiter`] is the process-local failure counter; [`AccountLockout`] and
//! the free functions are the persisted half, stored on the `User` row so a
//! restart or a second replica reads the same deadline (AUTH-04/AUTH-16).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// Rate limiting for auth endpoints.
///
/// Fail-counting rather than the old coarse check-per-attempt: callers ask
/// [`RateLimiter::is_blocked`] before validating credentials, then report the
/// outcome with [`RateLimiter::record_failure`] / [`RateLimiter::record_success`]
/// so only **failed** attempts count towards a lockout and a successful login
/// clears the key. The window is the configured lockout duration.
#[derive(Clone)]
pub struct RateLimiter {
    attempts: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    max_attempts: u32,
    window_seconds: u64,
}

impl RateLimiter {
    pub fn new(max_attempts: u32, window_seconds: u64) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            max_attempts,
            window_seconds,
        }
    }

    /// Whether `key` has exhausted its attempts inside the window.
    ///
    /// Prunes entries whose window elapsed as a side effect, so an expired
    /// lockout drops out on its own.
    #[must_use]
    pub fn is_blocked(&self, key: &str) -> bool {
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        attempts.retain(|_, (_, timestamp)| {
            now.duration_since(*timestamp).as_secs() < self.window_seconds
        });
        attempts
            .get(key)
            .is_some_and(|(count, _)| *count >= self.max_attempts)
    }

    /// Record a failed attempt, extending the lockout window.
    pub fn record_failure(&self, key: &str) {
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        attempts.retain(|_, (_, timestamp)| {
            now.duration_since(*timestamp).as_secs() < self.window_seconds
        });
        let entry = attempts.entry(key.to_string()).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    /// Clear `key` after a successful authentication.
    pub fn record_success(&self, key: &str) {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    /// The configured failure budget, so a caller can persist the same policy
    /// it enforces in memory (AUTH-04).
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// The configured lockout duration, in minutes.
    #[must_use]
    pub const fn lockout_minutes(&self) -> u64 {
        self.window_seconds / 60
    }
}

/// The persisted half of the login lockout (AUTH-04/AUTH-16).
///
/// [`RateLimiter`] is process-local, so a restart or a second replica forgets
/// every failure. The `User` document already declares
/// `failed_login_attempts` and `locked_until`, so the account half of the
/// lockout is stored on the row the login is about: the counter survives a
/// restart, every replica reads the same deadline, and the Desk can show why
/// the account is refusing attempts.
///
/// The storage lives in [`crate::UserStore`] (`account_lockout`,
/// `record_failed_login`, `clear_lockout`); the free functions below are the
/// stable surface for callers that only hold a [`ferris_db::Db`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountLockout {
    pub failed_login_attempts: u32,
    pub locked_until: Option<chrono::DateTime<chrono::Utc>>,
}

impl AccountLockout {
    /// Whether the account is locked as of `now`.
    #[must_use]
    pub fn is_locked(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.locked_until.is_some_and(|until| until > now)
    }
}

/// Read the persisted lockout of `user_id`; a missing row reads as "unlocked".
///
/// # Errors
///
/// Storage or document-decoding errors.
pub async fn account_lockout(
    db: &ferris_db::Db,
    user_id: &str,
) -> Result<AccountLockout, ferris_core::Error> {
    crate::UserStore::new(db.clone())
        .account_lockout(user_id)
        .await
}

/// Record one failed login against the user row.
///
/// Delegates to [`crate::UserStore::record_failed_login`], whose single
/// conditional `UPDATE` makes the increment atomic; the counter and the
/// deadline follow the same rule as [`RateLimiter`] and
/// [`ferris_core::User::record_failed_login`], so the in-memory and persisted
/// answers cannot disagree.
///
/// # Errors
///
/// Storage errors.
pub async fn record_account_failure(
    db: &ferris_db::Db,
    user_id: &str,
    max_attempts: u32,
    lockout_minutes: u64,
) -> Result<AccountLockout, ferris_core::Error> {
    crate::UserStore::new(db.clone())
        .record_failed_login(user_id, max_attempts, lockout_minutes)
        .await
}

/// Clear the persisted lockout after the **whole** login succeeded.
///
/// One write: [`crate::UserStore::clear_lockout`] resets both columns in place,
/// with no read-modify-write to race.
///
/// # Errors
///
/// Storage errors.
pub async fn clear_account_lockout(
    db: &ferris_db::Db,
    user_id: &str,
) -> Result<(), ferris_core::Error> {
    crate::UserStore::new(db.clone())
        .clear_lockout(user_id)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(3, 60);
        assert!(!limiter.is_blocked("test_key"));

        // Three failures cross the threshold and block the key.
        limiter.record_failure("test_key");
        limiter.record_failure("test_key");
        assert!(!limiter.is_blocked("test_key"));
        limiter.record_failure("test_key");
        assert!(limiter.is_blocked("test_key"));

        // A success clears the key again.
        limiter.record_success("test_key");
        assert!(!limiter.is_blocked("test_key"));

        // Unrelated keys are never blocked by someone else's failures.
        limiter.record_failure("other_key");
        assert!(!limiter.is_blocked("test_key"));
    }

    /// AUTH-04/AUTH-16: the account lockout is written to the user row, so a
    /// restart or another replica reads the same deadline instead of starting
    /// from zero.
    #[tokio::test]
    async fn the_account_lockout_is_persisted_on_the_user() {
        use chrono::Utc;

        let db = ferris_db::memory_db().await.unwrap();
        let store = crate::UserStore::new(db.clone());
        store.init().await.unwrap();
        let user = store
            .create(
                "lockout@example.com",
                "Lockout",
                None,
                crate::UserType::SystemUser,
                "Secret-Password1",
                &["User".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(
            account_lockout(&db, &user.id).await.unwrap(),
            AccountLockout::default(),
            "a fresh account is not locked"
        );

        let first = record_account_failure(&db, &user.id, 2, 15).await.unwrap();
        assert_eq!(first.failed_login_attempts, 1);
        assert!(!first.is_locked(Utc::now()));

        let second = record_account_failure(&db, &user.id, 2, 15).await.unwrap();
        assert!(second.is_locked(Utc::now()), "the budget is exhausted");

        // Reading again is what a restarted process does.
        assert!(
            account_lockout(&db, &user.id)
                .await
                .unwrap()
                .is_locked(Utc::now())
        );

        clear_account_lockout(&db, &user.id).await.unwrap();
        assert_eq!(
            account_lockout(&db, &user.id).await.unwrap(),
            AccountLockout::default(),
            "a completed login clears the counter"
        );
    }
}
