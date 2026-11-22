//! Native module contract.
//!
//! A **module** is a framework feature that compiles into the host and
//! contributes HTTP routes and, optionally, documentation. It is the native
//! counterpart of a WASM app: the app declares its surface in `describe()` and
//! runs sandboxed, while a native module implements the Rust trait and shares
//! the host's process, database and authorization.
//!
//! The dependency direction is the point of this module:
//!
//! * `ferris-api` defines the contract and never imports a vertical crate.
//! * A vertical (`ferris-print`, `ferris-report`, …) implements [`Module`] and
//!   depends on `ferris-api`, never the other way around.
//! * `ferris-server` is the composition root: it builds the module list, mounts
//!   the routers and applies the shared auth/CSRF layers **once**.
//!
//! A module does not mount its own security middleware. It declares routes and
//! the host wraps them with the same policy as the generic API; otherwise a
//! feature could silently bypass authentication.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::extract::FromRef;
use ferris_auth::AuthState;
use ferris_core::ports::ServiceHandles;
use ferris_core::workspaces::WorkspaceRegistration;
use ferris_core::{Config, FeatureFlags, Result};
use ferris_db::Db;
use serde_json::Value;

use crate::ApiState;
use crate::method::MethodRegistry;

/// The state a module handler resolves.
///
/// It is deliberately smaller than the server's `AppState`: a feature sees the
/// API services and the auth state, not the Leptos, plugin or file services it
/// has no business touching. Handlers keep extracting `State<Arc<ApiState>>`
/// (and `AuthenticatedUser`) because both are reachable through [`FromRef`].
#[derive(Clone)]
pub struct ModuleState {
    pub api: Arc<ApiState>,
    pub auth: AuthState,
}

impl ModuleState {
    /// Build the module state from the host state.
    ///
    /// The host is the only caller: it knows its own `S` and provides both
    /// sub-states through [`FromRef`].
    #[must_use]
    pub fn from_state<S>(state: &S) -> Self
    where
        Arc<ApiState>: FromRef<S>,
        AuthState: FromRef<S>,
    {
        Self {
            api: Arc::<ApiState>::from_ref(state),
            auth: AuthState::from_ref(state),
        }
    }
}

impl FromRef<ModuleState> for Arc<ApiState> {
    fn from_ref(state: &ModuleState) -> Self {
        state.api.clone()
    }
}

impl FromRef<ModuleState> for AuthState {
    fn from_ref(state: &ModuleState) -> Self {
        state.auth.clone()
    }
}

/// The server roles a module may need at boot.
///
/// This is a projection of the host's role set, not a second role model: the
/// server maps its own `ServerRole` values onto these booleans once.
// A role projection is a set of independent switches, not an enum: a process
// can be worker and scheduler at the same time.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServiceRoles {
    pub api: bool,
    pub web: bool,
    pub realtime: bool,
    pub worker: bool,
    pub scheduler: bool,
}

/// What a module receives at boot and shutdown.
///
/// It carries the shared services a native feature needs (database, auth,
/// config and the compiled/runtime switches) and nothing host-specific such as
/// Leptos or the plugin store. A module that needs a handle produced by another
/// module does not reach into this context: the composition root wires the two
/// explicitly, which keeps the dependency direction visible.
pub struct ServiceContext {
    pub db: Db,
    pub auth: AuthState,
    pub config: Arc<Config>,
    pub features: FeatureFlags,
    pub roles: ServiceRoles,
    /// Queue/mailer handles the host installed for its modules. Empty on a
    /// build with no background services or before they are booted.
    pub handles: ServiceHandles,
}

/// Ordering hint for the composition root.
///
/// Core modules run first (they register the vocabulary every vertical relies
/// on); verticals run in their natural order afterwards. The order is stable,
/// so the router and the `OpenAPI` document are deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModuleOrder {
    Core,
    Vertical,
}

/// A fragment of the `OpenAPI` `paths` object contributed by a module.
///
/// Each entry is a full path (for example `/api/print/{doctype}/{name}`) and
/// the operation object for that path (`get`, `post`, …). The host merges the
/// fragments into `/api/openapi.json`, so a module route cannot exist without
/// being documented.
#[derive(Clone, Debug, Default)]
pub struct OpenApiFragment {
    pub paths: Vec<(String, Value)>,
}

/// A native framework feature.
///
/// The trait is intentionally small: routes and `OpenAPI` now, with room to add
/// `DocType`/workspace/UI declarations as the catalog is generalized. A module
/// that only contributes metadata returns an empty router.
#[async_trait]
pub trait Module: Send + Sync {
    /// Stable, kebab-case name used in logs and diagnostics.
    fn name(&self) -> &'static str;

    /// Where the composition root places this module's routes.
    fn order(&self) -> ModuleOrder {
        ModuleOrder::Vertical
    }

    /// Register the module's `DocTypes` in the process-wide schema registry.
    ///
    /// The core crates keep their own hardcoded order in `ferris-server`; this
    /// hook is for the optional verticals, so their declarations leave the
    /// server's catalog.
    fn register_doctypes(&self) {}

    /// Register the module's Desk workspaces and return the validation result.
    fn register_workspaces(&self) -> WorkspaceRegistration {
        WorkspaceRegistration::default()
    }

    /// Register API methods (framework bases an app may override).
    fn register_methods(&self, _methods: &MethodRegistry) {}

    /// Path prefix the host nests [`Self::routes`] under. `/api` by default.
    fn mount(&self) -> &'static str {
        "/api"
    }

    /// Routes **relative to [`Self::mount`]**, already typed against
    /// [`ModuleState`]. The host applies auth and CSRF on the outside.
    fn routes(&self) -> Router<ModuleState> {
        Router::new()
    }

    /// Prefix for routes that must stay **outside** the authenticated API, such
    /// as a signed public download. The server mounts them before the auth
    /// layers; the handler owns its own authorization.
    fn public_mount(&self) -> &'static str {
        "/"
    }

    /// Routes **relative to [`Self::public_mount`]**.
    fn public_routes(&self) -> Router<ModuleState> {
        Router::new()
    }

    /// `OpenAPI` paths contributed by this module.
    fn openapi(&self) -> OpenApiFragment {
        OpenApiFragment::default()
    }

    /// Start the module's background work. Runs once, after the core boot and
    /// before the router is built.
    ///
    /// # Errors
    ///
    /// A failure is fatal: a role the operator asked for must start or the
    /// server must not serve a process that claims to have it.
    async fn boot(&self, _ctx: &ServiceContext) -> Result<()> {
        Ok(())
    }

    /// Stop the module's background work. Runs in reverse registration order
    /// after the listener stops; implementations log their own failures.
    async fn shutdown(&self, _ctx: &ServiceContext) {}
}

/// The ordered module list the server composes.
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn Module>>,
}

impl ModuleRegistry {
    /// Build a registry, sorting by [`Module::order`] so registration and
    /// generated documentation are deterministic.
    #[must_use]
    pub fn new(mut modules: Vec<Arc<dyn Module>>) -> Self {
        modules.sort_by_key(|module| module.order());
        Self { modules }
    }

    #[must_use]
    pub fn modules(&self) -> &[Arc<dyn Module>] {
        &self.modules
    }

    /// Every module's `OpenAPI` fragment, in registry order.
    #[must_use]
    pub fn openapi_fragments(&self) -> Vec<OpenApiFragment> {
        self.modules.iter().map(|module| module.openapi()).collect()
    }

    /// Boot every module in order, stopping at the first failure.
    ///
    /// # Errors
    ///
    /// The first module that cannot start aborts the boot; the caller reports
    /// which one failed and does not serve.
    // `ferris_core::Error` is the taxonomy every caller matches; boxing it at
    // this one boundary would only make the failure harder to handle.
    #[allow(clippy::result_large_err)]
    pub async fn boot(&self, ctx: &ServiceContext) -> Result<()> {
        for module in &self.modules {
            module
                .boot(ctx)
                .await
                .map_err(|error| error_with_module(module.name(), error))?;
        }
        Ok(())
    }

    /// Shut every module down in reverse order.
    pub async fn shutdown(&self, ctx: &ServiceContext) {
        for module in self.modules.iter().rev() {
            module.shutdown(ctx).await;
        }
    }
}

/// Keep the failing module in the message without inventing a new error class:
/// a boot failure already carries a typed `ferris_core::Error`.
fn error_with_module(module: &str, error: ferris_core::Error) -> ferris_core::Error {
    tracing::error!(module, error = %error, "native module failed to boot");
    error
}
