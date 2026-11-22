//! Registry of callable API methods (`/api/method/<name>`).
//!
//! The registry is an **ordered chain** per name, not a single slot: a
//! framework base can be overridden by an app, and an app can override another
//! app. Resolution is last-wins (the chain is built in app-name order, so it is
//! deterministic), and an override can reach the entry below itself with
//! [`MethodRegistry::call_next`] — the explicit `super` a guest reaches through
//! `data::call-method`.
//!
//! A base is registered by the framework, never replaced (`register_base`
//! refuses a duplicate). An app declaration is an override *entry*: it may be
//! the only entry (a method that exists only because the app declares it) or it
//! may shadow a base, which requires the `overrides` capability at load time
//! (enforced in `ferris-wasm`), not here.
//!
//! There is no automatic fallback on error: an override that fails is an error.
//! Falling through to the base would turn a broken app into a silently
//! different API.

use async_trait::async_trait;
use ferris_core::{DocumentError, Error, Result, User, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Who may invoke a method.
///
/// The default is **any authenticated user**: a base registered without an
/// explicit policy is a framework API, and the ambient caller is what the
/// document engine authorises the operation against. A method that is safe for
/// anonymous callers has to say so with [`MethodPolicy::guest`], so a future
/// base cannot become public merely by omission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MethodPolicy {
    /// Whether the method may run without an ambient user at all.
    pub allow_guest: bool,
    /// Roles allowed to call it. Empty means "any authenticated user"; when
    /// non-empty the caller must hold at least one of them.
    pub roles: Vec<String>,
}

impl MethodPolicy {
    /// A method callable by anonymous callers (a public RPC).
    #[must_use]
    pub fn guest() -> Self {
        Self {
            allow_guest: true,
            roles: Vec::new(),
        }
    }

    /// A method restricted to authenticated users holding one of `roles`.
    #[must_use]
    pub fn with_roles(roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allow_guest: false,
            roles: roles.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether `user` (the ambient caller, `None` when anonymous) may call.
    ///
    /// Anonymous callers pass only under `allow_guest`. An authenticated
    /// caller passes when the method is a guest method or declares no roles,
    /// and otherwise must hold at least one declared role.
    #[must_use]
    pub fn permits(&self, user: Option<&User>) -> bool {
        match user {
            None => self.allow_guest,
            Some(user) => {
                self.allow_guest
                    || self.roles.is_empty()
                    || self.roles.iter().any(|role| user.has_role(role))
            }
        }
    }
}

/// A callable API method.
#[async_trait]
pub trait MethodHandler: Send + Sync {
    async fn call(&self, params: Value) -> Result<Value>;

    /// Which app supplied this handler; `None` for a framework base.
    ///
    /// The chain uses it for two things: replacing an app's own entry on hot
    /// reload (one entry per owner) and resolving `call_next` for that owner.
    fn owner(&self) -> Option<&str> {
        None
    }
}

/// One name's chain: a base plus the app entries that shadow it.
#[derive(Clone, Default)]
struct MethodChain {
    base: Option<Arc<dyn MethodHandler>>,
    /// Registration order (apps sorted by name at boot).
    overrides: Vec<Arc<dyn MethodHandler>>,
    /// Who may invoke the name. Owned by the chain, not by one entry, so an
    /// app overriding a framework base inherits that base's policy instead of
    /// silently widening it to "any app entry".
    policy: MethodPolicy,
}

impl MethodChain {
    /// The handler a plain call reaches: the last override, else the base.
    fn last(&self) -> Option<Arc<dyn MethodHandler>> {
        self.overrides.last().cloned().or_else(|| self.base.clone())
    }
}

/// Thread-safe registry of API methods.
#[derive(Clone, Default)]
pub struct MethodRegistry {
    methods: Arc<RwLock<HashMap<String, MethodChain>>>,
}

impl MethodRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a framework base implementation with the default policy: any
    /// authenticated caller, never anonymous.
    ///
    /// # Errors
    ///
    /// Returns `AlreadyExists` when the name already has a base: two framework
    /// owners (or a reload that re-registers the same base) must not silently
    /// fight over it.
    pub fn register_base(
        &self,
        name: impl Into<String>,
        handler: Arc<dyn MethodHandler>,
    ) -> Result<()> {
        self.register_base_with_policy(name, handler, MethodPolicy::default())
    }

    /// Register a framework base implementation and the policy that guards it.
    ///
    /// # Errors
    ///
    /// Returns `AlreadyExists` when the name already has a base; see
    /// [`Self::register_base`].
    pub fn register_base_with_policy(
        &self,
        name: impl Into<String>,
        handler: Arc<dyn MethodHandler>,
        policy: MethodPolicy,
    ) -> Result<()> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(Error::Validation(ferris_core::ValidationError::Required(
                "method name".to_string(),
            )));
        }
        let mut methods = self.methods.write().unwrap_or_else(|e| e.into_inner());
        let chain = methods.entry(name.clone()).or_default();
        if chain.base.is_some() {
            return Err(Error::Document(DocumentError::AlreadyExists {
                doctype: "method".to_string(),
                name,
            }));
        }
        chain.base = Some(handler);
        chain.policy = policy;
        Ok(())
    }

    /// Declare a method, or add an app's override entry for an existing one.
    ///
    /// The handler must report its owner ([`MethodHandler::owner`]); an entry
    /// from the same owner replaces that owner's previous one, which is what a
    /// hot reload needs (one entry per app, not one per reload).
    pub fn declare(&self, name: impl Into<String>, handler: Arc<dyn MethodHandler>) {
        let name = name.into();
        let owner = handler.owner().map(str::to_string);
        let mut methods = self.methods.write().unwrap_or_else(|e| e.into_inner());
        let chain = methods.entry(name).or_default();
        if let Some(owner) = owner {
            chain
                .overrides
                .retain(|existing| existing.owner() != Some(owner.as_str()));
        }
        chain.overrides.push(handler);
    }

    /// Registered method names, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .methods
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    pub fn contains(&self, name: &str) -> bool {
        self.methods
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .is_some_and(|chain| chain.last().is_some())
    }

    /// The owners of `name`, base first (`"base"` for the framework entry).
    ///
    /// Boot logs one line per chain, so an operator can see which app shadows a
    /// framework API without reading the config.
    pub fn owners(&self, name: &str) -> Vec<String> {
        let methods = self.methods.read().unwrap_or_else(|e| e.into_inner());
        let Some(chain) = methods.get(name) else {
            return Vec::new();
        };
        let mut owners = Vec::with_capacity(chain.overrides.len() + 1);
        if chain.base.is_some() {
            owners.push("base".to_string());
        }
        owners.extend(
            chain
                .overrides
                .iter()
                .map(|handler| handler.owner().unwrap_or("base").to_string()),
        );
        owners
    }

    /// Invoke a method as the ambient caller.
    ///
    /// The policy is checked **before** the handler runs: an unauthenticated
    /// caller (a device authenticated only by mTLS, a job with no scoped user)
    /// is refused unless the method declared [`MethodPolicy::guest`], and a
    /// role-gated method needs the caller to hold one of those roles. This is
    /// the single authorization decision for `/api/method`, JSON-RPC, batch and
    /// the file adapter.
    ///
    /// # Errors
    ///
    /// `NotFound` when no entry exists, `PermissionDenied` when the ambient
    /// caller fails the policy.
    pub async fn call(&self, name: &str, params: Value) -> Result<Value> {
        // Clone the handler and policy out and drop the lock before awaiting.
        let (handler, policy) = {
            let methods = self.methods.read().unwrap_or_else(|e| e.into_inner());
            let chain = methods.get(name).ok_or_else(|| {
                Error::Document(DocumentError::NotFound {
                    doctype: "method".to_string(),
                    name: name.to_string(),
                })
            })?;
            let handler = chain.last().ok_or_else(|| {
                Error::Document(DocumentError::NotFound {
                    doctype: "method".to_string(),
                    name: name.to_string(),
                })
            })?;
            (handler, chain.policy.clone())
        };
        let caller = ferris_document::current_user();
        if !policy.permits(caller.as_ref()) {
            tracing::warn!(
                method = name,
                authenticated = caller.is_some(),
                "method call refused by policy"
            );
            return Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
                action: format!("call method `{name}`"),
                resource: "method".to_string(),
            }));
        }
        handler.call(params).await
    }

    /// Run the entry **below** `caller`'s own in the chain of `name`.
    ///
    /// This is the guest-visible `super`: `caller` must have an entry on the
    /// chain (it is overriding the method), otherwise the call is refused with
    /// `PermissionDenied`. It never re-enters `call`, so a chain of N overrides
    /// costs one step, not N, and a guest cannot recurse into itself.
    ///
    /// # Errors
    ///
    /// `NotFound` when the method does not exist, `PermissionDenied` when
    /// `caller` does not override it.
    pub async fn call_next(&self, name: &str, caller: &str, params: Value) -> Result<Value> {
        let handler = {
            let methods = self.methods.read().unwrap_or_else(|e| e.into_inner());
            let chain = methods.get(name).ok_or_else(|| {
                Error::Document(DocumentError::NotFound {
                    doctype: "method".to_string(),
                    name: name.to_string(),
                })
            })?;
            let position = chain
                .overrides
                .iter()
                .rposition(|handler| handler.owner() == Some(caller))
                .ok_or_else(|| {
                    Error::Authz(ferris_core::AuthzError::PermissionDenied {
                        action: "call-next".to_string(),
                        resource: format!("method `{name}` (app `{caller}` does not override it)"),
                    })
                })?;
            if position > 0 {
                chain.overrides[position - 1].clone()
            } else {
                chain.base.clone().ok_or_else(|| {
                    Error::Document(DocumentError::NotFound {
                        doctype: "method".to_string(),
                        name: name.to_string(),
                    })
                })?
            }
        };
        handler.call(params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(value: &'static str) -> Option<&'static str> {
        Some(value)
    }

    struct Echo {
        owner: Option<&'static str>,
        tag: &'static str,
    }

    #[async_trait]
    impl MethodHandler for Echo {
        async fn call(&self, params: Value) -> Result<Value> {
            Ok(Value::Object(std::collections::HashMap::from([
                ("tag".to_string(), Value::String(self.tag.to_string())),
                ("params".to_string(), params),
            ])))
        }

        fn owner(&self) -> Option<&str> {
            self.owner
        }
    }

    /// The `tag` field of a handler's answer, independent of the enum shape.
    fn tag(value: &Value) -> Option<&str> {
        value.get("tag").and_then(Value::as_str)
    }

    fn handler(owner_name: Option<&'static str>, tag: &'static str) -> Arc<dyn MethodHandler> {
        Arc::new(Echo {
            owner: owner_name,
            tag,
        })
    }

    fn admin() -> User {
        let mut user = User::system_user("admin@local".to_string(), "Admin".to_string());
        user.roles = vec![ferris_core::permissions::SYSTEM_MANAGER_ROLE.to_string()];
        user
    }

    /// Run `fut` with an ambient caller: `call` is an authenticated entry
    /// point, so a bare test future is anonymous by design.
    async fn as_admin<T, F>(fut: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        ferris_document::with_user(Some(admin()), fut).await
    }

    #[tokio::test]
    async fn a_plain_method_round_trips() {
        let registry = MethodRegistry::new();
        registry.declare("echo", handler(None, "app"));
        let out = as_admin(registry.call("echo", Value::from("hi")))
            .await
            .unwrap();
        assert_eq!(tag(&out), Some("app"));
        assert!(
            as_admin(registry.call("missing", Value::Null))
                .await
                .is_err()
        );
    }

    /// A base is overridden by the app, and `call-next` reaches the base.
    #[tokio::test]
    async fn an_override_wins_and_can_reach_the_base() {
        let registry = MethodRegistry::new();
        registry
            .register_base("files.list", handler(None, "base"))
            .unwrap();
        registry.declare("files.list", handler(owner("drive"), "drive"));

        let out = as_admin(registry.call("files.list", Value::Null))
            .await
            .unwrap();
        assert_eq!(
            tag(&out),
            Some("drive"),
            "the override is what a caller gets"
        );

        let inner = registry
            .call_next("files.list", "drive", Value::Null)
            .await
            .unwrap();
        assert_eq!(tag(&inner), Some("base"));
    }

    /// Two apps override the same name: the name-sorted second one wins and
    /// can reach the first, which can reach the base.
    #[tokio::test]
    async fn an_override_chain_steps_one_entry_at_a_time() {
        let registry = MethodRegistry::new();
        registry.register_base("m", handler(None, "base")).unwrap();
        registry.declare("m", handler(owner("app-a"), "a"));
        registry.declare("m", handler(owner("app-b"), "b"));

        let out = as_admin(registry.call("m", Value::Null)).await.unwrap();
        assert_eq!(tag(&out), Some("b"));
        assert_eq!(
            tag(&registry.call_next("m", "app-b", Value::Null).await.unwrap()),
            Some("a")
        );
        assert_eq!(
            tag(&registry.call_next("m", "app-a", Value::Null).await.unwrap()),
            Some("base")
        );
    }

    /// A guest that does not override the method cannot use `call-method` to
    /// reach it, and re-entering its own entry is impossible.
    #[tokio::test]
    async fn call_next_is_scoped_to_an_owner() {
        let registry = MethodRegistry::new();
        registry.register_base("m", handler(None, "base")).unwrap();
        registry.declare("m", handler(owner("app"), "app"));

        assert!(registry.call_next("m", "other", Value::Null).await.is_err());
        assert_eq!(
            tag(&registry.call_next("m", "app", Value::Null).await.unwrap()),
            Some("base")
        );
    }

    /// A base cannot be registered twice; a reload of the same app replaces its
    /// own entry instead of growing the chain.
    #[tokio::test]
    async fn bases_are_unique_and_reloads_replace_the_owner_entry() {
        let registry = MethodRegistry::new();
        registry.register_base("m", handler(None, "base")).unwrap();
        assert!(registry.register_base("m", handler(None, "base2")).is_err());

        registry.declare("m", handler(owner("app"), "v1"));
        registry.declare("m", handler(owner("app"), "v2"));
        assert_eq!(registry.owners("m"), ["base", "app"]);
        let out = as_admin(registry.call("m", Value::Null)).await.unwrap();
        assert_eq!(tag(&out), Some("v2"));
        assert_eq!(
            tag(&registry.call_next("m", "app", Value::Null).await.unwrap()),
            Some("base")
        );
    }

    /// The default policy is "authenticated": a device or job with no ambient
    /// user cannot reach a base, and a plain test registry is anonymous.
    #[tokio::test]
    async fn a_method_without_an_ambient_user_is_refused() {
        let registry = MethodRegistry::new();
        registry.register_base("m", handler(None, "base")).unwrap();

        let err = registry.call("m", Value::Null).await.unwrap_err();
        assert!(matches!(
            err,
            Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
        ));

        let out = as_admin(registry.call("m", Value::Null)).await.unwrap();
        assert_eq!(tag(&out), Some("base"));
    }

    /// `allow_guest` is the only way an anonymous caller gets in.
    #[tokio::test]
    async fn a_guest_policy_allows_an_anonymous_call() {
        let registry = MethodRegistry::new();
        registry
            .register_base_with_policy(
                "public.echo",
                handler(None, "public"),
                MethodPolicy::guest(),
            )
            .unwrap();
        let out = registry.call("public.echo", Value::Null).await.unwrap();
        assert_eq!(tag(&out), Some("public"));
    }

    /// A role-gated method needs the intersection with the caller's roles.
    #[tokio::test]
    async fn a_role_policy_requires_one_matching_role() {
        let registry = MethodRegistry::new();
        registry
            .register_base_with_policy(
                "m",
                handler(None, "base"),
                MethodPolicy::with_roles(["System Manager"]),
            )
            .unwrap();

        // Anonymous: refused.
        assert!(registry.call("m", Value::Null).await.is_err());

        // Authenticated but without the role: refused.
        let mut sales = User::system_user("sales@local".to_string(), "Sales".to_string());
        sales.roles = vec!["Sales".to_string()];
        let err = ferris_document::with_user(Some(sales), registry.call("m", Value::Null))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
        ));

        // The declared role passes.
        assert!(as_admin(registry.call("m", Value::Null)).await.is_ok());
    }
}
