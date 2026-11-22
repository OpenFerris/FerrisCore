//! JWT token management

use chrono::{DateTime, Duration, Utc};
use ferris_core::{ConfigError, Error, Result};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Minimum accepted JWT secret length (HS256).
pub const MIN_SECRET_LEN: usize = 32;

/// JWT configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    pub secret: String,
    pub algorithm: Algorithm,
    pub access_token_expiry_hours: u64,
    pub refresh_token_expiry_days: u64,
    pub issuer: String,
    pub audience: String,
}

impl JwtConfig {
    /// Build a config from an explicit secret, rejecting weak values.
    pub fn new(secret: impl Into<String>) -> Result<Self> {
        let secret = secret.into();
        validate_secret(&secret)?;
        Ok(Self {
            secret,
            algorithm: Algorithm::HS256,
            access_token_expiry_hours: 24,
            refresh_token_expiry_days: 30,
            issuer: "ferris".to_string(),
            audience: "ferris".to_string(),
        })
    }

    /// Load the JWT secret from the environment.
    ///
    /// Reads `FERRIS_JWT_SECRET` (preferred) or `JWT_SECRET`. Fails if the
    /// secret is missing or too weak: there is no usable default.
    pub fn from_env() -> Result<Self> {
        let secret = std::env::var("FERRIS_JWT_SECRET")
            .or_else(|_| std::env::var("JWT_SECRET"))
            .map_err(|_| Error::Config(ConfigError::Missing("FERRIS_JWT_SECRET".to_string())))?;
        Self::new(secret)
    }
}

fn validate_secret(secret: &str) -> Result<()> {
    if secret.len() < MIN_SECRET_LEN {
        return Err(Error::Config(ConfigError::Invalid {
            key: "FERRIS_JWT_SECRET".to_string(),
            msg: format!("JWT secret must be at least {MIN_SECRET_LEN} characters"),
        }));
    }
    if secret == "change-me-in-production" {
        return Err(Error::Config(ConfigError::Invalid {
            key: "FERRIS_JWT_SECRET".to_string(),
            msg: "refusing to use the placeholder JWT secret".to_string(),
        }));
    }
    Ok(())
}

/// JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,            // Subject (user ID)
    pub email: String,          // User email
    pub roles: Vec<String>,     // User roles
    pub tenant: Option<String>, // Tenant ID
    pub session_id: String,     // Session ID
    pub iat: i64,               // Issued at
    pub exp: i64,               // Expiration
    pub iss: String,            // Issuer
    pub aud: String,            // Audience
    pub jti: String,            // JWT ID
    pub token_type: TokenType,  // Access or Refresh
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    Access,
    Refresh,
}

/// Token pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub token_type: String,
}

/// JWT manager
#[derive(Clone)]
pub struct JwtManager {
    config: JwtConfig,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    /// Keys retired by [`JwtManager::rotate_secret`], kept so tokens issued
    /// before a rotation still validate until they expire.
    previous_decoding_keys: Vec<DecodingKey>,
    validation: Validation,
}

impl JwtManager {
    pub fn new(config: JwtConfig) -> Result<Self> {
        validate_secret(&config.secret)?;
        let encoding_key = EncodingKey::from_secret(config.secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(config.secret.as_bytes());
        let mut validation = Validation::new(config.algorithm);
        validation.set_issuer(&[config.issuer.clone()]);
        validation.set_audience(&[config.audience.clone()]);
        validation.validate_exp = true;

        Ok(Self {
            config,
            encoding_key,
            decoding_key,
            previous_decoding_keys: Vec::new(),
            validation,
        })
    }

    /// Rotate the signing secret.
    ///
    /// New tokens are signed with `new_secret`; the previous secret is retained
    /// for verification so tokens issued before the rotation keep working until
    /// they expire. Call this on every instance before the old tokens' lifetime
    /// elapses (or restart all instances) so no caller is locked out.
    pub fn rotate_secret(&mut self, new_secret: impl Into<String>) -> Result<()> {
        let new_secret = new_secret.into();
        validate_secret(&new_secret)?;
        self.previous_decoding_keys.push(self.decoding_key.clone());
        self.encoding_key = EncodingKey::from_secret(new_secret.as_bytes());
        self.decoding_key = DecodingKey::from_secret(new_secret.as_bytes());
        self.config.secret = new_secret;
        Ok(())
    }

    /// Decode a token, accepting any secret in the rotation ring.
    fn decode_any(&self, token: &str) -> Result<Claims> {
        let try_key = |key: &DecodingKey| {
            decode::<Claims>(token, key, &self.validation).map(|data| data.claims)
        };
        if let Ok(claims) = try_key(&self.decoding_key) {
            return Ok(claims);
        }
        for key in &self.previous_decoding_keys {
            if let Ok(claims) = try_key(key) {
                return Ok(claims);
            }
        }
        Err(Error::Auth(ferris_core::AuthError::TokenInvalid(
            "invalid or expired token".to_string(),
        )))
    }

    /// Generate access and refresh tokens
    pub fn generate_tokens(
        &self,
        user_id: &str,
        email: &str,
        roles: &[String],
        tenant: Option<&str>,
        session_id: &str,
    ) -> Result<TokenPair> {
        let now = Utc::now();
        let access_exp = now + Duration::hours(self.config.access_token_expiry_hours as i64);
        let refresh_exp = now + Duration::days(self.config.refresh_token_expiry_days as i64);

        let access_claims = Claims {
            sub: user_id.to_string(),
            email: email.to_string(),
            roles: roles.to_vec(),
            tenant: tenant.map(|s| s.to_string()),
            session_id: session_id.to_string(),
            iat: now.timestamp(),
            exp: access_exp.timestamp(),
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            jti: Uuid::new_v4().to_string(),
            token_type: TokenType::Access,
        };

        let refresh_claims = Claims {
            sub: user_id.to_string(),
            email: email.to_string(),
            roles: roles.to_vec(),
            tenant: tenant.map(|s| s.to_string()),
            session_id: session_id.to_string(),
            iat: now.timestamp(),
            exp: refresh_exp.timestamp(),
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            jti: Uuid::new_v4().to_string(),
            token_type: TokenType::Refresh,
        };

        let header = Header::new(self.config.algorithm);
        let access_token = encode(&header, &access_claims, &self.encoding_key).map_err(|e| {
            ferris_core::AuthError::TokenInvalid(format!("Failed to encode access token: {e}"))
        })?;

        let refresh_token = encode(&header, &refresh_claims, &self.encoding_key).map_err(|e| {
            ferris_core::AuthError::TokenInvalid(format!("Failed to encode refresh token: {e}"))
        })?;

        Ok(TokenPair {
            access_token,
            refresh_token,
            expires_in: self.config.access_token_expiry_hours * 3600,
            token_type: "Bearer".to_string(),
        })
    }

    /// Validate and decode access token
    pub fn validate_access_token(&self, token: &str) -> Result<Claims> {
        let claims = self.decode_any(token)?;

        if claims.token_type != TokenType::Access {
            return Err(Error::Auth(ferris_core::AuthError::TokenInvalid(
                "Not an access token".to_string(),
            )));
        }

        Ok(claims)
    }

    /// Validate and decode refresh token
    pub fn validate_refresh_token(&self, token: &str) -> Result<Claims> {
        let claims = self.decode_any(token)?;

        if claims.token_type != TokenType::Refresh {
            return Err(Error::Auth(ferris_core::AuthError::TokenInvalid(
                "Not a refresh token".to_string(),
            )));
        }

        Ok(claims)
    }

    /// Refresh access token using refresh token
    pub fn refresh_access_token(&self, refresh_token: &str) -> Result<TokenPair> {
        let claims = self.validate_refresh_token(refresh_token)?;
        self.generate_tokens(
            &claims.sub,
            &claims.email,
            &claims.roles,
            claims.tenant.as_deref(),
            &claims.session_id,
        )
    }

    /// Extract token from Authorization header
    pub fn extract_token(auth_header: &str) -> Result<String> {
        if !auth_header.starts_with("Bearer ") {
            return Err(Error::Auth(ferris_core::AuthError::TokenInvalid(
                "Invalid authorization header".to_string(),
            )));
        }
        Ok(auth_header[7..].to_string())
    }

    /// Get token expiry time
    ///
    /// # Errors
    ///
    /// [`AuthError::TokenInvalid`](ferris_core::AuthError::TokenInvalid) when
    /// the token's `exp` claim is not a representable timestamp (AUTH12: this
    /// used to be masked with `Utc::now()`, silently turning a corrupt claim
    /// into a plausible time).
    pub fn get_token_expiry(&self, token: &str) -> Result<DateTime<Utc>> {
        let claims = self.validate_access_token(token)?;
        DateTime::from_timestamp(claims.exp, 0).ok_or_else(|| {
            Error::Auth(ferris_core::AuthError::TokenInvalid(format!(
                "exp claim out of range: {}",
                claims.exp
            )))
        })
    }

    /// Check if token is expired
    pub fn is_token_expired(&self, token: &str) -> bool {
        self.get_token_expiry(token)
            .map_or(true, |exp| exp < Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager() -> JwtManager {
        JwtManager::new(JwtConfig::new("ferris-test-secret-0123456789abcdef").unwrap()).unwrap()
    }

    #[test]
    fn test_jwt_generation() {
        let manager = test_manager();

        let tokens = manager
            .generate_tokens(
                "user1",
                "test@example.com",
                &["User".to_string(), "Admin".to_string()],
                Some("tenant1"),
                "session1",
            )
            .unwrap();

        assert!(!tokens.access_token.is_empty());
        assert!(!tokens.refresh_token.is_empty());
        assert_eq!(tokens.token_type, "Bearer");
    }

    #[test]
    fn test_token_validation() {
        let manager = test_manager();

        let tokens = manager
            .generate_tokens(
                "user1",
                "test@example.com",
                &["User".to_string()],
                None,
                "session1",
            )
            .unwrap();

        let claims = manager.validate_access_token(&tokens.access_token).unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.email, "test@example.com");
        assert_eq!(claims.roles, vec!["User"]);
        assert_eq!(claims.token_type, TokenType::Access);
    }

    #[test]
    fn test_refresh_token() {
        let manager = test_manager();

        let tokens = manager
            .generate_tokens(
                "user1",
                "test@example.com",
                &["User".to_string()],
                None,
                "session1",
            )
            .unwrap();

        let new_tokens = manager.refresh_access_token(&tokens.refresh_token).unwrap();
        assert!(!new_tokens.access_token.is_empty());
        assert!(!new_tokens.refresh_token.is_empty());

        let claims = manager
            .validate_access_token(&new_tokens.access_token)
            .unwrap();
        assert_eq!(claims.sub, "user1");
    }

    #[test]
    fn test_secret_rotation_keeps_old_tokens_valid() {
        let mut manager = test_manager();
        let old = manager
            .generate_tokens(
                "user1",
                "test@example.com",
                &["User".to_string()],
                None,
                "session1",
            )
            .unwrap();

        manager
            .rotate_secret("ferris-rotated-secret-0123456789abcdef")
            .unwrap();

        // Tokens issued before the rotation still validate (old key retained).
        assert_eq!(
            manager
                .validate_access_token(&old.access_token)
                .unwrap()
                .sub,
            "user1"
        );

        // New tokens are signed with the new secret and validate too.
        let new = manager
            .generate_tokens(
                "user2",
                "other@example.com",
                &["User".to_string()],
                None,
                "s2",
            )
            .unwrap();
        assert_eq!(
            manager
                .validate_access_token(&new.access_token)
                .unwrap()
                .sub,
            "user2"
        );

        // A manager with only the new secret cannot verify the old token.
        let fresh =
            JwtManager::new(JwtConfig::new("ferris-rotated-secret-0123456789abcdef").unwrap())
                .unwrap();
        assert!(fresh.validate_access_token(&old.access_token).is_err());
    }

    #[test]
    fn test_token_extraction() {
        let token = JwtManager::extract_token("Bearer abc123").unwrap();
        assert_eq!(token, "abc123");

        assert!(JwtManager::extract_token("Basic abc123").is_err());
    }
}
