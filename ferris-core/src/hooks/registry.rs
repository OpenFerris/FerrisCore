use super::point::{DocStatus, HookPoint};

use crate::error::Result;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub doctype: String,
    pub docname: String,
    pub user: String,
    pub request_id: Option<String>,
    pub metadata: HashMap<String, Value>,
}

/// `HookContext.metadata` key that asks the plugin-event forwarder — the hook
/// that turns a document lifecycle point into a guest `doctype-event` — to stay
/// silent for one write. It scopes suppression to the *plugin* events only:
/// unrelated global hooks (audit, indexing) still run. Set by
/// `docstore::write-no-events`; see [`HookContext::suppress_events`].
pub const SUPPRESS_EVENTS_KEY: &str = "ferris.suppress-events";

impl HookContext {
    pub fn new(doctype: String, docname: String, user: String) -> Self {
        Self {
            doctype,
            docname,
            user,
            request_id: None,
            metadata: HashMap::new(),
        }
    }

    /// Whether the writer asked the plugin-event forwarder to stay silent.
    pub fn suppress_events(&self) -> bool {
        matches!(
            self.metadata.get(SUPPRESS_EVENTS_KEY),
            Some(Value::Bool(true))
        )
    }

    /// Mark this context so the plugin-event forwarder is skipped for the write.
    pub fn set_suppress_events(&mut self) {
        self.metadata
            .insert(SUPPRESS_EVENTS_KEY.to_string(), Value::Bool(true));
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    pub fn with_metadata(mut self, key: String, value: Value) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

pub trait Hook: Send + Sync {
    fn name(&self) -> &'static str;
    fn priority(&self) -> i32 {
        0
    }
    async fn execute(&self, ctx: &mut HookContext, doc: &mut dyn DocumentHook) -> Result<()>;
}

pub trait DocumentHook: Send + Sync {
    fn doctype(&self) -> &str;
    fn name(&self) -> &str;
    fn get_field(&self, field: &str) -> Option<&Value>;
    fn set_field(&mut self, field: String, value: Value);
    fn get_fields(&self) -> &HashMap<String, Value>;
    fn is_new(&self) -> bool;
    fn docstatus(&self) -> DocStatus;
    fn set_docstatus(&mut self, status: DocStatus);
    fn owner(&self) -> &str;
    fn set_owner(&mut self, owner: String);
    fn modified_by(&self) -> &str;
    fn set_modified_by(&mut self, user: String);
}

pub type HookFn = Box<
    dyn Fn(
            &mut HookContext,
            &mut dyn DocumentHook,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

pub struct HookRegistry {
    hooks: HashMap<String, Vec<HookEntry>>,
}

type HookCondition = dyn Fn(&HookContext, &dyn DocumentHook) -> bool + Send + Sync;

struct HookEntry {
    point: HookPoint,
    /// `Arc` so matching entries can be cloned out and awaited *without* holding
    /// the registry lock across an `await`.
    function: Arc<HookFn>,
    priority: i32,
    condition: Option<Arc<HookCondition>>,
    /// Owner of this entry. Re-registering the same `(tag, doctype, point)`
    /// replaces the previous entry instead of stacking another copy of it, which
    /// is what the dev server needs when it re-wires the plugin on hot reload.
    /// `None` means "caller-managed", the historical append-only behaviour.
    tag: Option<Arc<str>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        doctype: String,
        point: HookPoint,
        function: HookFn,
        priority: i32,
        condition: Option<Box<HookCondition>>,
    ) {
        self.insert(None, doctype, point, function, priority, condition);
    }

    /// Register a hook owned by `tag`, dropping any entry the same `tag` already
    /// holds for this `doctype`/`point`.
    pub fn register_tagged(
        &mut self,
        tag: &str,
        doctype: String,
        point: HookPoint,
        function: HookFn,
        priority: i32,
        condition: Option<Box<HookCondition>>,
    ) {
        self.insert(
            Some(tag.into()),
            doctype,
            point,
            function,
            priority,
            condition,
        );
    }

    fn insert(
        &mut self,
        tag: Option<Arc<str>>,
        doctype: String,
        point: HookPoint,
        function: HookFn,
        priority: i32,
        condition: Option<Box<HookCondition>>,
    ) {
        if let Some(tag) = &tag {
            if let Some(entries) = self.hooks.get_mut(&doctype) {
                entries.retain(|e| !(e.point == point && e.tag.as_deref() == Some(tag.as_ref())));
            }
        }
        let entry = HookEntry {
            point,
            function: Arc::new(function),
            priority,
            condition: condition.map(Arc::from),
            tag,
        };
        self.hooks.entry(doctype).or_default().push(entry);
    }

    /// Clone the functions matching `doctype`/`point` (conditions evaluated),
    /// orderered by descending priority.
    fn matching(
        &self,
        doctype: &str,
        point: HookPoint,
        ctx: &HookContext,
        doc: &dyn DocumentHook,
    ) -> Vec<Arc<HookFn>> {
        let Some(hooks) = self.hooks.get(doctype) else {
            return Vec::new();
        };
        let mut filtered: Vec<&HookEntry> = hooks
            .iter()
            .filter(|h| h.point == point)
            .filter(|h| h.condition.as_ref().map_or(true, |c| c(ctx, doc)))
            .collect();
        filtered.sort_by_key(|h| std::cmp::Reverse(h.priority));
        filtered.into_iter().map(|h| h.function.clone()).collect()
    }

    pub async fn execute(
        &self,
        doctype: &str,
        point: HookPoint,
        ctx: &mut HookContext,
        doc: &mut dyn DocumentHook,
    ) -> Result<()> {
        let functions = self.matching(doctype, point, ctx, doc);
        for function in functions {
            (function)(ctx, doc).await?;
        }
        Ok(())
    }

    /// Execute the globally-registered hooks for `doctype`/`point`.
    ///
    /// The registry lock is released before any hook future is awaited, so the
    /// returned future is `Send`.
    pub async fn execute_global(
        doctype: &str,
        point: HookPoint,
        ctx: &mut HookContext,
        doc: &mut dyn DocumentHook,
    ) -> Result<()> {
        let functions = {
            let registry = Self::global().lock().unwrap_or_else(|e| e.into_inner());
            registry.matching(doctype, point, ctx, doc)
        };
        for function in functions {
            (function)(ctx, doc).await?;
        }
        Ok(())
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRegistry {
    /// Register a hook in the process-wide registry.
    ///
    /// Prefer [`HookRegistry::replace_global`] (tagged) when the caller may run
    /// again — a hot reload would otherwise stack a second copy of the same
    /// hook on every pass.
    pub fn register_global(
        doctype: String,
        point: HookPoint,
        function: HookFn,
        priority: i32,
        condition: Option<Box<dyn Fn(&HookContext, &dyn DocumentHook) -> bool + Send + Sync>>,
    ) {
        Self::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register(doctype, point, function, priority, condition);
    }

    /// Replace the hook `tag` holds for this `doctype`/`point`. Used by the dev
    /// server, which re-registers the plugin's hooks on every hot reload.
    pub fn replace_global(
        tag: &str,
        doctype: String,
        point: HookPoint,
        function: HookFn,
        priority: i32,
        condition: Option<Box<HookCondition>>,
    ) {
        Self::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register_tagged(tag, doctype, point, function, priority, condition);
    }

    /// The single process-wide registry.
    pub fn global() -> &'static Mutex<HookRegistry> {
        static REGISTRY: LazyLock<Mutex<HookRegistry>> =
            LazyLock::new(|| Mutex::new(HookRegistry::new()));
        &REGISTRY
    }
}
