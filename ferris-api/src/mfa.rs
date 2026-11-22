//! Authenticated MFA self-service: `POST /api/auth/mfa/{enable,activate,disable}`.
//!
//! the login gate (`ferris-api::session`) *demands* the second factor
//! once an account enables it; these endpoints are how a signed-in user gets
//! there:
//!
//! - **enable** — generate a fresh TOTP secret and single-use backup codes,
//!   persist both **without flipping the flag** (a partially-enabled row is
//!   inert for the gate, so nothing changes until the first code verifies);
//! - **activate** — verify the first TOTP code against the pending secret and
//!   only then flip `mfa_enabled`, so a row can never be "enabled with a
//!   secret nobody controls";
//! - **disable** — verify a TOTP code or burn a backup code, then clear the
//!   secret and the remaining codes.
//!
//! The tolerance window, code length and backup-code count come from the same
//! [`MfaManager`] the login gate uses (wired through
//! [`ApiState::with_auth_config`] to the operator's `AuthConfig`), so the two
//! halves of the flow can never disagree about settings.
//!
//! All three routes run inside the API router's `require_auth` layer and behind
//! its CSRF gate: for cookie sessions the caller must present the session's
//! `X-CSRF-Token` (as every other mutating route since ).

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use ferris_auth::middleware::AuthenticatedUser;
use ferris_auth::{TotpAlgorithm, TotpConfig, UserStore};
use ferris_core::Error;
use ferris_core::ValidationError;

use crate::router::{ApiState, error_response};

/// `POST /api/auth/mfa/enable` — request body (all fields optional).
#[derive(Debug, Deserialize)]
pub struct MfaEnableRequest {
    /// Display name embedded in the otpauth URI (defaults to the user's email).
    #[serde(default)]
    pub account_name: Option<String>,
}

/// `POST /api/auth/mfa/{activate,disable}` — the code proving control of the
/// second factor (a TOTP code, or a single-use backup code for disable).
#[derive(Debug, Deserialize)]
pub struct MfaCodeRequest {
    pub code: String,
}

/// Start enabling MFA: persist a fresh secret + hashed backup codes without
/// activating them.
///
/// The plaintext backup codes are part of **this** response only; from then on
/// only their SHA-256 hashes exist server-side.
pub async fn enable(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Json(req): Json<MfaEnableRequest>,
) -> Response {
    let store = UserStore::new(state.pool.clone());
    let id = &user.0.id;
    match store.mfa_enabled(id).await {
        Ok(true) => {
            return error_response(Error::Validation(ValidationError::Invalid(
                "MFA is already enabled for this account".to_string(),
            )));
        }
        Ok(false) => {}
        Err(e) => return error_response(e),
    }
    let account_name = req
        .account_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| user.0.email.clone());
    let totp = state.mfa.generate_totp_secret();
    let uri = state.mfa.generate_totp_uri(&totp, &account_name);
    let backup_codes = state.mfa.generate_backup_codes();
    // Argon2 is deliberately expensive, and ten codes hashed one after another
    // pushed a debug build past the request timeout. The codes are independent:
    // hash them on the blocking pool at once. The stored hashes keep their
    // order, so the response and the row still line up.
    let mut tasks = Vec::with_capacity(backup_codes.len());
    for code in &backup_codes {
        let code = code.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            UserStore::hash_backup_code(&code)
        }));
    }
    let mut hashes = Vec::with_capacity(tasks.len());
    for task in tasks {
        match task.await {
            Ok(Ok(hash)) => hashes.push(hash),
            Ok(Err(e)) => return error_response(e),
            Err(e) => {
                return error_response(Error::Job(ferris_core::JobError::Failed(format!(
                    "backup code hashing failed: {e}"
                ))));
            }
        }
    }
    if let Err(e) = store.set_mfa(id, false, Some(&totp.secret), &hashes).await {
        return error_response(e);
    }
    Json(serde_json::json!({
        "secret": totp.secret,
        "otpauth_uri": uri,
        "backup_codes": backup_codes,
    }))
    .into_response()
}

/// Verify the first TOTP code and only then mark the account MFA-enabled.
pub async fn activate(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Json(req): Json<MfaCodeRequest>,
) -> Response {
    let store = UserStore::new(state.pool.clone());
    let id = &user.0.id;
    if req.code.trim().is_empty() {
        return error_response(Error::Validation(ValidationError::Invalid(
            "missing MFA code".to_string(),
        )));
    }
    let secret = match store.mfa_secret(id).await {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            return error_response(Error::Validation(ValidationError::Invalid(
                "no MFA secret pending activation; run enable first".to_string(),
            )));
        }
        Err(e) => return error_response(e),
    };
    let totp = TotpConfig {
        secret,
        digits: state.mfa.totp_digits(),
        period: state.mfa.totp_period(),
        algorithm: TotpAlgorithm::Sha1,
    };
    match state.mfa.verify_totp(&totp, &req.code, 1) {
        Ok(true) => {}
        Ok(false) => {
            return error_response(Error::Validation(ValidationError::Invalid(
                "invalid MFA code".to_string(),
            )));
        }
        Err(e) => return error_response(e),
    }
    if let Err(e) = store.set_mfa_enabled(id, true).await {
        return error_response(e);
    }
    Json(serde_json::json!({ "status": "enabled", "id": id })).into_response()
}

/// Disable MFA after proving control of the account's current second factor
/// (a valid TOTP code, or a backup code, which is burnt in the process). The
/// secret and any remaining backup codes are cleared.
pub async fn disable(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Json(req): Json<MfaCodeRequest>,
) -> Response {
    let store = UserStore::new(state.pool.clone());
    let id = &user.0.id;
    if req.code.trim().is_empty() {
        return error_response(Error::Validation(ValidationError::Invalid(
            "missing MFA code".to_string(),
        )));
    }
    // TOTP first, then a single-use backup code (hashed at rest, consumed on
    // match) — the same order the login gate's `verify_mfa` uses.
    let code_ok = match store.mfa_secret(id).await {
        Ok(Some(secret)) => {
            let totp = TotpConfig {
                secret,
                digits: state.mfa.totp_digits(),
                period: state.mfa.totp_period(),
                algorithm: TotpAlgorithm::Sha1,
            };
            match state.mfa.verify_totp(&totp, &req.code, 1) {
                Ok(ok) => ok,
                Err(e) => return error_response(e),
            }
        }
        Ok(None) => false,
        Err(e) => return error_response(e),
    };
    let ok = code_ok
        || match store.consume_mfa_backup_code(id, &req.code).await {
            Ok(consumed) => consumed,
            Err(e) => return error_response(e),
        };
    if !ok {
        return error_response(Error::Validation(ValidationError::Invalid(
            "invalid MFA code".to_string(),
        )));
    }
    if let Err(e) = store.set_mfa(id, false, None, &[]).await {
        return error_response(e);
    }
    Json(serde_json::json!({ "status": "disabled", "id": id })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_auth::{MfaConfig, MfaManager, User, UserType};

    fn auth_user(user: &ferris_auth::StoredUser) -> AuthenticatedUser {
        let mut u = User::new(
            user.email.clone(),
            user.full_name.clone(),
            UserType::SystemUser,
        );
        u.id = user.id.clone();
        u.roles = user.roles.clone();
        AuthenticatedUser(u)
    }

    async fn json_response(response: Response) -> serde_json::Value {
        let (parts, body) = response.into_parts();
        assert_eq!(
            parts.status,
            axum::http::StatusCode::OK,
            "handler answered {}",
            parts.status
        );
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .unwrap_or_default();
        serde_json::from_slice(&bytes).expect("handler answers JSON")
    }

    /// Seed one user into `pool` — the caller keeps the pool so the
    /// `ApiState` under test shares the *same* `:memory:` database (each
    /// `memory_db()` call is a fresh, isolated database).
    async fn seeded_user(pool: ferris_db::Db) -> (ferris_auth::StoredUser, UserStore) {
        let store = UserStore::new(pool);
        store.init().await.unwrap();
        let user = store
            .create(
                "mfa@local",
                "Mfa User",
                Some("mfauser"),
                UserType::SystemUser,
                "ferrisAdmin123",
                &["User".to_string()],
            )
            .await
            .unwrap();
        (user, store)
    }

    fn totp_code(state: &ApiState, secret: &str) -> String {
        let totp = TotpConfig {
            secret: secret.to_string(),
            digits: state.mfa.totp_digits(),
            period: state.mfa.totp_period(),
            algorithm: TotpAlgorithm::Sha1,
        };
        state.mfa.generate_totp(&totp).unwrap()
    }

    /// The full round trip: enable persists an inert secret, activate flips the
    /// flag only after the first code verifies, and disable clears everything
    /// after proving the second factor again.
    #[tokio::test]
    async fn enable_activate_and_disable_round_trip() {
        let pool = ferris_db::memory_db().await.unwrap();
        let (user, store) = seeded_user(pool.clone()).await;
        let state = Arc::new(ApiState::new(pool));
        let me = auth_user(&user);

        // enable: secret + backup codes returned, flag still off.
        let body = json_response(
            enable(
                State(state.clone()),
                me.clone(),
                Json(MfaEnableRequest { account_name: None }),
            )
            .await,
        )
        .await;
        let secret = body["secret"].as_str().unwrap();
        let backup_codes: Vec<String> = body["backup_codes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert_eq!(backup_codes.len(), 10, "default backup code count");
        assert!(!store.mfa_enabled(&user.id).await.unwrap());
        assert_eq!(
            store.mfa_secret(&user.id).await.unwrap().as_deref(),
            Some(secret)
        );

        // activate with a wrong code is refused and the flag stays off.
        let response = activate(
            State(state.clone()),
            me.clone(),
            Json(MfaCodeRequest {
                code: "000000".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(!store.mfa_enabled(&user.id).await.unwrap());

        // The real code activates.
        json_response(
            activate(
                State(state.clone()),
                me.clone(),
                Json(MfaCodeRequest {
                    code: totp_code(&state, secret),
                }),
            )
            .await,
        )
        .await;
        assert!(store.mfa_enabled(&user.id).await.unwrap());

        // Another enable attempt is refused once the account is live.
        let response = enable(
            State(state.clone()),
            me.clone(),
            Json(MfaEnableRequest { account_name: None }),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

        // Disable without a valid code is refused.
        let response = disable(
            State(state.clone()),
            me.clone(),
            Json(MfaCodeRequest {
                code: "000000".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(store.mfa_enabled(&user.id).await.unwrap());

        // A backup code also disables (and is burnt in the process).
        json_response(
            disable(
                State(state.clone()),
                me,
                Json(MfaCodeRequest {
                    code: backup_codes[0].clone(),
                }),
            )
            .await,
        )
        .await;
        assert!(!store.mfa_enabled(&user.id).await.unwrap());
        assert!(store.mfa_secret(&user.id).await.unwrap().is_none());
        assert!(
            !store
                .consume_mfa_backup_code(&user.id, &backup_codes[0])
                .await
                .unwrap(),
            "the disabling backup code was consumed once"
        );
    }

    /// The manager settings the gate runs with drive the self-service window:
    /// a custom code length is honoured end to end.
    #[test]
    fn generated_totp_matches_the_manager_configuration() {
        let manager = MfaManager::new(MfaConfig {
            totp_digits: 8,
            ..MfaConfig::default()
        });
        let totp = manager.generate_totp_secret();
        let code = manager.generate_totp(&totp).unwrap();
        assert_eq!(code.len(), 8);
        assert!(manager.verify_totp(&totp, &code, 1).unwrap());
    }
}
