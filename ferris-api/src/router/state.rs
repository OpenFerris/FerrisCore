use std::sync::Arc;

use ferris_auth::{MfaConfig, MfaManager};
use ferris_core::FeatureFlags;
use ferris_db::Db;

use crate::method::MethodRegistry;
use crate::module::OpenApiFragment;

/// Shared API state.
#[derive(Clone)]
pub struct ApiState {
    pub pool: Db,
    pub methods: MethodRegistry,
    /// The catalogue set metadata endpoints translate against. The composition
    /// root replaces the English-only default with the instance policy.
    pub i18n: ferris_core::i18n::I18n,
    /// The operator policy document path, reloaded after every Role Permissions
    /// Builder edit so those rows never silently drop the operator JSON.
    pub policy_path: Option<std::path::PathBuf>,
    /// TOTP/backup-code settings for the authenticated MFA self-service routes
    /// (`ferris-api::mfa`). Wired to the same `AuthConfig` the login gate's
    /// [`crate::session::SessionHandlers`] uses, so both halves of the flow
    /// agree on code length, period and issuer.
    pub mfa: MfaManager,
    /// Install-wide feature switches for the framework-owned routes that stay
    /// in this crate (for example `search`). A disabled feature is a **missing
    /// route** (404), not a handler that pretends to work. Vertical modules are
    /// registered by the composition root instead: if the module is not in the
    /// registry, its routes and fragments do not exist.
    pub features: FeatureFlags,
    /// `OpenAPI` paths contributed by the native modules mounted on this router.
    /// The composition root fills this before the router is built, so
    /// `/api/openapi.json` and the mounted routes share one source.
    pub openapi_fragments: Arc<[OpenApiFragment]>,
}

impl ApiState {
    pub fn new(pool: Db) -> Self {
        Self {
            pool,
            i18n: ferris_core::i18n::I18n::default(),
            methods: MethodRegistry::new(),
            policy_path: None,
            mfa: MfaManager::new(MfaConfig::default()),
            features: FeatureFlags::default(),
            openapi_fragments: Arc::from(Vec::<OpenApiFragment>::new()),
        }
    }

    /// [`Self::new`], plus the operator policy path the server boots with.
    pub fn with_policy_path(pool: Db, policy_path: Option<std::path::PathBuf>) -> Self {
        Self {
            policy_path,
            ..Self::new(pool)
        }
    }

    /// Carry the operator's `[features]` so the route table can be built from
    /// them. Defaults to "everything on", which is what a state built by a test
    /// or a bare caller means.
    #[must_use]
    pub fn with_features(mut self, features: FeatureFlags) -> Self {
        self.features = features;
        self
    }

    /// Carry the OpenAPI fragments of the modules mounted on this router.
    #[must_use]
    pub fn with_openapi_fragments(mut self, fragments: Vec<OpenApiFragment>) -> Self {
        self.openapi_fragments = Arc::from(fragments);
        self
    }

    /// Carry the instance's catalogue set, so `/api/*` projects metadata in the
    /// caller's language.
    #[must_use]
    pub fn with_i18n(mut self, i18n: ferris_core::i18n::I18n) -> Self {
        self.i18n = i18n;
        self
    }

    /// Point the MFA self-service routes at the operator's `AuthConfig`.
    #[must_use]
    pub fn with_auth_config(mut self, auth: &ferris_core::AuthConfig) -> Self {
        self.mfa = MfaManager::new(MfaConfig {
            issuer: auth.mfa_issuer.clone(),
            ..MfaConfig::default()
        });
        self
    }
}
