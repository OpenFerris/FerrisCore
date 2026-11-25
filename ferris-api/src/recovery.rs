//! Password reset and email verification over HTTP.
//!
//! The flows themselves live in [`ferris_auth::recovery`]; this module is the
//! thin transport around them:
//!
//! * `POST /api/auth/password-reset/request` — public, **always `204`**, so the
//!   status never reveals whether the address has an account. The mail is sent
//!   through [`SessionHandlers::mailer`] when the host attached one; without a
//!   mailer the token is not delivered and the failure is logged.
//! * `POST /api/auth/password-reset/confirm` — public, `204` or `400`.
//! * `POST /api/auth/email/verify/request` — authenticated + CSRF, `204`. An
//!   account that is already verified (or unknown to the service) still gets
//!   `204`: there is nothing to prove, and the caller is the account holder.
//! * `POST /api/auth/email/verify/confirm` — public, `204` or `400`.
//!
//! # Links
//!
//! The mail carries the relative Desk paths `/desk/reset?token=<token>` and
//! `/desk/verify-email?token=<token>`. The absolute origin is a deployment fact
//! (reverse proxy, `external_url`) that this crate does not hold; the Desk
//! resolves the path against whatever origin served the mail's recipient. The
//! token is never logged.
//!
//! # No enumeration
//!
//! Requesting a reset for an unknown address answers exactly like a known one;
//! only the side effect differs. The rate limiter is keyed by
//! `format!("reset:{email}")`, so one address cannot be mail-bombed; a blocked
//! key still answers `204`.
//!
//! # Redaction
//!
//! Every request struct with a secret (the token, the new password) has a manual
//! [`std::fmt::Debug`] that prints `<redacted>` instead.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Duration;
use ferris_auth::middleware::AuthenticatedUser;
use ferris_auth::{EmailVerificationService, PasswordResetService, recovery::VerificationToken};
use ferris_core::ports::EmailMessage;
use ferris_core::{Error, ValidationError};
use serde::Deserialize;
use tracing::warn;

use crate::SessionHandlers;
use crate::router::error_response;

/// How long a password-reset link stays valid.
const RESET_TTL: Duration = Duration::hours(1);

/// How long an email-verification link stays valid. Long, because verifying is
/// only ever a convenience and the account already exists.
const VERIFICATION_TTL: Duration = Duration::hours(48);

/// `POST /api/auth/password-reset/request` body.
#[derive(Deserialize)]
pub struct PasswordResetRequest {
    pub email: String,
}

/// The address is not a credential; it stays visible for operator correlation.
impl std::fmt::Debug for PasswordResetRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordResetRequest")
            .field("email", &self.email)
            .finish()
    }
}

/// `POST /api/auth/password-reset/confirm` body.
#[derive(Deserialize)]
pub struct PasswordResetConfirmRequest {
    pub token: String,
    pub new_password: String,
}

impl std::fmt::Debug for PasswordResetConfirmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordResetConfirmRequest")
            .field("token", &"<redacted>")
            .field("new_password", &"<redacted>")
            .finish()
    }
}

/// `POST /api/auth/email/verify/confirm` body.
#[derive(Deserialize)]
pub struct EmailVerificationConfirmRequest {
    pub token: String,
}

impl std::fmt::Debug for EmailVerificationConfirmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailVerificationConfirmRequest")
            .field("token", &"<redacted>")
            .finish()
    }
}

/// The generic answer of an anonymous flow: no body, no cache.
fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// A `400` that says only "this token is not usable", never which check failed.
fn unusable_token(what: &str) -> Response {
    error_response(Error::Validation(ValidationError::Invalid(format!(
        "{what} token is invalid or expired"
    ))))
}

/// Send one plain-text message through the host mailer.
///
/// Returns `false` when there is no mailer or the transport refused the message;
/// the caller decides whether that is a `warn` (it is: the flow must not fail
/// because email is not configured).
fn send_mail(handlers: &SessionHandlers, to: &str, subject: &str, text: String) -> bool {
    let Some(mailer) = handlers.mailer() else {
        warn!("no host mailer is attached; the auth email was not sent");
        return false;
    };
    match mailer.send(&EmailMessage::new(to, subject, text)) {
        Ok(()) => true,
        Err(error) => {
            warn!(%error, "the host mailer refused an auth message");
            false
        }
    }
}

/// The reset message. The token appears in the body only, never in the logs.
fn reset_message(token: &str) -> String {
    format!(
        "Somebody asked to reset the password of this Ferris account.\n\n\
         Open /desk/reset?token={token} to choose a new one. The link expires in one hour.\n\n\
         If it was not you, ignore this message: the password stays unchanged.\n"
    )
}

/// The verification message.
fn verification_message(token: &str) -> String {
    format!(
        "Open /desk/verify-email?token={token} to verify this address.\n\n\
         The link expires in 48 hours.\n"
    )
}

/// `POST /api/auth/password-reset/request` — mint a token and mail the link.
///
/// Always `204`: an unknown address, a disabled account, a blocked rate limiter
/// and a storage error all answer the same, so the endpoint cannot enumerate
/// accounts. The token is only handled when the service issued one.
pub async fn request_password_reset(
    State(handlers): State<SessionHandlers>,
    Json(req): Json<PasswordResetRequest>,
) -> Response {
    let email = req.email.trim();
    if email.is_empty() {
        return no_content();
    }
    let key = format!("reset:{email}");
    if handlers.rate_limiter.is_blocked(&key) {
        return no_content();
    }
    // Count every request, not just failures: the point is to bound the number
    // of links one address can receive.
    handlers.rate_limiter.record_failure(&key);

    let service = PasswordResetService::new(handlers.user_store.clone());
    match service.request_reset(email, RESET_TTL).await {
        Ok(Some(token)) => {
            send_mail(
                &handlers,
                &email,
                "Reset your Ferris password",
                reset_message(&token.token),
            );
        }
        Ok(None) => {}
        Err(error) => warn!(%error, "could not issue a password reset token"),
    }
    no_content()
}

/// `POST /api/auth/password-reset/confirm` — spend the token and set the
/// password. `204` on success, `400` for an unknown/expired/used token or a
/// password the policy refuses.
pub async fn confirm_password_reset(
    State(handlers): State<SessionHandlers>,
    Json(req): Json<PasswordResetConfirmRequest>,
) -> Response {
    let service = PasswordResetService::new(handlers.user_store.clone());
    match service.confirm_reset(&req.token, &req.new_password).await {
        Ok(Some(_)) => no_content(),
        Ok(None) => unusable_token("password reset"),
        Err(error) => error_response(error),
    }
}

/// `POST /api/auth/email/verify/request` — mail a verification link to the
/// signed-in account's address.
///
/// An account that is already verified has nothing pending, so the service
/// answers `None` and the route still answers `204` (it is the account holder
/// asking, not a probe).
pub async fn request_email_verification(
    State(handlers): State<SessionHandlers>,
    user: AuthenticatedUser,
) -> Response {
    let service = EmailVerificationService::new(handlers.user_store.clone());
    match service
        .request_verification(&user.0.id, VERIFICATION_TTL)
        .await
    {
        Ok(Some(token)) => send_verification_mail(&handlers, &user.0.email, &token),
        Ok(None) => {}
        Err(error) => return error_response(error),
    }
    no_content()
}

/// Deliver the verification link. Kept separate so the handler stays one
/// question long.
fn send_verification_mail(handlers: &SessionHandlers, email: &str, token: &VerificationToken) {
    send_mail(
        handlers,
        email,
        "Verify your Ferris email",
        verification_message(&token.token),
    );
}

/// `POST /api/auth/email/verify/confirm` — mark the address verified.
/// `204` on success, `400` for an unknown/expired/used token.
pub async fn confirm_email_verification(
    State(handlers): State<SessionHandlers>,
    Json(req): Json<EmailVerificationConfirmRequest>,
) -> Response {
    let service = EmailVerificationService::new(handlers.user_store.clone());
    match service.confirm_verification(&req.token).await {
        Ok(Some(_)) => no_content(),
        Ok(None) => unusable_token("email verification"),
        Err(error) => error_response(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Neither the token nor the new password may appear in a formatted request.
    #[test]
    fn request_debug_redacts_credentials() {
        let request = PasswordResetConfirmRequest {
            token: "super-secret-token".to_string(),
            new_password: "hunter2".to_string(),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("super-secret-token"), "{debug}");
        assert!(!debug.contains("hunter2"), "{debug}");

        let request = EmailVerificationConfirmRequest {
            token: "super-secret-token".to_string(),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("super-secret-token"), "{debug}");

        let request = PasswordResetRequest {
            email: "ada@example.com".to_string(),
        };
        assert!(format!("{request:?}").contains("ada@example.com"));
    }

    /// The link is the Desk path with the token, and nothing else.
    #[test]
    fn the_messages_carry_the_desk_link() {
        assert!(reset_message("abc").contains("/desk/reset?token=abc"));
        assert!(verification_message("abc").contains("/desk/verify-email?token=abc"));
    }

    /// Requesting a reset for an address with no account answers exactly like a
    /// real one, and an unusable token is a `400`, not a leak.
    #[tokio::test]
    async fn an_unknown_address_and_a_bad_token_do_not_reveal_anything() {
        crate::test_support::install_test_keyring();
        let pool = ferris_db::memory_db().await.unwrap();
        ferris_auth::UserStore::new(pool.clone())
            .init()
            .await
            .unwrap();
        let handlers = SessionHandlers::without_store(ferris_auth::SessionConfig::default(), pool);

        let response = request_password_reset(
            State(handlers.clone()),
            Json(PasswordResetRequest {
                email: "nobody@local".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = confirm_password_reset(
            State(handlers),
            Json(PasswordResetConfirmRequest {
                token: "not-a-token".to_string(),
                new_password: "ferrisAdmin123".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
