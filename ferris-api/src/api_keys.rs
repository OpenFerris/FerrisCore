//! API-key lifecycle: `GET`/`POST /api/auth/api-keys` and
//! `DELETE /api/auth/api-keys/{id}`.
//!
//! A key is always scoped to the caller: the owner comes from
//! [`AuthenticatedUser`], never from the body, and a delete only succeeds for a
//! key the caller owns (anything else is a `404`, so an id cannot be probed
//! across accounts). The plaintext secret is returned by `POST` **once**;
//! [`ferris_auth::ApiKey`] skips `secret_hash` when serialised, so listing can
//! never leak it.
//!
//! The routes live in the nested CSRF router next to logout: they mutate the
//! caller's account and must carry the session's `X-CSRF-Token` (API-key callers
//! are exempt, like everywhere else). Every response that can carry a credential
//! is marked `Cache-Control: no-store`.
//!
//! The manager normally comes from [`AuthState::api_keys`], the same instance
//! the authentication middleware verifies with. A state without it (a test, a
//! minimal boot) falls back to a manager over [`ApiState::pool`], initialised on
//! first use.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_auth::{ApiKeyManager, AuthState};
use ferris_core::{AuthError, Error, ValidationError};
use serde::Deserialize;

use crate::router::{ApiState, error_response};

/// `POST /api/auth/api-keys` body.
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    /// Free-form label the desk shows next to the key.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional RFC 3339 deadline. A key past it is refused on every request.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// The manager the auth middleware verifies with, or a fresh one over the API
/// pool when the state carries none.
///
/// # Errors
///
/// Database errors from creating the `API Key` table on the fallback path.
async fn manager(auth: &AuthState, api: &ApiState) -> Result<Arc<ApiKeyManager>, Error> {
    if let Some(manager) = &auth.api_keys {
        return Ok(manager.clone());
    }
    let manager = ApiKeyManager::with_db(api.pool.clone());
    manager.init().await?;
    Ok(Arc::new(manager))
}

/// Mark a response that may carry a credential as uncacheable.
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `GET /api/auth/api-keys` — the caller's keys, without the stored digest.
pub async fn list_api_keys(
    State(auth): State<AuthState>,
    State(api): State<ApiState>,
    user: AuthenticatedUser,
) -> Response {
    let manager = match manager(&auth, &api).await {
        Ok(manager) => manager,
        Err(error) => return error_response(error),
    };
    match manager.list(&user.0.id).await {
        Ok(keys) => no_store(Json(keys).into_response()),
        Err(error) => error_response(error),
    }
}

/// `POST /api/auth/api-keys` — issue a key for the caller.
///
/// Answers `{id, key, secret, expires_at}`. `secret` exists only in this
/// response; the stored form is a digest.
pub async fn create_api_key(
    State(auth): State<AuthState>,
    State(api): State<ApiState>,
    user: AuthenticatedUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> Response {
    let expires_at = match parse_expires_at(req.expires_at.as_deref()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let description = req
        .description
        .map(|description| description.trim().to_string())
        .filter(|description| !description.is_empty());
    let manager = match manager(&auth, &api).await {
        Ok(manager) => manager,
        Err(error) => return error_response(error),
    };
    match manager.create(&user.0.id, description, expires_at).await {
        Ok((key, secret)) => no_store(
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": key.id,
                    "key": key.key,
                    "secret": secret,
                    "expires_at": key.expires_at,
                })),
            )
                .into_response(),
        ),
        Err(error) => error_response(error),
    }
}

/// `DELETE /api/auth/api-keys/{id}` — revoke one of the caller's keys.
///
/// A key that exists but belongs to another account answers the same `404` as
/// an unknown id: the route does not confirm that someone else's key exists.
pub async fn delete_api_key(
    State(auth): State<AuthState>,
    State(api): State<ApiState>,
    user: AuthenticatedUser,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let manager = match manager(&auth, &api).await {
        Ok(manager) => manager,
        Err(error) => return error_response(error),
    };
    let owned = match manager.list(&user.0.id).await {
        Ok(keys) => keys.into_iter().any(|key| key.id == id),
        Err(error) => return error_response(error),
    };
    if !owned {
        return error_response(Error::Auth(AuthError::NotFound(format!("API key {id}"))));
    }
    match manager.revoke(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error_response(error),
    }
}

/// Parse the optional `expires_at` body field.
fn parse_expires_at(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, Response> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let expires_at = DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| {
            error_response(Error::Validation(ValidationError::InvalidValue {
                field: "expires_at".to_string(),
                message: "expected an RFC 3339 timestamp".to_string(),
            }))
        })?;
    if expires_at <= Utc::now() {
        return Err(error_response(Error::Validation(
            ValidationError::InvalidValue {
                field: "expires_at".to_string(),
                message: "must be in the future".to_string(),
            },
        )));
    }
    Ok(Some(expires_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_auth::{User, UserType};

    /// The issue → list → revoke round trip over the fallback manager: the
    /// secret is returned once, never by a list, and the owner check keeps
    /// another account's id a 404.
    #[tokio::test]
    async fn the_issue_list_and_revoke_round_trip_never_leaks_the_hash() {
        crate::test_support::install_test_keyring();
        let pool = ferris_db::memory_db().await.unwrap();
        let api = ApiState::new(pool);
        let auth = AuthState::default();
        let mut owner = User::new(
            "keys@local".to_string(),
            "Key Owner".to_string(),
            UserType::SystemUser,
        );
        owner.id = "key-owner".to_string();
        let me = AuthenticatedUser(owner);

        let response = create_api_key(
            State(auth.clone()),
            State(api.clone()),
            me.clone(),
            Json(CreateApiKeyRequest {
                description: Some("laptop".to_string()),
                expires_at: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        let (_, body) = response.into_parts();
        let body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(body, usize::MAX).await.unwrap()).unwrap();
        let id = body["id"].as_str().unwrap().to_string();
        let secret = body["secret"].as_str().unwrap().to_string();
        assert!(!secret.is_empty());

        let response = list_api_keys(State(auth.clone()), State(api.clone()), me.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let (_, body) = response.into_parts();
        let list = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&list);
        assert!(text.contains(&id), "{text}");
        assert!(!text.contains("secret_hash"), "{text}");
        assert!(!text.contains(&secret), "{text}");

        // Another account cannot revoke it: the id is a 404 for them.
        let mut other = User::new(
            "other@local".to_string(),
            "Other".to_string(),
            UserType::SystemUser,
        );
        other.id = "other-owner".to_string();
        let response = delete_api_key(
            State(auth.clone()),
            State(api.clone()),
            AuthenticatedUser(other),
            axum::extract::Path(id.clone()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = delete_api_key(State(auth), State(api), me, axum::extract::Path(id)).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    /// The body keeps its label; the parser rejects a malformed or past
    /// deadline before a key is written.
    #[test]
    fn expires_at_must_be_a_future_rfc3339_timestamp() {
        assert!(parse_expires_at(None).unwrap().is_none());
        assert!(parse_expires_at(Some("  ")).unwrap().is_none());
        assert!(parse_expires_at(Some("tomorrow")).is_err());
        assert!(parse_expires_at(Some("2000-01-01T00:00:00Z")).is_err());
        assert!(parse_expires_at(Some("2999-01-01T00:00:00Z")).is_ok());
    }

    #[test]
    fn a_response_that_carries_a_credential_is_not_cacheable() {
        let response = no_store(StatusCode::OK.into_response());
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }
}
