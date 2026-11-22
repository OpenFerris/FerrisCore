use super::manager::PendingState;
use super::*;
use ferris_core::Error;

#[test]
fn test_oauth_config() {
    let config = OAuthConfig::default();
    assert!(config.providers.is_empty());
    assert!(config.state_store.enabled);
}

fn provider_with_token_url(token_url: String) -> OAuthProviderConfig {
    OAuthProviderConfig {
        name: "test".to_string(),
        client_id: "client".to_string(),
        client_secret: "secret".to_string(),
        auth_url: "https://provider.example/auth".to_string(),
        token_url,
        user_info_url: None,
        device_auth_url: None,
        scopes: vec!["openid".to_string()],
        pkce_enabled: false,
        pkce_method: PkceMethod::S256,
        redirect_uris: vec!["https://app.example/callback".to_string()],
    }
}

/// AUTH4: a disabled state store must not brick the callback — the provider
/// named in the path is used, and no state entry is required. (The old
/// `callback` always required the entry, so `enabled = false` made every
/// login fail with "Invalid or expired state".)
#[tokio::test]
async fn a_disabled_state_store_still_completes_the_callback() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "stateless-access",
            "token_type": "Bearer",
            "expires_in": 600
        })))
        .mount(&server)
        .await;

    let provider = provider_with_token_url(format!("{}/token", server.uri()));
    let mut config = OAuthConfig {
        providers: vec![provider],
        state_store: StateStoreConfig {
            enabled: false,
            ttl_seconds: 600,
        },
    };
    config.state_store.enabled = false;
    let mut manager = OAuthManager::new(config).unwrap();

    // `authorize` succeeds but stores nothing.
    let (_, oauth_state) = manager.authorize("test").unwrap();
    assert!(
        manager.state_store.is_empty(),
        "disabled store keeps nothing"
    );

    // The callback still resolves the provider from the request path.
    let tokens = manager
        .callback("test", oauth_state, "code".to_string())
        .await
        .expect("stateless mode completes the exchange");
    assert_eq!(tokens.access_token, "stateless-access");
}

/// AUTH4: `authorize` prunes expired states, so an abandoned login cannot
/// grow the in-memory store without bound.
#[test]
fn authorize_prunes_expired_states() {
    let mut manager = OAuthManager::new(OAuthConfig {
        providers: vec![provider_with_token_url(
            "https://provider.example/token".to_string(),
        )],
        state_store: StateStoreConfig {
            enabled: true,
            ttl_seconds: 600,
        },
    })
    .unwrap();

    // Seed an already-expired entry, then run a fresh authorize.
    manager.state_store.insert(
        "stale".to_string(),
        PendingState {
            provider: "test".to_string(),
            pkce_verifier: None,
            expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
        },
    );
    let (_, state) = manager.authorize("test").unwrap();

    assert!(!manager.state_store.contains_key("stale"));
    assert!(manager.state_store.contains_key(&state));
}

/// AUTH1: `refresh_token` must perform the real RFC 6749 §6 exchange, not
/// return a hardcoded placeholder. The stub asserts the request body is the
/// refresh grant and the reply is echoed back verbatim.
#[tokio::test]
async fn refresh_token_performs_the_real_grant() {
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=old-refresh"))
        .and(body_string_contains("client_id=client"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "new-access",
            "token_type": "Bearer",
            "expires_in": 1200,
            "refresh_token": "new-refresh"
        })))
        .mount(&server)
        .await;

    let client =
        OAuthClient::new(provider_with_token_url(format!("{}/token", server.uri()))).unwrap();

    let tokens = client
        .refresh_token("old-refresh".to_string())
        .await
        .expect("the refresh grant succeeds");
    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("new-refresh"));
    assert_eq!(tokens.expires_in, 1200);
    assert!(!tokens.access_token.contains("placeholder"));
}

/// AUTH1: a rejected grant is a typed `OAuthTokenRefresh` error, never a
/// fabricated success.
#[tokio::test]
async fn a_rejected_refresh_is_a_typed_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant"))
        .mount(&server)
        .await;

    let client =
        OAuthClient::new(provider_with_token_url(format!("{}/token", server.uri()))).unwrap();

    let err = client
        .refresh_token("bad".to_string())
        .await
        .expect_err("a 400 must not be a fake success");
    assert!(
        matches!(
            err,
            Error::Auth(ferris_core::AuthError::OAuthTokenRefresh(_))
        ),
        "expected OAuthTokenRefresh, got {err:?}"
    );
}
