//! OAuth2/OpenID Connect integration.
//!
//! Split into focused fragments: [`client`] owns the per-provider HTTP client
//! (PKCE, token exchange, refresh, userinfo), [`manager`] owns provider lookup
//! and the server-side state store, and this module holds the shared wire types
//! and the PKCE verifier helper. `mod.rs` only declares and re-exports, so every
//! existing `crate::oauth::Thing` path keeps resolving.

mod client;
mod cookie;
mod manager;

pub use client::{OAuthClient, TokenResponse};
pub use cookie::{NEXT_COOKIE, STATE_COOKIE, clear_cookie, flow_cookie};
pub use manager::OAuthManager;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ferris_core::config::Redacted;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fmt;

/// OAuth provider configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub user_info_url: Option<String>,
    pub device_auth_url: Option<String>,
    pub scopes: Vec<String>,
    pub pkce_enabled: bool,
    pub pkce_method: PkceMethod,
    /// Allow-list of redirect URIs. At least one must be configured; the URI is
    /// never derived from the provider URL (avoids open redirects).
    #[serde(default)]
    pub redirect_uris: Vec<String>,
}

impl fmt::Debug for OAuthProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthProviderConfig")
            .field("name", &self.name)
            .field("client_id", &self.client_id)
            .field("client_secret", &Redacted)
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("user_info_url", &self.user_info_url)
            .field("device_auth_url", &self.device_auth_url)
            .field("scopes", &self.scopes)
            .field("pkce_enabled", &self.pkce_enabled)
            .field("pkce_method", &self.pkce_method)
            .field("redirect_uris", &self.redirect_uris)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PkceMethod {
    S256,
    Plain,
}

/// Generate an RFC 7636 PKCE code verifier (43 chars, URL-safe, no padding).
pub(super) fn generate_pkce_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// OAuth configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub providers: Vec<OAuthProviderConfig>,
    pub state_store: StateStoreConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStoreConfig {
    pub enabled: bool,
    pub ttl_seconds: u64,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            state_store: StateStoreConfig {
                enabled: true,
                ttl_seconds: 600,
            },
        }
    }
}

/// Device authorization response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

#[cfg(test)]
mod tests;
