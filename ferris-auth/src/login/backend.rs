//! The `axum-login` backend: password verification, TOTP and rehashing.
//!
//! One `authenticate` call is the whole first factor and, when the account has
//! it, the second factor. Every rejection — unknown account, wrong password,
//! missing/wrong/replayed TOTP code, disabled account — answers `Ok(None)`, so
//! a handler can only ever render one generic "invalid credentials" response.
//! The typed [`LoginError`] is reserved for infrastructure failures (the store
//! could not be read, the write of a rehashed password failed hard enough to be
//! reported); it never says *why* the credentials were refused.

use std::sync::LazyLock;

use axum_login::{AuthnBackend, UserId};
use ferris_core::Error;
use ferris_db::Db;
use serde::{Deserialize, Serialize};

use crate::password::{PasswordConfig, needs_rehash};
use crate::user_store::{LoginRecord, UserStore};

use super::user::SessionUser;

/// The plaintext burned to make an unknown account cost the same Argon2 work as
/// a real one. It is not a credential anyone holds.
const TIMING_EQUALISER_SECRET: &str = "ferris-login-timing-equaliser-not-a-password";

/// An Argon2 hash of [`TIMING_EQUALISER_SECRET`], derived once with the same
/// parameters as the installed hasher.
///
/// Derived lazily so the (expensive) hash is not computed at process start for
/// deployments that never see a failed login. If hashing fails — it cannot, the
/// parameters are the crate defaults — the empty string makes verification
/// fail, which is the safe direction: the login is still refused.
static TIMING_EQUALISER_HASH: LazyLock<String> = LazyLock::new(|| {
    crate::password::hash_secret_default(TIMING_EQUALISER_SECRET).unwrap_or_default()
});

/// Credentials submitted to the login route.
///
/// `totp_code` is `None` for accounts without MFA and for a first attempt whose
/// form did not carry a code; the backend never asks for one speculatively.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    /// Email address or username, as the account store resolves both.
    pub email: String,
    /// The plaintext password. Never logged, never stored.
    pub password: String,
    /// The six-digit TOTP code, when the account has MFA enabled.
    pub totp_code: Option<String>,
}

/// `Debug` is redacted: a handler that logs its request must not publish the
/// password or the current TOTP code. The email stays, because it is the only
/// field an operator can use to correlate a failure.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("totp_code", &self.totp_code.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// A failure of the login *infrastructure*.
///
/// Wrong credentials are [`Ok(None)`](AuthnBackend::authenticate), not this.
/// The box keeps the error small enough to return by value from every layer
/// without tripping `clippy::result_large_err`.
#[derive(Debug)]
pub struct LoginError(Box<Error>);

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "login backend failure: {}", self.0)
    }
}

impl std::error::Error for LoginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl From<Error> for LoginError {
    fn from(error: Error) -> Self {
        Self(Box::new(error))
    }
}

/// The [`AuthnBackend`] a server wires with
/// [`axum_login::AuthManagerLayerBuilder`].
///
/// Cheap to clone: the store is a pool handle, so the layer can clone the
/// backend per request.
#[derive(Debug, Clone)]
pub struct Backend {
    store: UserStore,
    password_config: PasswordConfig,
}

impl Backend {
    /// Build a backend over `db`.
    ///
    /// The user table is created by [`Backend::init`]; boot-time wiring calls
    /// it once, the same way it calls `UserStore::init` today.
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self::with_store(UserStore::new(db))
    }

    /// Build a backend over an already-configured store.
    #[must_use]
    pub fn with_store(store: UserStore) -> Self {
        Self {
            store,
            password_config: PasswordConfig::default(),
        }
    }

    /// Replace the Argon2 parameters used to decide whether a stored hash is
    /// stale.
    ///
    /// Defaults to [`PasswordConfig::default`], which is what
    /// [`UserStore::init`] installs as the process-wide hasher
    /// (`hash_password_default`). Only change it together with a hasher
    /// installed through [`ferris_core::password::set_password_hasher`] using
    /// the same parameters — otherwise every login rehashes and the stored hash
    /// never converges.
    #[must_use]
    pub const fn with_password_config(mut self, password_config: PasswordConfig) -> Self {
        self.password_config = password_config;
        self
    }

    /// The account store this backend reads.
    #[must_use]
    pub const fn store(&self) -> &UserStore {
        &self.store
    }

    /// Create the user table (idempotent) and install the password hasher.
    ///
    /// # Errors
    ///
    /// Database errors from the table/index creation.
    pub async fn init(&self) -> Result<(), LoginError> {
        self.store.init().await?;
        Ok(())
    }

    /// Verify only the first factor: the password, not TOTP.
    ///
    /// The Desk's login route uses this when an account has MFA enabled: it
    /// must know the password was right before it can ask for a code, while
    /// [`AuthnBackend::authenticate`] deliberately refuses to answer at all
    /// without the second factor. The returned [`SessionUser`] is not logged in
    /// by itself; the caller stores a pending challenge and completes the
    /// session with `AuthSession::login` after the code checks out.
    ///
    /// Rehashing happens here, so the record read back on the MFA step already
    /// carries the fresh hash and the session stamp matches it.
    ///
    /// # Errors
    ///
    /// Infrastructure failures only (the store could not be read/written in a
    /// way the rehash reports). Wrong credentials are `Ok(None)`.
    pub async fn authenticate_password(
        &self,
        creds: &Credentials,
    ) -> Result<Option<SessionUser>, LoginError> {
        let Some(record) = self.store.login_record(&creds.email).await? else {
            self.equalise_unknown_login(&creds.password).await;
            return Ok(None);
        };
        self.authenticate_record(record, &creds.password).await
    }

    /// Verify the **first factor** against an already-resolved record.
    ///
    /// The handler resolves the account once (for the lockout key, the MFA
    /// decision and the audit) and would otherwise make [`Backend::authenticate`]
    /// read the very same row again. This entry point consumes that record:
    /// verify the password, rehash it when the stored one is stale, and build
    /// the [`SessionUser`]. TOTP is *not* checked here — the caller owns the
    /// second-factor flow (the pending challenge), exactly as with
    /// [`Backend::authenticate_password`].
    ///
    /// # Errors
    ///
    /// Infrastructure failures only (the store could not be read/written in a
    /// way the rehash reports). Wrong credentials are `Ok(None)`.
    pub async fn authenticate_record(
        &self,
        record: LoginRecord,
        password: &str,
    ) -> Result<Option<SessionUser>, LoginError> {
        if !verify_password(password, &record.password_hash).await {
            return Ok(None);
        }
        let record = self.rehash_if_stale(record, password).await?;
        Ok(Some(SessionUser::new(
            record.user.to_user(),
            record.session_auth_hash(),
        )))
    }

    /// Spend the same Argon2 work a real account costs on an **unresolved**
    /// login, so an unknown name cannot be told apart from a wrong password by
    /// timing.
    ///
    /// Callers that resolve the account first (and therefore already know there
    /// is no record to verify) use this instead of re-entering
    /// [`AuthnBackend::authenticate`] just for the equaliser.
    pub async fn equalise_unknown_login(&self, password: &str) {
        verify_password(password, &TIMING_EQUALISER_HASH).await;
    }

    /// Re-derive the stored hash when it was written under older parameters.
    ///
    /// A failed rehash never fails the login: the password has already been
    /// verified, and refusing the session would turn a maintenance operation
    /// into an outage. On success the row is read back, because the session
    /// stamp is derived from the *new* hash — using the old one would make the
    /// very next request invalidate the session (AUTH self-lockout).
    async fn rehash_if_stale(
        &self,
        record: LoginRecord,
        password: &str,
    ) -> Result<LoginRecord, LoginError> {
        let stale = match needs_rehash(&record.password_hash, &self.password_config) {
            Ok(stale) => stale,
            Err(error) => {
                // Unreachable after a successful verify (the hash parsed), but a
                // warning is cheaper than assuming.
                tracing::warn!(%error, "stored password hash is unusable for the rehash check");
                return Ok(record);
            }
        };
        if !stale {
            return Ok(record);
        }
        if let Err(error) = self.store.set_password(&record.user.id, password).await {
            tracing::warn!(%error, user.id = %record.user.id, "could not rehash the password on login");
            return Ok(record);
        }
        match self.store.login_record_by_id(&record.user.id).await {
            Ok(Some(updated)) => Ok(updated),
            Ok(None) => Ok(record),
            Err(error) => {
                tracing::warn!(%error, user.id = %record.user.id, "could not read back the rehashed password");
                Ok(record)
            }
        }
    }
}

impl AuthnBackend for Backend {
    type User = SessionUser;
    type Credentials = Credentials;
    type Error = LoginError;

    async fn authenticate(
        &self,
        creds: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        let Some(record) = self.store.login_record(&creds.email).await? else {
            // An unknown account must not be distinguishable from a wrong
            // password by response *or* by timing: spend the same Argon2 work
            // before answering `None`.
            self.equalise_unknown_login(&creds.password).await;
            return Ok(None);
        };

        if !verify_password(&creds.password, &record.password_hash).await {
            return Ok(None);
        }

        if record.user.mfa_enabled {
            let Some(secret) = record.mfa_secret.as_deref() else {
                tracing::warn!(user.id = %record.user.id, "MFA is enabled without a TOTP secret; refusing the login");
                return Ok(None);
            };
            let Some(code) = creds.totp_code.as_deref().map(str::trim) else {
                return Ok(None);
            };
            if code.is_empty() {
                return Ok(None);
            }
            match crate::totp::verify(secret, code, 6, 30, 1, totp_rs::Algorithm::SHA1) {
                Ok(Some(step)) => {
                    // Persisted, so the step is spent for every replica, not
                    // just this process: the atomic conditional `UPDATE` is
                    // what makes a code single-use cluster-wide.
                    if !self.store.claim_totp_step(&record.user.id, step).await? {
                        tracing::warn!(user.id = %record.user.id, "replayed TOTP code refused");
                        return Ok(None);
                    }
                }
                Ok(None) => return Ok(None),
                Err(error) => {
                    tracing::warn!(%error, user.id = %record.user.id, "stored TOTP secret is unusable; refusing the login");
                    return Ok(None);
                }
            }
        }

        let record = self.rehash_if_stale(record, &creds.password).await?;
        Ok(Some(SessionUser::new(
            record.user.to_user(),
            record.session_auth_hash(),
        )))
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        let Some(record) = self.store.login_record_by_id(user_id).await? else {
            return Ok(None);
        };
        Ok(Some(SessionUser::new(
            record.user.to_user(),
            record.session_auth_hash(),
        )))
    }
}

/// Verify `password` against `hash` on the blocking pool.
///
/// Argon2 is CPU- and memory-bound; doing it on an async worker stalls the
/// whole runtime for the duration of the hash. A verification failure (a
/// malformed stored hash) and a task failure both mean "not authenticated".
#[allow(clippy::result_large_err)] // the closure returns `ferris_core::Error`, the crate-wide type.
async fn verify_password(password: &str, hash: &str) -> bool {
    let password = password.to_string();
    let hash = hash.to_string();
    match tokio::task::spawn_blocking(move || crate::verify_password(&password, &hash)).await {
        Ok(Ok(matched)) => matched,
        Ok(Err(error)) => {
            tracing::warn!(%error, "password verification failed");
            false
        }
        Err(error) => {
            tracing::warn!(%error, "password verification task failed");
            false
        }
    }
}
