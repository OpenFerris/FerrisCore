//! [`SessionManager`](super::SessionManager) lifecycle and cache surface.
//!
//! The manager is a cache-then-engine facade: every read goes through the
//! private `load_session` (memory first, then the `Session` document), and every
//! write refreshes the cache and persists through the engine. The engine is the
//! single source of truth, so a second manager with an empty cache sees the same
//! rows.

use chrono::{DateTime, Utc};
use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_document::ListQuery;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::crypto::constant_time_eq;
use crate::store_support::system_context;

use super::{Session, SessionManager, TOUCH_PERSIST_INTERVAL_SECS};

/// When a session stopped being usable because it was revoked.
///
/// A signed access token names its `session_id`, and [`crate::JwtManager`] only
/// proves the signature: nothing in the token changes when the session is
/// deleted. `logout` therefore has to leave a trace the request path can see
/// (AUTH-05). With a database the deleted `Session` row already is that trace;
/// this map covers a pool-less install and the window before the next lookup
/// misses the cache. Entries expire with the session that named them, so the
/// map cannot grow past the set of tokens still worth rejecting.
static REVOKED_SESSIONS: LazyLock<parking_lot::RwLock<HashMap<String, DateTime<Utc>>>> =
    LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

/// Drop revocation entries whose tokens can no longer be presented.
fn prune_revoked(sessions: &mut HashMap<String, DateTime<Utc>>) {
    let now = Utc::now();
    sessions.retain(|_, expires_at| *expires_at > now);
}

fn remember_revoked(session_id: &str, expires_at: DateTime<Utc>) {
    let mut sessions = REVOKED_SESSIONS.write();
    prune_revoked(&mut sessions);
    sessions.insert(session_id.to_string(), expires_at);
}

fn is_revoked(session_id: &str) -> bool {
    let mut sessions = REVOKED_SESSIONS.write();
    prune_revoked(&mut sessions);
    sessions.contains_key(session_id)
}

impl SessionManager {
    /// Insert-or-update a session as a document.
    pub(super) async fn persist(&self, session: &Session) -> Result<()> {
        let Some(engine) = &self.engine else {
            return Ok(());
        };
        let meta = crate::doctypes::session_meta();
        let ctx = system_context(&meta);
        let document = session.to_document();
        match engine.get(&ctx, &session.id).await {
            Ok(_) => {
                engine.update(&ctx, &session.id, document).await?;
            }
            Err(Error::Document(DocumentError::NotFound { .. })) => {
                engine.create(&ctx, document).await?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    async fn delete_persisted(&self, session_id: &str) -> Result<()> {
        let Some(engine) = &self.engine else {
            return Ok(());
        };
        let meta = crate::doctypes::session_meta();
        let ctx = system_context(&meta);
        match engine.delete(&ctx, session_id).await {
            Ok(()) | Err(Error::Document(DocumentError::NotFound { .. })) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Every persisted session for one user, oldest first.
    async fn persisted_user_sessions(&self, user_id: &str) -> Result<Vec<Session>> {
        let Some(engine) = &self.engine else {
            return Ok(Vec::new());
        };
        let meta = crate::doctypes::session_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            filters: HashMap::from([("user_id".to_string(), Value::from(user_id))]),
            limit: Some(u32::MAX),
            ..ListQuery::default()
        };
        let (documents, _) = engine.list(&ctx, &query).await?;
        let mut sessions: Vec<Session> = documents
            .iter()
            .map(Session::from_document)
            .collect::<Result<_>>()?;
        sessions.sort_by_key(|s| s.created_at);
        Ok(sessions)
    }

    /// Load a session from the cache, falling back to the database.
    async fn load_session(&self, session_id: &str) -> Result<Option<Session>> {
        if let Some(session) = self.sessions.read().get(session_id).cloned() {
            return Ok(Some(session));
        }
        let Some(engine) = &self.engine else {
            return Ok(None);
        };
        let meta = crate::doctypes::session_meta();
        let ctx = system_context(&meta);
        let session = match engine.get(&ctx, session_id).await {
            Ok(document) => Some(Session::from_document(&document)?),
            Err(Error::Document(DocumentError::NotFound { .. })) => None,
            Err(e) => return Err(e),
        };
        if let Some(s) = &session {
            self.sessions
                .write()
                .insert(session_id.to_string(), s.clone());
            // Record the loaded session in the per-user index too: a session
            // hydrated from the database used to live in `sessions` only, so a
            // later `delete_user_sessions` ("sign out everywhere") left it in
            // the cache and kept honouring it (AUTH-05).
            if !self
                .user_sessions
                .read()
                .get(&s.user_id)
                .is_some_and(|ids| ids.iter().any(|id| id == session_id))
            {
                self.user_sessions
                    .write()
                    .entry(s.user_id.clone())
                    .or_default()
                    .push(session_id.to_string());
            }
        }
        Ok(session)
    }

    /// Create a new session
    ///
    /// # Errors
    ///
    /// Storage errors from persisting the session.
    pub async fn create_session(
        &self,
        user_id: String,
        ip: Option<String>,
        user_agent: Option<String>,
    ) -> Result<Session> {
        let mut session = Session::new(user_id.clone(), &self.config);
        session.ip_address = ip;
        session.user_agent = user_agent;

        // Refuse the login when the user already holds the maximum: evicting an
        // older session would silently steal a tab that may be mid-edit. The
        // capacity check also sweeps this user's expired rows, so a login after
        // the timeout succeeds instead of counting a dead session forever
        // (AUTH-03).
        self.ensure_session_capacity(&user_id).await?;

        // Store session (memory + durable store)
        let session_id = session.id.clone();
        self.sessions
            .write()
            .insert(session_id.clone(), session.clone());
        self.user_sessions
            .write()
            .entry(user_id)
            .or_default()
            .push(session_id);
        self.persist(&session).await?;

        Ok(session)
    }

    /// Get session by ID
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        self.load_session(session_id).await
    }

    /// Get session and update activity.
    ///
    /// The in-memory session is refreshed on every call, but the **persisted**
    /// `last_activity` is only written when it has moved by at least
    /// [`TOUCH_PERSIST_INTERVAL_SECS`](super::TOUCH_PERSIST_INTERVAL_SECS)
    /// (AUTH14): a request-per-write `UPDATE` on the authenticated hot path is
    /// pure waste when the idle timeout is measured in minutes. The window is far
    /// smaller than the shortest configured idle timeout, so a session can never
    /// be kept alive by a write that would have been skipped.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn touch_session(&self, session_id: &str) -> Result<Option<Session>> {
        let Some(mut session) = self.load_session(session_id).await? else {
            return Ok(None);
        };
        if session.is_expired() || session.is_idle_expired(self.config.idle_timeout_minutes) {
            self.delete_session(session_id).await?;
            return Ok(None);
        }
        let previous_activity = session.last_activity;
        session.refresh(&self.config);
        let moved_enough = (session.last_activity - previous_activity).num_seconds()
            >= TOUCH_PERSIST_INTERVAL_SECS;
        self.sessions
            .write()
            .insert(session.id.clone(), session.clone());
        if moved_enough {
            self.persist(&session).await?;
        }
        Ok(Some(session))
    }

    /// Update session data
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Invalid`] for an unknown session, or a storage
    /// error.
    pub async fn update_session(
        &self,
        session_id: &str,
        data: HashMap<String, Value>,
    ) -> Result<()> {
        let Some(mut session) = self.load_session(session_id).await? else {
            return Err(Error::Validation(ValidationError::Invalid(
                "Session not found".to_string(),
            )));
        };
        session.data.extend(data);
        session.refresh(&self.config);
        self.sessions
            .write()
            .insert(session.id.clone(), session.clone());
        self.persist(&session).await?;
        Ok(())
    }

    /// Delete a session
    ///
    /// Revocation is durable even for a token that is still signed: the deleted
    /// row (and the in-process marker, see `REVOKED_SESSIONS`) makes
    /// [`SessionManager::session_allows_token`] answer `false`, so a bearer JWT
    /// minted for this session stops working (AUTH-05). The registered
    /// revocation listener is told as well, so the realtime engine closes the
    /// socket instead of draining it (RT-01).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let session = self.load_session(session_id).await?;
        self.sessions.write().remove(session_id);
        if let Some(session) = &session {
            let mut user_sessions = self.user_sessions.write();
            if let Some(user_sess) = user_sessions.get_mut(&session.user_id) {
                user_sess.retain(|id| id != session_id);
            }
        }
        if let Some(session) = &session {
            remember_revoked(&session.id, session.expires_at);
        }
        self.delete_persisted(session_id).await?;
        if let Some(session) = session {
            crate::middleware::notify_session_revoked(&session.user_id, Some(session_id));
        }
        Ok(())
    }

    /// Delete all sessions for a user
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn delete_user_sessions(&self, user_id: &str) -> Result<()> {
        if self.engine.is_some() {
            for session in self.persisted_user_sessions(user_id).await? {
                remember_revoked(&session.id, session.expires_at);
                self.delete_persisted(&session.id).await?;
            }
        }

        // Drop every cached session the user owns, not only the ids listed in
        // the per-user index: a session loaded from the database may predate
        // that index (AUTH-05).
        let cached: Vec<Session> = {
            let sessions = self.sessions.read();
            sessions
                .values()
                .filter(|session| session.user_id == user_id)
                .cloned()
                .collect()
        };
        for session in &cached {
            remember_revoked(&session.id, session.expires_at);
        }
        self.sessions.write().retain(|_, s| s.user_id != user_id);
        self.user_sessions.write().remove(user_id);

        crate::middleware::notify_session_revoked(user_id, None);
        Ok(())
    }

    /// Whether a bearer token that names `session_id` may still authenticate a
    /// request.
    ///
    /// The middleware calls this after [`crate::JwtManager::validate_access_token`],
    /// which proves the signature and the expiry but cannot see a revoked
    /// session (AUTH-05). A token minted without a session (an empty
    /// `session_id`, e.g. a service token) is not bound to a session and is
    /// left to the signature and the user check.
    ///
    /// With a database the stored row is the authority: its absence means the
    /// session was revoked or never existed, so the token is refused. Without
    /// one, an unknown id cannot be told apart from a token minted outside a
    /// session, so only the revocation marker refuses it.
    ///
    /// # Errors
    ///
    /// Storage errors while loading the session.
    pub async fn session_allows_token(&self, session_id: &str, user_id: &str) -> Result<bool> {
        if session_id.is_empty() {
            return Ok(true);
        }
        if is_revoked(session_id) {
            return Ok(false);
        }
        match self.get_session(session_id).await? {
            // Absolute expiry only: the sliding idle window is refreshed by the
            // session endpoints, not by bearer traffic, and enforcing it here
            // would sign an active API client out mid-work.
            Some(session) => Ok(session.user_id == user_id && !session.is_expired()),
            None => Ok(self.engine.is_none()),
        }
    }

    /// Get all sessions for a user
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn get_user_sessions(&self, user_id: &str) -> Result<Vec<Session>> {
        if self.engine.is_some() {
            return self.persisted_user_sessions(user_id).await;
        }

        let session_ids: Vec<String> = {
            let user_sessions = self.user_sessions.read();
            user_sessions.get(user_id).cloned().unwrap_or_default()
        };

        let sessions = self.sessions.read();
        Ok(session_ids
            .iter()
            .filter_map(|id| sessions.get(id).cloned())
            .collect())
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) -> usize {
        if self.engine.is_some() {
            let now = Utc::now();
            let mut count = 0;
            for session in self.all_persisted_sessions().await {
                if session.expires_at < now {
                    if self.delete_persisted(&session.id).await.is_ok() {
                        count += 1;
                    }
                }
            }
            self.sessions.write().retain(|_, s| {
                !s.is_expired() && !s.is_idle_expired(self.config.idle_timeout_minutes)
            });
            return count;
        }

        let mut sessions = self.sessions.write();
        let mut user_sessions = self.user_sessions.write();

        let expired_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.is_expired() || s.is_idle_expired(self.config.idle_timeout_minutes))
            .map(|(id, _)| id.clone())
            .collect();

        let count = expired_ids.len();
        for id in &expired_ids {
            if let Some(session) = sessions.remove(id) {
                if let Some(user_sess) = user_sessions.get_mut(&session.user_id) {
                    user_sess.retain(|sid| sid != id);
                }
            }
        }

        count
    }

    /// Every persisted session, in no particular order (empty without a
    /// database or on a storage error, so the sweeper never fails a request).
    async fn all_persisted_sessions(&self) -> Vec<Session> {
        let Some(engine) = &self.engine else {
            return Vec::new();
        };
        let meta = crate::doctypes::session_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            limit: Some(u32::MAX),
            ..ListQuery::default()
        };
        match engine.list(&ctx, &query).await {
            Ok((documents, _)) => documents
                .iter()
                .filter_map(|d| Session::from_document(d).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Refuse a login when the user already holds `max_sessions_per_user`.
    ///
    /// Eviction was the old behaviour and it is a data-loss bug: a new login
    /// silently took the oldest tab's session while that tab was working. The
    /// caller gets the typed error, the login handler answers it, and the user
    /// decides which device to sign out.
    ///
    /// Only **live** sessions hold a slot. Counting rows without looking at
    /// `expires_at`/`last_activity` turned the cap into a permanent 429 after
    /// five logins without a logout, because nothing ever deleted the expired
    /// rows (AUTH-03). The expired row is deleted here, at creation time, so
    /// the caller does not depend on a sweeper having run.
    async fn ensure_session_capacity(&self, user_id: &str) -> Result<()> {
        let max = self.config.max_sessions_per_user as usize;
        if max == 0 {
            return Ok(());
        }
        if self.engine.is_some() {
            let mut live = 0usize;
            for session in self.persisted_user_sessions(user_id).await? {
                if session.is_expired() || session.is_idle_expired(self.config.idle_timeout_minutes)
                {
                    if self.delete_persisted(&session.id).await.is_ok() {
                        self.sessions.write().remove(&session.id);
                    }
                    continue;
                }
                live += 1;
            }
            if live >= max {
                return Err(ferris_core::Error::Auth(
                    ferris_core::AuthError::TooManySessions(max as u32),
                ));
            }
            return Ok(());
        }

        // Pool-less install: the cache is the whole store, so prune the dead
        // ids the same way before counting.
        let stale: Vec<String> = {
            let sessions = self.sessions.read();
            let user_sessions = self.user_sessions.read();
            user_sessions
                .get(user_id)
                .map(|ids| {
                    ids.iter()
                        .filter(|id| {
                            sessions.get(*id).is_none_or(|session| {
                                session.is_expired()
                                    || session.is_idle_expired(self.config.idle_timeout_minutes)
                            })
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        if !stale.is_empty() {
            {
                let mut sessions = self.sessions.write();
                for id in &stale {
                    sessions.remove(id);
                }
            }
            if let Some(ids) = self.user_sessions.write().get_mut(user_id) {
                ids.retain(|id| !stale.contains(id));
            }
        }

        let count = {
            let user_sessions = self.user_sessions.read();
            user_sessions.get(user_id).map_or(0, Vec::len)
        };
        if count >= max {
            return Err(ferris_core::Error::Auth(
                ferris_core::AuthError::TooManySessions(max as u32),
            ));
        }
        Ok(())
    }

    /// Sweep every expired session now, and keep sweeping on `interval`.
    ///
    /// [`SessionManager::cleanup_expired`] had no caller, which is what let
    /// expired rows accumulate until the concurrent-session cap saw them as
    /// live (AUTH-03). A manager is built once at boot, so the constructor is
    /// the right place to start the sweep; the first tick runs immediately and
    /// doubles as the boot-time sweep. Returns `false` when there is no Tokio
    /// runtime to attach the task to (a pool-less unit-test manager).
    #[must_use]
    pub fn spawn_cleanup_task(&self, interval: std::time::Duration) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let manager = self.clone();
        handle.spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let removed = manager.cleanup_expired().await;
                if removed > 0 {
                    tracing::debug!(removed, "expired sessions swept");
                }
            }
        });
        true
    }

    /// Validate a CSRF token against the session, in constant time.
    ///
    /// Loads through the same cache-then-database path as every other lookup
    /// ([`SessionManager::get_session`]), so a session that lives only in the
    /// database validates too — the old check read just the in-memory cache and
    /// could not see persisted sessions. The comparison is constant-time so a
    /// mismatch does not leak how many bytes agreed.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub async fn validate_csrf_token(&self, session_id: &str, token: &str) -> Result<bool> {
        let Some(session) = self.load_session(session_id).await? else {
            return Ok(false);
        };
        Ok(constant_time_eq(
            session.csrf_token.as_bytes(),
            token.as_bytes(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionConfig;

    /// AUTH-03: an expired session must not hold a slot of
    /// `max_sessions_per_user`. Before the fix the cap counted every row, so
    /// five logins without a logout left the account on a permanent 429 because
    /// nothing ever deleted the expired rows.
    #[tokio::test]
    async fn an_expired_session_does_not_hold_a_slot() {
        let config = SessionConfig {
            // Zero-length sessions so the first one is expired at birth; no
            // clock injection is needed.
            timeout_minutes: 0,
            max_sessions_per_user: 1,
            ..SessionConfig::default()
        };
        let manager = SessionManager::new(config);
        let first = manager
            .create_session("user@example.com".to_string(), None, None)
            .await
            .unwrap();

        let second = manager
            .create_session("user@example.com".to_string(), None, None)
            .await
            .expect("an expired session must not exhaust the cap");
        assert_ne!(first.id, second.id);
        // The expired row was swept while checking the cap.
        assert!(manager.get_session(&first.id).await.unwrap().is_none());
    }

    /// AUTH-03 (persisted path): the same sweep applies to rows read from the
    /// database, and the live session still counts.
    #[tokio::test]
    async fn the_persisted_cap_counts_only_live_sessions() {
        let db = ferris_db::memory_db().await.unwrap();
        let manager = SessionManager::with_db(
            SessionConfig {
                max_sessions_per_user: 2,
                ..SessionConfig::default()
            },
            db.clone(),
        );
        manager.init().await.unwrap();

        let live = manager
            .create_session("user@example.com".to_string(), None, None)
            .await
            .unwrap();

        // An expired row written by an older deployment.
        let mut expired = Session::new("user@example.com".to_string(), &manager.config);
        expired.expires_at = Utc::now() - chrono::Duration::minutes(5);
        manager.persist(&expired).await.unwrap();

        // One live + one expired is under the cap of two, and the sweep removes
        // the expired document.
        let second = manager
            .create_session("user@example.com".to_string(), None, None)
            .await
            .unwrap();
        assert_ne!(live.id, second.id);
        assert!(
            manager.get_session(&expired.id).await.unwrap().is_none(),
            "the expired row is deleted when the cap is checked"
        );
    }

    /// AUTH-05: deleting a session revokes the JWT minted for it. The token
    /// itself cannot be unsigned, so the session behind it is what the request
    /// path consults.
    #[tokio::test]
    async fn a_revoked_session_no_longer_backs_a_token() {
        let manager = SessionManager::new(SessionConfig::default());
        let session = manager
            .create_session("user@example.com".to_string(), None, None)
            .await
            .unwrap();
        assert!(
            manager
                .session_allows_token(&session.id, "user@example.com")
                .await
                .unwrap()
        );

        manager.delete_session(&session.id).await.unwrap();
        assert!(
            !manager
                .session_allows_token(&session.id, "user@example.com")
                .await
                .unwrap(),
            "a token naming a revoked session must be refused"
        );
        // A token that names no session is not bound to one.
        assert!(
            manager
                .session_allows_token("", "user@example.com")
                .await
                .unwrap(),
            "a token minted outside a session keeps working"
        );
    }

    /// AUTH-05: "sign out everywhere" must also drop sessions that were
    /// hydrated from the database into the cache. They used to survive because
    /// only the ids tracked in `user_sessions` were removed.
    #[tokio::test]
    async fn delete_user_sessions_drops_cached_sessions() {
        let db = ferris_db::memory_db().await.unwrap();
        let first = SessionManager::with_db(SessionConfig::default(), db.clone());
        first.init().await.unwrap();
        let session = first
            .create_session("user@example.com".to_string(), None, None)
            .await
            .unwrap();

        // A second manager has an empty cache and hydrates the session.
        let second = SessionManager::with_db(SessionConfig::default(), db);
        assert!(second.get_session(&session.id).await.unwrap().is_some());

        second
            .delete_user_sessions("user@example.com")
            .await
            .unwrap();
        assert!(
            second.get_session(&session.id).await.unwrap().is_none(),
            "the cached session must not survive a global sign-out"
        );
        assert_eq!(
            second
                .get_user_sessions("user@example.com")
                .await
                .unwrap()
                .len(),
            0
        );
    }
}
