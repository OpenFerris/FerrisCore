//! Ferris Auth - Authentication and authorization

pub mod api_key;
pub mod cors;
pub mod crypto;
pub mod docperm;
pub mod doctypes;
pub mod jwt;
pub mod mfa;
pub mod middleware;
pub mod oauth;
pub mod password;
pub mod permissions;
pub mod session;
mod store_support;
pub mod user;
pub mod user_permission;
pub mod user_store;
pub mod workspaces;

pub use api_key::{ApiKey, ApiKeyManager};
pub use cors::CorsPolicy;
pub use docperm::{DocPermRow, DocPermStore};
pub use jwt::{Claims, JwtConfig, JwtManager};
pub use mfa::{MAX_TOTP_DIGITS, MfaConfig, MfaManager, TotpAlgorithm, TotpConfig};
pub use middleware::{
    AuthState, CSRF_HEADER, RateLimiter, UserValidator, require_auth, require_csrf,
    resolve_principal,
};
pub use oauth::{
    OAuthClient, OAuthConfig, OAuthManager, OAuthProviderConfig, PkceMethod, StateStoreConfig,
    TokenResponse,
};
pub use password::{PasswordConfig, hash_password, hash_secret, verify_password};
pub use permissions::{
    Decision, Effect, Permission, PermissionEngine, PermissionRule, Policy, PolicyStatement,
    Principal, PrincipalType, Resource, check_permission,
};
pub use session::{Session, SessionConfig, SessionManager};
pub use user::{User, UserRole, UserType};
pub use user_permission::UserPermissionStore;
pub use user_store::{StoredUser, UserStore};
