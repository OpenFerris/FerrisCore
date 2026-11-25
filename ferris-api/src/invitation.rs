//! Invitation administration and public acceptance.
//!
//! * `POST /api/auth/invitations` — `System Manager` only. Issues the token,
//!   mails the `/desk/accept-invitation?token=...` link and answers `201` with
//!   the invitation as the desk sees it (never the token or its digest).
//! * `GET /api/auth/invitations` — `System Manager` only, same shape.
//! * `DELETE /api/auth/invitations/{id}` — `System Manager` only, `204` or
//!   `404`.
//! * `POST /api/auth/invitations/accept` — public: `{token, full_name, password}`
//!   creates the account through [`InvitationStore::accept`], `204` or `400`.
//!
//! # No enumeration
//!
//! Acceptance answers the same `400` for an unknown, expired, revoked or
//! already-used token and for an address that already has an account: the
//! service deliberately collapses those cases. It never echoes the invitation
//! or the address.
//!
//! # Tokens
//!
//! [`ferris_auth::Invitation`] has no token field at all; only its SHA-256
//! digest is stored. The plaintext is handed to the mailer once and never
//! logged.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Duration;
use ferris_auth::middleware::AuthenticatedUser;
use ferris_auth::{Invitation, InvitationStore};
use ferris_core::ports::EmailMessage;
use ferris_core::{AuthzError, Error, ValidationError};
use serde::Deserialize;
use tracing::warn;

use crate::SessionHandlers;
use crate::router::{ApiState, error_response};

/// The role that may issue, list and revoke invitations.
const ADMIN_ROLE: &str = "System Manager";

/// Default lifetime of an invitation: two days.
const DEFAULT_TTL_SECONDS: u64 = 48 * 60 * 60;

/// Longest accepted lifetime, so a link cannot be valid for a year by accident.
const MAX_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

/// `POST /api/auth/invitations` body.
#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    pub email: String,
    #[serde(default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

/// `POST /api/auth/invitations/accept` body.
#[derive(Deserialize)]
pub struct AcceptInvitationRequest {
    pub token: String,
    pub full_name: String,
    pub password: String,
}

impl std::fmt::Debug for AcceptInvitationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptInvitationRequest")
            .field("token", &"<redacted>")
            .field("full_name", &self.full_name)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Refuse a caller without the admin role.
fn require_admin(user: &AuthenticatedUser) -> Result<(), Response> {
    if user.0.has_role(ADMIN_ROLE) {
        return Ok(());
    }
    Err(error_response(Error::Authz(AuthzError::InsufficientRole {
        required: ADMIN_ROLE.to_string(),
        actual: user.0.roles.join(", "),
    })))
}

/// Serialise an invitation without its token fields (there are none to leak).
fn invitation_json(invitation: &Invitation) -> serde_json::Value {
    serde_json::json!({
        "id": invitation.id,
        "email": invitation.email,
        "full_name": invitation.full_name,
        "roles": invitation.roles,
        "invited_by": invitation.invited_by,
        "expires_at": invitation.expires_at.map(|value| value.to_rfc3339()),
        "accepted": invitation.accepted,
        "accepted_at": invitation.accepted_at.map(|value| value.to_rfc3339()),
    })
}

/// The lifetime the body asks for, or the default.
fn invitation_ttl(seconds: Option<u64>) -> Result<Duration, Response> {
    let seconds = seconds.unwrap_or(DEFAULT_TTL_SECONDS);
    if seconds == 0 || seconds > MAX_TTL_SECONDS {
        return Err(error_response(Error::Validation(
            ValidationError::InvalidValue {
                field: "ttl_seconds".to_string(),
                message: format!("must be between 1 and {MAX_TTL_SECONDS}"),
            },
        )));
    }
    let seconds = i64::try_from(seconds).map_err(|_| {
        error_response(Error::Validation(ValidationError::InvalidValue {
            field: "ttl_seconds".to_string(),
            message: "is out of range".to_string(),
        }))
    })?;
    Ok(Duration::seconds(seconds))
}

/// The invitation mail. The token appears in the body only.
fn invitation_message(token: &str) -> String {
    format!(
        "You have been invited to a Ferris workspace.\n\n\
         Open /desk/accept-invitation?token={token} to create your account.\n\n\
         If you were not expecting this message, ignore it.\n"
    )
}

/// Send the invitation link through the host mailer.
fn send_invitation(handlers: &SessionHandlers, to: &str, token: &str) {
    let Some(mailer) = handlers.mailer() else {
        warn!("no host mailer is attached; the invitation was not emailed");
        return;
    };
    if let Err(error) = mailer.send(&EmailMessage::new(
        to,
        "You have been invited to Ferris",
        invitation_message(token),
    )) {
        warn!(%error, "the host mailer refused an invitation");
    }
}

/// `POST /api/auth/invitations` — issue an invitation and mail the link.
pub async fn create_invitation(
    State(api): State<ApiState>,
    State(handlers): State<SessionHandlers>,
    user: AuthenticatedUser,
    Json(req): Json<CreateInvitationRequest>,
) -> Response {
    if let Err(response) = require_admin(&user) {
        return response;
    }
    let ttl = match invitation_ttl(req.ttl_seconds) {
        Ok(ttl) => ttl,
        Err(response) => return response,
    };
    let store = InvitationStore::new(api.pool.clone());
    let created = store
        .create(
            &req.email,
            req.full_name.as_deref(),
            &req.roles,
            ttl,
            Some(&user.0.id),
        )
        .await;
    match created {
        Ok((invitation, token)) => {
            send_invitation(&handlers, &invitation.email, &token);
            (StatusCode::CREATED, Json(invitation_json(&invitation))).into_response()
        }
        Err(error) => error_response(error),
    }
}

/// `GET /api/auth/invitations` — every invitation, token-free.
pub async fn list_invitations(State(api): State<ApiState>, user: AuthenticatedUser) -> Response {
    if let Err(response) = require_admin(&user) {
        return response;
    }
    let store = InvitationStore::new(api.pool.clone());
    match store.list().await {
        Ok(invitations) => {
            let body: Vec<serde_json::Value> = invitations.iter().map(invitation_json).collect();
            Json(body).into_response()
        }
        Err(error) => error_response(error),
    }
}

/// `DELETE /api/auth/invitations/{id}` — revoke one invitation.
pub async fn delete_invitation(
    State(api): State<ApiState>,
    user: AuthenticatedUser,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_admin(&user) {
        return response;
    }
    let store = InvitationStore::new(api.pool.clone());
    match store.revoke(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error_response(error),
    }
}

/// `POST /api/auth/invitations/accept` — create the account from the token.
///
/// Public by design: the invited person has no session yet. `400` covers every
/// unusable token and every rejected password, without telling them apart.
pub async fn accept_invitation(
    State(api): State<ApiState>,
    Json(req): Json<AcceptInvitationRequest>,
) -> Response {
    let store = InvitationStore::new(api.pool.clone());
    match store
        .accept(&req.token, &req.full_name, &req.password)
        .await
    {
        Ok(Some(_)) => StatusCode::NO_CONTENT.into_response(),
        Ok(None) => error_response(Error::Validation(ValidationError::Invalid(
            "the invitation is invalid, expired or already used".to_string(),
        ))),
        Err(error) => error_response(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_request_debug_redacts_the_token_and_password() {
        let request = AcceptInvitationRequest {
            token: "super-secret-token".to_string(),
            full_name: "Ada Lovelace".to_string(),
            password: "hunter2".to_string(),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("super-secret-token"), "{debug}");
        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(debug.contains("Ada Lovelace"), "{debug}");
    }

    #[test]
    fn only_a_system_manager_passes_the_admin_gate() {
        use ferris_auth::{User, UserType};

        let mut user = User::new(
            "ada@local".to_string(),
            "Ada".to_string(),
            UserType::SystemUser,
        );
        assert!(require_admin(&AuthenticatedUser(user.clone())).is_err());
        user.roles = vec![ADMIN_ROLE.to_string()];
        assert!(require_admin(&AuthenticatedUser(user)).is_ok());
    }

    #[test]
    fn an_invitation_lifetime_is_bounded() {
        assert_eq!(invitation_ttl(None).unwrap(), Duration::seconds(172_800));
        assert!(invitation_ttl(Some(0)).is_err());
        assert!(invitation_ttl(Some(MAX_TTL_SECONDS + 1)).is_err());
        assert!(invitation_ttl(Some(60)).is_ok());
    }
}
