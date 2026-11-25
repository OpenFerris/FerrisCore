use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ferris_core::{ConfigError, Error, Result, ValidationError, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::{OAuthProviderConfig, PkceMethod, generate_pkce_verifier};

/// Per-provider OAuth2 client.
///
/// `Clone` on purpose: a callback takes the client out of the manager (dropping
/// the manager lock) and awaits the token exchange and userinfo on its own
/// copy, so no network I/O happens while shared state is locked.
#[derive(Clone, Debug)]
pub struct OAuthClient {
    provider_name: String,
    config: OAuthProviderConfig,
    http_client: reqwest::Client,
}

impl OAuthClient {
    pub fn new(config: OAuthProviderConfig) -> Result<Self> {
        let http_client = reqwest::Client::new();

        Ok(Self {
            provider_name: config.name.clone(),
            config,
            http_client,
        })
    }

    /// Whether a login through this provider must present a userinfo
    /// `email_verified: true` before its email resolves a local account.
    #[must_use]
    pub const fn require_verified_email(&self) -> bool {
        self.config.require_verified_email
    }

    /// Resolve the redirect URI for this provider.
    ///
    /// Requires an explicit allow-list entry: deriving it from `auth_url` would
    /// allow an open redirect.
    pub fn redirect_uri(&self) -> Result<String> {
        self.config.redirect_uris.first().cloned().ok_or_else(|| {
            Error::Config(ConfigError::Missing(format!(
                "redirect_uris for OAuth provider {}",
                self.provider_name
            )))
        })
    }

    /// Generate authorization URL, state and (when PKCE is on) the verifier.
    ///
    /// The verifier must be stored server-side keyed by `state` and sent back on
    /// the token exchange. The `state` is part of the URL (RFC 6749 §4.1.1): the
    /// provider echoes it back on the redirect, which is the only way the
    /// callback can recognize the flow it minted. It used to be handed back to
    /// the caller and shipped in an `X-OAuth-State` header instead, so the
    /// provider never received it and nothing bound the callback to the browser.
    pub fn authorize_url(&self) -> Result<(Url, String, Option<String>)> {
        let mut url = Url::parse(&self.config.auth_url)
            .map_err(|e| ferris_core::AuthError::OAuthError(format!("Invalid auth URL: {e}")))?;

        let redirect_uri = self.redirect_uri()?;
        let state = uuid::Uuid::new_v4().to_string();

        url.query_pairs_mut()
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &self.config.scopes.join(" "))
            .append_pair("state", &state);

        let mut verifier = None;
        if self.config.pkce_enabled {
            let v = generate_pkce_verifier();
            let challenge = match self.config.pkce_method {
                PkceMethod::S256 => URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())),
                PkceMethod::Plain => v.clone(),
            };
            let method = match self.config.pkce_method {
                PkceMethod::S256 => "S256",
                PkceMethod::Plain => "plain",
            };
            url.query_pairs_mut()
                .append_pair("code_challenge_method", method)
                .append_pair("code_challenge", &challenge);
            verifier = Some(v);
        }

        Ok((url, state, verifier))
    }

    /// Exchange authorization code for tokens (RFC 6749 + PKCE).
    pub async fn exchange_code(
        &self,
        code: String,
        pkce_verifier: Option<String>,
    ) -> Result<TokenResponse> {
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code),
            ("client_id", self.config.client_id.clone()),
            ("client_secret", self.config.client_secret.clone()),
            ("redirect_uri", self.redirect_uri()?),
        ];
        if let Some(verifier) = pkce_verifier {
            form.push(("code_verifier", verifier));
        }

        let response = self
            .http_client
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| Error::Auth(ferris_core::AuthError::OAuthCodeExchange(e.to_string())))?;

        if !response.status().is_success() {
            return Err(Error::Auth(ferris_core::AuthError::OAuthCodeExchange(
                format!("token endpoint returned {}", response.status()),
            )));
        }

        response
            .json::<TokenResponse>()
            .await
            .map_err(|e| Error::Auth(ferris_core::AuthError::OAuthCodeExchange(e.to_string())))
    }

    /// Refresh an access token (RFC 6749 §6, `grant_type=refresh_token`).
    ///
    /// Posts the refresh token to the provider's token endpoint and returns the
    /// provider's real response. Providers may omit `refresh_token` in the
    /// reply (the old one stays valid) or `scope`, so both are optional.
    ///
    /// # Errors
    ///
    /// [`AuthError::OAuthTokenRefresh`](ferris_core::AuthError::OAuthTokenRefresh)
    /// when the endpoint is unreachable, rejects the grant, or answers with a
    /// body that is not a token response.
    pub async fn refresh_token(&self, refresh_token: String) -> Result<TokenResponse> {
        let form = [
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
            ("client_id", self.config.client_id.clone()),
            ("client_secret", self.config.client_secret.clone()),
        ];

        let response = self
            .http_client
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| Error::Auth(ferris_core::AuthError::OAuthTokenRefresh(e.to_string())))?;

        if !response.status().is_success() {
            return Err(Error::Auth(ferris_core::AuthError::OAuthTokenRefresh(
                format!("token endpoint returned {}", response.status()),
            )));
        }

        response
            .json::<TokenResponse>()
            .await
            .map_err(|e| Error::Auth(ferris_core::AuthError::OAuthTokenRefresh(e.to_string())))
    }

    /// Get user info
    pub async fn get_user_info(&self, access_token: &str) -> Result<Value> {
        if let Some(user_info_url) = &self.config.user_info_url {
            let response = self
                .http_client
                .get(user_info_url)
                // The userinfo endpoint authenticates with the access token
                // (RFC 6750); without the bearer header most providers answer
                // 401.
                .bearer_auth(access_token)
                .send()
                .await
                .map_err(|e| {
                    ferris_core::AuthError::OAuthError(format!("User info request failed: {e}"))
                })?;

            if !response.status().is_success() {
                return Err(Error::Auth(ferris_core::AuthError::OAuthCodeExchange(
                    format!("userinfo endpoint returned {}", response.status()),
                )));
            }

            let user_info = response.json().await.map_err(|e| {
                ferris_core::AuthError::OAuthError(format!("Failed to parse user info: {e}"))
            })?;

            Ok(user_info)
        } else {
            Err(Error::Validation(ValidationError::Invalid(
                "User info URL not configured".to_string(),
            )))
        }
    }
}

/// Token response.
///
/// An `id_token` in the provider's reply is **not** deserialized: Ferris does
/// not validate OIDC tokens (no JWKS signature check, no
/// `iss`/`aud`/`nonce`/`exp`), so it is deliberately dropped rather than
/// trusted. Identity comes from [`OAuthClient::get_user_info`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}
