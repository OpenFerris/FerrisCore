//! The user identity, at the base of the crate graph.
//!
//! `User` is the subject every permission decision is made about, and the
//! document engine authorises each read and write against it ([`crate::permissions`]).
//! It lives here, in the bottom crate, rather than in `ferris-auth` so that the
//! engine (`ferris-document`) does not have to depend on `ferris-auth`: the
//! moment `ferris-auth` uses the engine for its own stores, that dependency
//! would be a crate cycle. `ferris-auth` re-exports these types, so
//! `ferris_auth::User` keeps its historical path.
//!
//! The type is deliberately storage-agnostic: it carries the fields a login
//! and an authorisation decision need, and nothing about how an account is
//! persisted.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::Value;

/// User type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserType {
    SystemUser,
    WebsiteUser,
    Guest,
}

/// User role
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRole {
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub permissions: Vec<String>,
}

/// User model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub username: Option<String>,
    pub full_name: String,
    pub user_type: UserType,
    pub enabled: bool,
    pub password_hash: Option<String>,
    pub salt: Option<String>,
    pub roles: Vec<String>,
    pub role_profile: Option<String>,
    pub last_login: Option<DateTime<Utc>>,
    pub login_count: u32,
    pub failed_login_attempts: u32,
    pub locked_until: Option<DateTime<Utc>>,
    pub mfa_enabled: bool,
    pub mfa_secret: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
    pub modified_by: String,
    pub metadata: HashMap<String, Value>,
}

impl User {
    /// Create a new user
    ///
    /// The id is a v7 UUID (time-ordered) rather than a v4: it sorts by creation
    /// time, so the primary-key index stays clustered on insert. The workspace
    /// already enables the `v7` feature.
    pub fn new(email: String, full_name: String, user_type: UserType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7().to_string(),
            email,
            username: None,
            full_name,
            user_type,
            enabled: true,
            password_hash: None,
            salt: None,
            roles: Vec::new(),
            role_profile: None,
            last_login: None,
            login_count: 0,
            failed_login_attempts: 0,
            locked_until: None,
            mfa_enabled: false,
            mfa_secret: None,
            language: Some("en".to_string()),
            timezone: Some("UTC".to_string()),
            created_at: now,
            updated_at: now,
            created_by: "System".to_string(),
            modified_by: "System".to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Create a system user
    pub fn system_user(email: String, full_name: String) -> Self {
        Self::new(email, full_name, UserType::SystemUser)
    }

    /// Create a website user
    pub fn website_user(email: String, full_name: String) -> Self {
        Self::new(email, full_name, UserType::WebsiteUser)
    }

    /// Create a guest user
    pub fn guest() -> Self {
        let now = Utc::now();
        Self {
            id: "guest".to_string(),
            email: "guest@local".to_string(),
            username: Some("Guest".to_string()),
            full_name: "Guest".to_string(),
            user_type: UserType::Guest,
            enabled: true,
            password_hash: None,
            salt: None,
            roles: vec!["Guest".to_string()],
            role_profile: None,
            last_login: None,
            login_count: 0,
            failed_login_attempts: 0,
            locked_until: None,
            mfa_enabled: false,
            mfa_secret: None,
            language: Some("en".to_string()),
            timezone: Some("UTC".to_string()),
            created_at: now,
            updated_at: now,
            created_by: "System".to_string(),
            modified_by: "System".to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Check if user has a role
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(&role.to_string())
    }

    /// Check if user has any of the roles
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        roles.iter().any(|r| self.has_role(r))
    }

    /// Check if user has all roles
    pub fn has_all_roles(&self, roles: &[&str]) -> bool {
        roles.iter().all(|r| self.has_role(r))
    }

    /// Add a role
    pub fn add_role(&mut self, role: String) {
        if !self.roles.contains(&role) {
            self.roles.push(role);
        }
    }

    /// Remove a role
    pub fn remove_role(&mut self, role: &str) {
        self.roles.retain(|r| r != role);
    }

    /// Check if the account is locked as of `now`.
    ///
    /// `now` is a parameter rather than `Utc::now()` so a caller (and a test)
    /// decides what "now" is: a lockout expiry test otherwise has to sleep.
    pub fn is_locked(&self, now: DateTime<Utc>) -> bool {
        self.locked_until.is_some_and(|until| until > now)
    }

    /// Check if password is set
    pub fn has_password(&self) -> bool {
        self.password_hash.is_some()
    }

    /// Get display name
    pub fn display_name(&self) -> &str {
        self.username.as_deref().unwrap_or(&self.full_name)
    }

    /// Record a successful login at `now`.
    pub fn record_login(&mut self, now: DateTime<Utc>) {
        self.last_login = Some(now);
        self.login_count += 1;
        self.failed_login_attempts = 0;
        self.locked_until = None;
    }

    /// Record a failed login at `now`, locking the account once
    /// `max_attempts` is reached.
    pub fn record_failed_login(
        &mut self,
        now: DateTime<Utc>,
        max_attempts: u32,
        lockout_duration_minutes: u64,
    ) {
        self.failed_login_attempts += 1;
        if self.failed_login_attempts >= max_attempts {
            self.locked_until =
                Some(now + chrono::Duration::minutes(lockout_duration_minutes as i64));
        }
    }

    /// Enable MFA
    pub fn enable_mfa(&mut self, secret: String) {
        self.mfa_enabled = true;
        self.mfa_secret = Some(secret);
    }

    /// Disable MFA
    pub fn disable_mfa(&mut self) {
        self.mfa_enabled = false;
        self.mfa_secret = None;
    }

    /// Set password hash
    pub fn set_password(&mut self, hash: String, salt: String) {
        self.password_hash = Some(hash);
        self.salt = Some(salt);
    }
}

impl Default for User {
    fn default() -> Self {
        Self::guest()
    }
}

/// User creation input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserInput {
    pub email: String,
    pub full_name: String,
    pub username: Option<String>,
    pub user_type: UserType,
    pub password: Option<String>,
    pub roles: Vec<String>,
    pub role_profile: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub metadata: HashMap<String, Value>,
}

/// User update input
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateUserInput {
    pub email: Option<String>,
    pub full_name: Option<String>,
    pub username: Option<String>,
    pub user_type: Option<UserType>,
    pub enabled: Option<bool>,
    pub roles: Option<Vec<String>>,
    pub role_profile: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub metadata: Option<HashMap<String, Value>>,
}

/// User filters for listing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserFilters {
    pub email: Option<String>,
    pub username: Option<String>,
    pub full_name: Option<String>,
    pub user_type: Option<UserType>,
    pub enabled: Option<bool>,
    pub role: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_creation() {
        let user = User::system_user("test@example.com".to_string(), "Test User".to_string());
        assert_eq!(user.email, "test@example.com");
        assert_eq!(user.full_name, "Test User");
        assert_eq!(user.user_type, UserType::SystemUser);
        assert!(user.enabled);
    }

    #[test]
    fn test_user_roles() {
        let mut user = User::system_user("test@example.com".to_string(), "Test User".to_string());
        user.add_role("Admin".to_string());
        user.add_role("User".to_string());

        assert!(user.has_role("Admin"));
        assert!(user.has_role("User"));
        assert!(!user.has_role("Guest"));

        assert!(user.has_any_role(&["Admin", "Guest"]));
        assert!(!user.has_all_roles(&["Admin", "Guest"]));
    }

    #[test]
    fn test_guest_user() {
        let guest = User::guest();
        assert_eq!(guest.user_type, UserType::Guest);
        assert_eq!(guest.email, "guest@local");
        assert!(guest.has_role("Guest"));
    }

    #[test]
    fn test_lockout() {
        let mut user = User::system_user("test@example.com".to_string(), "Test User".to_string());
        let t0 = Utc::now();
        user.record_failed_login(t0, 3, 15);
        user.record_failed_login(t0, 3, 15);
        assert!(!user.is_locked(t0));

        user.record_failed_login(t0, 3, 15);
        assert!(user.is_locked(t0));
    }

    /// The lockout expiry is testable without sleeping because `now` is injected.
    #[test]
    fn a_lockout_expires_on_the_injected_clock() {
        let mut user = User::system_user("test@example.com".to_string(), "Test User".to_string());
        let t0 = Utc::now();
        user.record_failed_login(t0, 1, 15);
        assert!(user.is_locked(t0));

        // 14 minutes later it still holds; a minute later it has expired.
        assert!(user.is_locked(t0 + chrono::Duration::minutes(14)));
        assert!(!user.is_locked(t0 + chrono::Duration::minutes(16)));

        // A successful login clears the lockout immediately.
        user.record_login(t0 + chrono::Duration::minutes(16));
        assert!(!user.is_locked(t0 + chrono::Duration::minutes(16)));
        assert_eq!(user.login_count, 1);
        assert_eq!(user.failed_login_attempts, 0);
    }
}
