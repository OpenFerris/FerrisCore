//! Session management.
//!
//! Sessions are persisted as [`Session`](crate::doctypes::session_meta)
//! documents when a database is configured: the document engine is the single
//! source of truth, and the manager is a small cache-then-engine facade on top.
//! The manager acts as `System Manager` (see `store_support.rs`) because it runs
//! at sign-in, before any user is authenticated, and it reads the session's
//! `csrf_token` — a `permlevel` 1, hidden credential. Without a database the
//! manager keeps everything in memory, which is what its unit tests and a
//! pool-less boot use.
//!
//! Split into fragments: this `mod.rs` owns the config, the cookie builders and
//! the [`Session`] type; [`store`] owns the [`SessionManager`] lifecycle/cache
//! surface; [`tests`] holds the suite.

mod store;

#[cfg(test)]
mod tests;

use chrono::{DateTime, Duration, Utc};
use ferris_core::{Error, Map, Result, Value};
use ferris_db::Db;
use ferris_document::DocumentEngine;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// The DocType the manager persists.
const DOCTYPE: &str = "Session";

/// Minimum movement of `last_activity`, in seconds, before a `touch_session`
/// writes the row back (AUTH14). Far below any sane idle timeout, so throttling
/// the write never lets an active session lapse.
const TOUCH_PERSIST_INTERVAL_SECS: i64 = 30;

/// Session configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub timeout_minutes: u64,
    pub idle_timeout_minutes: u64,
    pub max_sessions_per_user: u32,
    pub secure_cookies: bool,
    pub same_site: SameSite,
    pub cookie_name: String,
    pub cookie_domain: Option<String>,
    pub cookie_path: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            timeout_minutes: 60,
            idle_timeout_minutes: 30,
            max_sessions_per_user: 5,
            secure_cookies: true,
            same_site: SameSite::Lax,
            cookie_name: "ferris_session".to_string(),
            cookie_domain: None,
            cookie_path: "/".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

impl SessionConfig {
    /// Build a `Set-Cookie` value for a session honoring the configured
    /// security attributes (HttpOnly, Secure, SameSite, path and domain).
    pub fn build_cookie(&self, session_id: &str) -> cookie::Cookie<'static> {
        use cookie::{Cookie, SameSite as CookieSameSite};
        let same_site = match self.same_site {
            SameSite::Strict => CookieSameSite::Strict,
            SameSite::Lax => CookieSameSite::Lax,
            SameSite::None => CookieSameSite::None,
        };
        let mut builder = Cookie::build(self.cookie_name.clone(), session_id.to_string())
            .path(self.cookie_path.clone())
            .http_only(true)
            .secure(self.secure_cookies)
            .same_site(same_site);
        if let Some(domain) = &self.cookie_domain {
            builder = builder.domain(domain.clone());
        }
        builder.finish()
    }

    /// Build a `Set-Cookie` value that deletes the session cookie (empty
    /// value with an immediate `Max-Age`, matched to [`SessionConfig::build_cookie`]).
    pub fn clear_cookie(&self) -> cookie::Cookie<'static> {
        let mut builder = cookie::Cookie::build(self.cookie_name.clone(), String::new())
            .path(self.cookie_path.clone())
            .http_only(true)
            .secure(self.secure_cookies)
            .max_age(cookie::time::Duration::ZERO);
        if let Some(domain) = &self.cookie_domain {
            builder = builder.domain(domain.clone());
        }
        builder.finish()
    }
}

/// Session data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub data: HashMap<String, Value>,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub csrf_token: String,
}

impl Session {
    pub fn new(user_id: String, config: &SessionConfig) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            ip_address: None,
            user_agent: None,
            data: HashMap::new(),
            created_at: now,
            last_activity: now,
            expires_at: now + Duration::minutes(config.timeout_minutes as i64),
            csrf_token: Uuid::new_v4().to_string(),
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn is_idle_expired(&self, idle_timeout_minutes: u64) -> bool {
        Utc::now() > self.last_activity + Duration::minutes(idle_timeout_minutes as i64)
    }

    pub fn refresh(&mut self, config: &SessionConfig) {
        self.last_activity = Utc::now();
        self.expires_at = Utc::now() + Duration::minutes(config.timeout_minutes as i64);
    }

    pub fn set_data(&mut self, key: String, value: Value) {
        self.data.insert(key, value);
    }

    pub fn get_data(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn remove_data(&mut self, key: &str) -> Option<Value> {
        self.data.remove(key)
    }

    /// The engine document for this session.
    pub(super) fn to_document(&self) -> Value {
        let text = |value: &Option<String>| {
            value
                .clone()
                .map_or(Value::Null, |s| Value::from(s.as_str()))
        };
        let mut map = Map::new();
        map.insert("name".to_string(), Value::from(self.id.as_str()));
        map.insert("user_id".to_string(), Value::from(self.user_id.as_str()));
        map.insert("ip_address".to_string(), text(&self.ip_address));
        map.insert("user_agent".to_string(), text(&self.user_agent));
        map.insert(
            "data".to_string(),
            Value::from(serde_json::to_string(&self.data).unwrap_or_else(|_| "{}".into())),
        );
        map.insert(
            "created_at".to_string(),
            Value::from(self.created_at.to_rfc3339()),
        );
        map.insert(
            "last_activity".to_string(),
            Value::from(self.last_activity.to_rfc3339()),
        );
        map.insert(
            "expires_at".to_string(),
            Value::from(self.expires_at.to_rfc3339()),
        );
        map.insert(
            "csrf_token".to_string(),
            Value::from(self.csrf_token.as_str()),
        );
        Value::Object(map)
    }

    /// Engine document → `Session`.
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`](ferris_core::AuthError::TokenInvalid) when a
    /// stored timestamp column is not RFC 3339. An invalid `expires_at` used to
    /// be masked with `Utc::now()`, which read back as "already expired"/"never
    /// expires" depending on direction — a silent, security-relevant default
    /// (AUTH12).
    pub(super) fn from_document(document: &Value) -> Result<Self> {
        let text = |name: &str| {
            document
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let parse = |name: &str| {
            DateTime::parse_from_rfc3339(&text(name))
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| {
                    Error::Auth(ferris_core::AuthError::TokenInvalid(format!(
                        "session {name} is not RFC 3339: {e}"
                    )))
                })
        };
        let data = document
            .get("data")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<HashMap<String, Value>>(s).ok())
            .unwrap_or_default();
        Ok(Self {
            id: text("name"),
            user_id: text("user_id"),
            ip_address: document
                .get("ip_address")
                .and_then(Value::as_str)
                .map(str::to_string),
            user_agent: document
                .get("user_agent")
                .and_then(Value::as_str)
                .map(str::to_string),
            data,
            created_at: parse("created_at")?,
            last_activity: parse("last_activity")?,
            expires_at: parse("expires_at")?,
            csrf_token: text("csrf_token"),
        })
    }
}

/// Session manager
#[derive(Clone)]
pub struct SessionManager {
    config: SessionConfig,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    user_sessions: Arc<RwLock<HashMap<String, Vec<String>>>>, // user_id -> session_ids
    engine: Option<DocumentEngine>,
}

impl SessionManager {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_sessions: Arc::new(RwLock::new(HashMap::new())),
            engine: None,
        }
    }

    pub fn with_db(config: SessionConfig, db: Db) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_sessions: Arc::new(RwLock::new(HashMap::new())),
            engine: Some(DocumentEngine::new(db)),
        }
    }

    /// Create the `Session` table (no-op without a database).
    ///
    /// # Errors
    ///
    /// Database errors from creating the table or its indexes.
    pub async fn init(&self) -> Result<()> {
        let Some(engine) = &self.engine else {
            return Ok(());
        };
        // Register the meta first so `ensure_table` materializes the declared
        // `user_id` index, exactly as the boot-time registry does.
        ferris_schema::register_doctype(crate::doctypes::session_meta());
        engine.ensure_table(DOCTYPE).await.map(|_| ())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(SessionConfig::default())
    }
}
