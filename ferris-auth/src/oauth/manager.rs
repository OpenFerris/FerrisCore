use ferris_core::{Error, Result, ValidationError};
use std::collections::HashMap;
use url::Url;

use super::OAuthConfig;
use super::client::{OAuthClient, TokenResponse};

/// Pending OAuth authorization state.
///
/// `pub(super)` so the crate's own tests can seed an expired entry directly;
/// it is not part of the public API.
#[derive(Debug, Clone)]
pub(super) struct PendingState {
    pub(super) provider: String,
    pub(super) pkce_verifier: Option<String>,
    pub(super) expires_at: chrono::DateTime<chrono::Utc>,
}

/// OAuth manager
pub struct OAuthManager {
    clients: HashMap<String, OAuthClient>,
    config: OAuthConfig,
    pub(super) state_store: HashMap<String, PendingState>,
}

impl OAuthManager {
    pub fn new(config: OAuthConfig) -> Result<Self> {
        let mut clients = HashMap::new();

        for provider_config in &config.providers {
            let client = OAuthClient::new(provider_config.clone())?;
            clients.insert(provider_config.name.clone(), client);
        }

        Ok(Self {
            clients,
            config,
            state_store: HashMap::new(),
        })
    }

    /// Get provider by name
    pub fn get_provider(&self, name: &str) -> Option<&OAuthClient> {
        self.clients.get(name)
    }

    /// List available providers
    pub fn list_providers(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// How long a pending state (and the browser-bound state cookie) lives.
    ///
    /// The login handler sets the cookie's `Max-Age` from this, so the two
    /// halves of the flow can never drift: a cookie that outlives its
    /// server-side state would let a callback through only to fail the state
    /// lookup, and one that dies early breaks a slow provider round trip.
    #[must_use]
    pub const fn state_ttl_seconds(&self) -> u64 {
        self.config.state_store.ttl_seconds
    }

    /// Start OAuth flow
    ///
    /// The state (and PKCE verifier) is kept server-side so the callback can
    /// bind the provider's reply to the browser that started the flow. Pruning
    /// expired entries here — on every login — keeps the store bounded without
    /// a background task (AUTH4): the map only ever holds states a caller could
    /// still be waiting on.
    pub fn authorize(&mut self, provider: &str) -> Result<(Url, String)> {
        let client = self.clients.get(provider).ok_or_else(|| {
            Error::Validation(ValidationError::Invalid(format!(
                "Provider not found: {provider}"
            )))
        })?;

        let (url, state, pkce_verifier) = client.authorize_url()?;

        // Store state (and PKCE verifier) server-side. A disabled store means
        // "do not persist between the two requests", not "break the flow": the
        // callback then trusts the state it is handed (AUTH4).
        if self.config.state_store.enabled {
            self.cleanup_states();
            let expiry = chrono::Utc::now()
                + chrono::Duration::seconds(self.config.state_store.ttl_seconds as i64);
            self.state_store.insert(
                state.clone(),
                PendingState {
                    provider: provider.to_string(),
                    pkce_verifier,
                    expires_at: expiry,
                },
            );
        }

        Ok((url, state))
    }

    /// Handle OAuth callback.
    ///
    /// `provider` is the provider named in the callback path. It is the only
    /// way to find the client when the state store is disabled (no entry was
    /// kept), which is what makes `state_store.enabled = false` a working
    /// stateless mode rather than a flow-breaker (AUTH4).
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`](ferris_core::AuthError::TokenInvalid) for an
    /// unknown or already-used state (single-use: the entry is removed before it
    /// is validated, so a replay fails), [`AuthError::TokenExpired`]
    /// (ferris_core::AuthError::TokenExpired) for an entry past its TTL, and
    /// [`ValidationError::Invalid`] for an unknown provider.
    pub async fn callback(
        &mut self,
        provider: &str,
        state: String,
        code: String,
    ) -> Result<TokenResponse> {
        let (provider_name, pkce_verifier) = if self.config.state_store.enabled {
            // Single-use state: remove before validating so a replayed state
            // fails.
            let pending = self.state_store.remove(&state).ok_or_else(|| {
                Error::Auth(ferris_core::AuthError::TokenInvalid(
                    "Invalid or expired state".to_string(),
                ))
            })?;
            if pending.expires_at < chrono::Utc::now() {
                return Err(Error::Auth(ferris_core::AuthError::TokenExpired));
            }
            // The state names the provider that minted it. A callback for
            // provider B carrying a state minted by A must not exchange B's
            // authorization code (or A's) through the wrong client: the path is
            // attacker-controlled, the stored provider is not.
            if pending.provider != provider {
                return Err(Error::Auth(ferris_core::AuthError::TokenInvalid(format!(
                    "state was issued for provider `{}`, not `{provider}`",
                    pending.provider
                ))));
            }
            (pending.provider, pending.pkce_verifier)
        } else {
            // Stateless mode: no stored state, so no PKCE verifier and no
            // state binding — the caller took that trade-off when it disabled
            // the store. The provider comes from the request path.
            (provider.to_string(), None)
        };

        let client = self.clients.get(&provider_name).ok_or_else(|| {
            Error::Validation(ValidationError::Invalid(format!(
                "Provider not found: {provider_name}"
            )))
        })?;

        client.exchange_code(code, pkce_verifier).await
    }

    /// Drop states whose TTL elapsed, so an abandoned login cannot grow the
    /// map without bound. Called on every `authorize`; also public so a caller
    /// can prune on a timer if it prefers.
    pub fn cleanup_states(&mut self) {
        let now = chrono::Utc::now();
        self.state_store
            .retain(|_, pending| pending.expires_at > now);
    }
}

impl Default for OAuthManager {
    fn default() -> Self {
        // The default config has no providers and cannot fail in practice, but
        // a `Default` impl that panics is a trap for the next field added
        // (AUTH11): fall back to the empty manager when construction fails.
        Self::new(OAuthConfig::default()).unwrap_or_else(|_| Self {
            clients: HashMap::new(),
            config: OAuthConfig::default(),
            state_store: HashMap::new(),
        })
    }
}
