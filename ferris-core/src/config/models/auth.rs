//! Authentication, sessions and OAuth providers.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::redact::Redacted;

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub password_min_length: usize,
    pub password_require_uppercase: bool,
    pub password_require_lowercase: bool,
    pub password_require_numbers: bool,
    pub password_require_symbols: bool,
    pub max_login_attempts: u32,
    pub lockout_duration_minutes: u64,
    pub mfa_issuer: String,
    pub session_timeout_minutes: u64,
    /// Sliding idle window for a session, in minutes. A session expires after
    /// this long without a request, independently of the absolute
    /// `session_timeout_minutes` ceiling.
    #[serde(default = "default_session_idle_timeout_minutes")]
    pub session_idle_timeout_minutes: u64,
    /// Concurrent sessions one user may hold. A login beyond this is refused
    /// (it never evicts an older session).
    pub max_sessions_per_user: u32,
    /// Opt in to gating password logins on a verified email address.
    ///
    /// `false` (the default) keeps the historical behaviour: a verified address
    /// is self-service only and never blocks a login. When `true`, a login for
    /// an account whose `User.email_verified` is false is refused after the
    /// password (and the second factor, when configured) is proven. A
    /// provider-verified OAuth email satisfies the check and marks the local
    /// account verified.
    #[serde(default)]
    pub require_email_verification: bool,
    pub oauth: OAuthConfig,
}

/// 30 minutes: the effective idle window before `[auth]` exposed the knob, so
/// an existing config keeps behaving exactly the same.
fn default_session_idle_timeout_minutes() -> u64 {
    30
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("password_min_length", &self.password_min_length)
            .field(
                "password_require_uppercase",
                &self.password_require_uppercase,
            )
            .field(
                "password_require_lowercase",
                &self.password_require_lowercase,
            )
            .field("password_require_numbers", &self.password_require_numbers)
            .field("password_require_symbols", &self.password_require_symbols)
            .field("max_login_attempts", &self.max_login_attempts)
            .field("lockout_duration_minutes", &self.lockout_duration_minutes)
            .field("mfa_issuer", &self.mfa_issuer)
            .field("session_timeout_minutes", &self.session_timeout_minutes)
            .field(
                "session_idle_timeout_minutes",
                &self.session_idle_timeout_minutes,
            )
            .field(
                "require_email_verification",
                &self.require_email_verification,
            )
            .field("oauth", &self.oauth)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub providers: Vec<OAuthProvider>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthProvider {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub user_info_url: String,
    pub scopes: Vec<String>,
}

impl fmt::Debug for OAuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthProvider")
            .field("name", &self.name)
            .field("client_id", &self.client_id)
            .field("client_secret", &Redacted)
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("user_info_url", &self.user_info_url)
            .field("scopes", &self.scopes)
            .finish()
    }
}
