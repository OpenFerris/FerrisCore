//! Authentication, sessions and OAuth providers.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::redact::Redacted;

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// JWT signing secret. There is deliberately **no usable default**: it must
    /// come from the environment — `FERRIS_JWT_SECRET`, i.e. `jwt_secret` under
    /// the `FERRIS_` prefix [`Config::load_from`] installs (a bare `JWT_SECRET`
    /// is *not* read here) — or an explicit config value. `None` means "not
    /// configured" and any consumer that needs to sign/validate tokens must
    /// fail rather than fall back to a placeholder.
    pub jwt_secret: Option<String>,
    pub jwt_expiry_hours: u64,
    pub refresh_expiry_days: u64,
    pub password_min_length: usize,
    pub password_require_uppercase: bool,
    pub password_require_lowercase: bool,
    pub password_require_numbers: bool,
    pub password_require_symbols: bool,
    pub max_login_attempts: u32,
    pub lockout_duration_minutes: u64,
    pub mfa_enabled: bool,
    pub mfa_issuer: String,
    pub session_timeout_minutes: u64,
    /// Concurrent sessions one user may hold. A login beyond this is refused
    /// (it never evicts an older session).
    pub max_sessions_per_user: u32,
    pub oauth: OAuthConfig,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("jwt_secret", &self.jwt_secret.as_ref().map(|_| Redacted))
            .field("jwt_expiry_hours", &self.jwt_expiry_hours)
            .field("refresh_expiry_days", &self.refresh_expiry_days)
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
            .field("mfa_enabled", &self.mfa_enabled)
            .field("mfa_issuer", &self.mfa_issuer)
            .field("session_timeout_minutes", &self.session_timeout_minutes)
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
