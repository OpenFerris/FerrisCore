use ferris_core::{ConfigError, Error, Result, ValidationError};
use std::collections::HashMap;
use url::Url;

use super::OAuthConfig;
use super::client::{OAuthClient, TokenResponse};
use super::state_store::{PendingState, StateStore};

/// OAuth manager
///
/// Owns the per-provider clients and the pending-state store. The manager does
/// **not** perform the token exchange itself once a callback reaches it:
/// [`OAuthManager::take_pending`] hands the caller a cloned [`OAuthClient`] and
/// the PKCE verifier, so the caller can await the network with no shared state
/// locked (the state store is the only thing the lock protects).
pub struct OAuthManager {
    clients: HashMap<String, OAuthClient>,
    config: OAuthConfig,
    pub(super) state_store: StateStore,
}

impl OAuthManager {
    /// Build a manager with the process-local state store.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] for a provider that advertises OIDC (see
    /// [`OAuthProviderConfig::oidc`](super::OAuthProviderConfig::oidc)), and any
    /// error from [`OAuthClient::new`].
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub fn new(config: OAuthConfig) -> Result<Self> {
        Self::build(config, StateStore::memory())
    }

    /// Build a manager whose pending states live in `tab_oauth_state`, shared by
    /// every replica.
    ///
    /// # Errors
    ///
    /// The same as [`OAuthManager::new`].
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub fn with_db(config: OAuthConfig, db: ferris_db::Db) -> Result<Self> {
        Self::build(config, StateStore::from_db(db))
    }

    /// Switch the pending-state store to `db` after construction.
    pub fn set_db(&mut self, db: ferris_db::Db) {
        self.state_store = StateStore::from_db(db);
    }

    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    fn build(config: OAuthConfig, state_store: StateStore) -> Result<Self> {
        let mut clients = HashMap::new();

        for provider_config in &config.providers {
            // Refuse a configuration that expects OpenID Connect: the token
            // endpoint may answer with an `id_token`, and this crate does not
            // validate it (no JWKS, no `iss`/`aud`/`nonce`/`exp`). Silently
            // ignoring it would make a provider that *requires* OIDC look like a
            // working login while proving nothing.
            if provider_config.oidc {
                return Err(Error::Config(ConfigError::Invalid {
                    key: format!("oauth.providers.{}.oidc", provider_config.name),
                    msg: "OpenID Connect is not supported: id_token signatures and \
                          iss/aud/nonce/exp are not validated. Use OAuth2 + userinfo, \
                          or terminate OIDC in a proxy that validates the token"
                        .to_string(),
                }));
            }
            let client = OAuthClient::new(provider_config.clone())?;
            clients.insert(provider_config.name.clone(), client);
        }

        Ok(Self {
            clients,
            config,
            state_store,
        })
    }

    /// Get provider by name.
    ///
    /// The returned client is borrowed from the manager; a callback should use
    /// [`OAuthManager::take_pending`] instead, which clones it so the caller can
    /// drop the lock before the token exchange.
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

    /// Start OAuth flow.
    ///
    /// The state (and PKCE verifier) is persisted **before** the authorization
    /// URL is returned, so a callback that arrives immediately after the
    /// redirect cannot miss it. Pruning expired entries here — on every login —
    /// keeps the store bounded without a background task (AUTH4); the DB store
    /// needs the same sweep because it is shared and grows across replicas.
    ///
    /// # Errors
    ///
    /// [`ValidationError::Invalid`] for an unknown provider, the state store's
    /// database errors, and errors from building the authorization URL.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn authorize(&mut self, provider: &str) -> Result<(Url, String)> {
        let (url, state, pkce_verifier) = {
            let client = self.clients.get(provider).ok_or_else(|| {
                Error::Validation(ValidationError::Invalid(format!(
                    "Provider not found: {provider}"
                )))
            })?;
            client.authorize_url()?
        };

        // A disabled store means "do not persist between the two requests", not
        // "break the flow": the callback then trusts the state it is handed
        // (AUTH4).
        if self.config.state_store.enabled {
            self.cleanup_states().await?;
            let expiry = chrono::Utc::now()
                + chrono::Duration::seconds(
                    i64::try_from(self.config.state_store.ttl_seconds).unwrap_or(i64::MAX),
                );
            self.state_store
                .put(
                    &state,
                    PendingState {
                        provider: provider.to_string(),
                        pkce_verifier,
                        expires_at: expiry,
                    },
                )
                .await?;
        }

        Ok((url, state))
    }

    /// Consume the pending state and resolve the client that must exchange the
    /// code.
    ///
    /// This is the first half of [`OAuthManager::callback`], split out so the
    /// caller can drop the manager lock before any network I/O. It performs no
    /// request: the DB read/delete of a shared state store is the only await.
    ///
    /// `provider` is the provider named in the callback path. It is the only
    /// way to find the client when the state store is disabled (no entry was
    /// kept), which is what makes `state_store.enabled = false` a working
    /// stateless mode rather than a flow-breaker (AUTH4).
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`](ferris_core::AuthError::TokenInvalid) for an
    /// unknown state, one another callback already consumed, or one minted for
    /// another provider; [`AuthError::TokenExpired`](ferris_core::AuthError::TokenExpired)
    /// for an entry past its TTL; [`ValidationError::Invalid`] for a provider
    /// the state names but this manager does not know.
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn take_pending(
        &mut self,
        provider: &str,
        state: &str,
    ) -> Result<(OAuthClient, Option<String>)> {
        let (provider_name, pkce_verifier) = if self.config.state_store.enabled {
            // Single use: the store removes the entry before it is validated, so
            // a replayed state fails.
            let pending = self.state_store.take(state).await?.ok_or_else(|| {
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

        let client = self.clients.get(&provider_name).cloned().ok_or_else(|| {
            Error::Validation(ValidationError::Invalid(format!(
                "Provider not found: {provider_name}"
            )))
        })?;

        Ok((client, pkce_verifier))
    }

    /// Handle OAuth callback (convenience: consume the state and exchange the
    /// code in one call).
    ///
    /// Callers that must not hold a lock across network I/O should use
    /// [`OAuthManager::take_pending`] and await the exchange on the returned
    /// client instead.
    ///
    /// # Errors
    ///
    /// Everything [`OAuthManager::take_pending`] reports, plus the token
    /// endpoint's errors from [`OAuthClient::exchange_code`].
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn callback(
        &mut self,
        provider: &str,
        state: String,
        code: String,
    ) -> Result<TokenResponse> {
        let (client, pkce_verifier) = self.take_pending(provider, &state).await?;
        client.exchange_code(code, pkce_verifier).await
    }

    /// Drop states whose TTL elapsed, so an abandoned login cannot grow the
    /// store without bound.
    ///
    /// Called on every `authorize`; also public so a caller can prune on a timer
    /// if it prefers.
    ///
    /// # Errors
    ///
    /// Database errors from the sweep (the in-memory store cannot fail).
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn cleanup_states(&mut self) -> Result<()> {
        self.state_store.sweep().await
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
            state_store: StateStore::memory(),
        })
    }
}
