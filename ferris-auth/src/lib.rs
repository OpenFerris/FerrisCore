//! Ferris Auth - Authentication and authorization

pub mod api_key;
pub mod crypto;
pub mod docperm;
pub mod doctypes;
pub mod invitation;
pub mod login;
pub mod mfa;
pub mod middleware;
#[cfg(feature = "oauth")]
pub mod oauth;
pub mod password;
pub mod permissions;
pub mod recovery;
mod store_support;
mod totp;
pub mod user;
pub mod user_permission;
pub mod user_store;
pub mod workspaces;

pub use api_key::{ApiKey, ApiKeyManager};
pub use docperm::{DocPermRow, DocPermStore};
pub use invitation::{Invitation, InvitationStore};
pub use login::{Backend, Credentials, DbSessionStore, SessionConfig, SessionUser};
pub use mfa::{MAX_TOTP_DIGITS, MfaConfig, MfaManager, TotpAlgorithm, TotpConfig};
pub use middleware::{
    AuthState, CSRF_HEADER, RateLimiter, UserValidator, require_auth, require_csrf,
    resolve_principal,
};
#[cfg(feature = "oauth")]
pub use oauth::{
    OAuthClient, OAuthConfig, OAuthManager, OAuthProviderConfig, PkceMethod, StateStoreConfig,
    TokenResponse,
};
pub use password::{PasswordConfig, hash_password, hash_secret, verify_password};
pub use permissions::{
    Decision, Effect, Permission, PermissionEngine, PermissionRule, Policy, PolicyStatement,
    Principal, PrincipalType, Resource, check_permission,
};
pub use recovery::{EmailVerificationService, PasswordResetService, ResetToken, VerificationToken};
pub use user::{User, UserRole, UserType};
pub use user_permission::UserPermissionStore;
pub use user_store::{StoredUser, UserStore};
