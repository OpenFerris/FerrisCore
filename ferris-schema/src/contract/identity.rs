//! The identity the Desk renders: the body of `POST /api/auth/login` and
//! `GET /api/auth/session` (`ferris_auth::UserStore::public_json`).
//!
//! Intentionally narrower than `ferris_auth::User`: this is what a browser is
//! told about itself. Password hashes, salts, MFA secrets and session rows stay
//! server side, so the hydrate build can deserialize a session without pulling
//! the native auth stack (sqlx, argon2) into the wasm graph.
//!
//! `user_type` stays a string rather than an enum because it is whatever
//! `UserType` serialized to on the way out (`"Guest"`, `"SystemUser"`,
//! `"WebsiteUser"`); modelling it here would put a second, drifting
//! enumeration of the same concept on the client.

use serde::{Deserialize, Serialize};

/// Identity surface the Desk renders (avatar, full name, email, roles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebUser {
    pub id: String,
    pub email: String,
    pub full_name: String,
    pub username: Option<String>,
    pub roles: Vec<String>,
    pub user_type: String,
    /// The account's chosen locale; `None` follows `Accept-Language`.
    #[serde(default)]
    pub language: Option<String>,
}

impl WebUser {
    /// Guest identity, used by the SSR shell and as the hydrate starting state.
    ///
    /// SSR renders the guest and hydrate renders the guest again before the
    /// session check resolves; any divergence between the two would be a
    /// hydration mismatch.
    pub fn guest() -> Self {
        Self {
            id: "guest".into(),
            email: String::new(),
            full_name: "Guest".into(),
            username: Some("Guest".into()),
            roles: vec!["Guest".into()],
            user_type: "Guest".into(),
            language: None,
        }
    }

    /// Whether this is the unauthenticated placeholder rather than a real
    /// session. The Desk gates the whole chrome behind it.
    pub fn is_guest(&self) -> bool {
        self.user_type == "Guest"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chrome is hidden for the guest and shown for everyone else, so a
    /// real session must never read as a guest.
    #[test]
    fn only_the_guest_placeholder_reads_as_guest() {
        assert!(WebUser::guest().is_guest());

        let signed_in = WebUser {
            user_type: "SystemUser".into(),
            ..WebUser::guest()
        };
        assert!(!signed_in.is_guest());
    }

    /// Exactly the body `public_json()` emits: `username` may be null, every
    /// other key is always present.
    #[test]
    fn the_session_body_deserializes_into_the_contract() {
        let user: WebUser = serde_json::from_value(serde_json::json!({
            "id": "ada@example.com",
            "email": "ada@example.com",
            "username": null,
            "full_name": "Ada Lovelace",
            "roles": ["System Manager"],
            "user_type": "SystemUser",
        }))
        .expect("session body parses");

        assert_eq!(user.roles, vec!["System Manager".to_string()]);
        assert_eq!(user.username, None);
        assert!(!user.is_guest());
    }
}
