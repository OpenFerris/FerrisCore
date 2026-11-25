//! Login rate limiting and the persisted account lockout.
//!
//! [`RateLimiter`] is the process-local failure counter; [`AccountLockout`] and
//! the free functions are the persisted half, stored on the `User` row so a
//! restart or a second replica reads the same deadline (AUTH-04/AUTH-16).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// Upper bound on the keys one limiter tracks.
///
/// The key can be attacker-influenced (`reset:<email>` is one entry per address
/// a reset is requested for; a login failure records the connection address), so
/// an unbounded map would let a burst of distinct keys grow this process's
/// memory without limit. Beyond the cap, expired entries are dropped first and
/// the least recently touched one afterwards.
const MAX_TRACKED_KEYS: usize = 10_000;

/// Rate limiting for auth endpoints.
///
/// Fail-counting rather than the old coarse check-per-attempt: callers ask
/// [`RateLimiter::is_blocked`] before validating credentials, then report the
/// outcome with [`RateLimiter::record_failure`] / [`RateLimiter::record_success`]
/// so only **failed** attempts count towards a lockout and a successful login
/// clears the key. The window is the configured lockout duration.
///
/// Both operations are O(1) on the hot path: the window is evaluated against the
/// key's own timestamp instead of sweeping the map on every attempt, and the map
/// is pruned only when a new key would exceed [`MAX_TRACKED_KEYS`].
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
    /// The entry's own timestamp decides, so an expired lockout no longer blocks
    /// the key even though the row is still in the map (it is reclaimed the next
    /// time the map needs room).
    #[must_use]
    pub fn is_blocked(&self, key: &str) -> bool {
        let attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        attempts.get(key).is_some_and(|(count, timestamp)| {
            *count >= self.max_attempts
                && now.duration_since(*timestamp).as_secs() < self.window_seconds
        })
    }

    /// Record a failed attempt, extending the lockout window.
    pub fn record_failure(&self, key: &str) {
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if attempts.len() >= MAX_TRACKED_KEYS && !attempts.contains_key(key) {
            // Reclaim expired rows before evicting a live lockout: a burst of
            // abandoned keys then costs nothing beyond the sweep itself.
            attempts.retain(|_, (_, timestamp)| {
                now.duration_since(*timestamp).as_secs() < self.window_seconds
            });
            if attempts.len() >= MAX_TRACKED_KEYS
                && let Some(oldest) = attempts
                    .iter()
                    .min_by_key(|(_, (_, timestamp))| *timestamp)
                    .map(|(key, _)| key.clone())
            {
                attempts.remove(&oldest);
            }
        }
        let entry = attempts.entry(key.to_string()).or_insert((0, now));
        // A failure after the window started a new one for this key: the old
        // count is spent, exactly as if the entry had been pruned. Without this
        // reset a key that once exhausted its budget would stay locked forever,
        // because a blocked key never records a success.
        if now.duration_since(entry.1).as_secs() >= self.window_seconds {
            entry.0 = 0;
        }
        entry.0 = entry.0.saturating_add(1);
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

    /// The window is evaluated per key, so a lockout stops blocking once it
    /// expires even though the entry is still in the map.
    #[tokio::test]
    async fn an_expired_window_releases_the_key() {
        let limiter = RateLimiter::new(1, 0);
        limiter.record_failure("test_key");
        // Zero-second window: the entry is instantly outside it.
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(!limiter.is_blocked("test_key"));
    }

    /// A failure after the window restarts the count: a key that once hit the
    /// budget is not locked forever, since a blocked key never sees a success.
    #[tokio::test]
    async fn a_lapsed_window_restarts_the_count() {
        let limiter = RateLimiter::new(2, 0);
        limiter.record_failure("test_key");
        limiter.record_failure("test_key");
        // A zero-second window has already lapsed, so the next failure opens a
        // new window instead of continuing the exhausted one.
        std::thread::sleep(std::time::Duration::from_millis(1));
        limiter.record_failure("test_key");
        assert_eq!(
            limiter
                .attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get("test_key")
                .map(|(count, _)| *count),
            Some(1),
            "the new window starts from one failure"
        );
    }

    /// The map is bounded: an attacker varying the key (one reset per address,
    /// one login per spoofed avenue) must not grow this process's memory.
    #[tokio::test]
    async fn the_tracked_key_set_is_bounded() {
        let limiter = RateLimiter::new(3, 60);
        for index in 0..(MAX_TRACKED_KEYS + 500) {
            limiter.record_failure(&format!("key-{index}"));
        }
        let tracked = limiter
            .attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        assert!(
            tracked <= MAX_TRACKED_KEYS,
            "the limiter tracked {tracked} keys, above the {MAX_TRACKED_KEYS} cap"
        );
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
