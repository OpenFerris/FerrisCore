//! Lifecycle dispatch for the engine.
//!
//! The engine has one write path for both the typed `#[doctype]` CRUD and the
//! dynamic REST/Desk path, so the controller order lives here instead of in each
//! caller. That is the same reason Frappe keeps it in `Document.run_method`:
//! two copies of the order would disagree the first time a point is added.
//!
//! The order, once, for every write:
//!
//! * **Insert**: [`HookPoint::BeforeNaming`] → [`HookPoint::BeforeValidate`] →
//!   [`HookPoint::Validate`] → [`HookPoint::BeforeInsert`] → row write →
//!   [`HookPoint::AfterInsert`] → [`HookPoint::OnUpdate`].
//! * **Update**: [`HookPoint::BeforeValidate`] → [`HookPoint::Validate`] →
//!   [`HookPoint::BeforeSave`] →
//!   [`HookPoint::BeforeUpdateAfterSubmit`] (only for a submitted row whose
//!   non-engine fields changed) → row write → [`HookPoint::AfterSave`] →
//!   [`HookPoint::OnUpdate`] →
//!   [`HookPoint::OnUpdateAfterSubmit`] (same submitted/changed condition) →
//!   [`HookPoint::OnChange`] (any non-engine change).
//! * **Rename**: [`HookPoint::BeforeRename`] → rename → [`HookPoint::AfterRename`].
//!
//! [`run_hooks_with_metadata`] is the metadata-carrying form. Rename needs the
//! old and new names in `HookContext.metadata`; every existing call site keeps
//! using [`run_hooks`], whose semantics are unchanged.

use super::super::*;
use ferris_core::Result;
use ferris_core::Value;
use ferris_core::hooks::{HookContext, HookPoint, HookRegistry};

pub(in crate::engine) async fn run_hooks(
    ctx: &EngineContext<'_>,
    name: &str,
    document: &mut Value,
    is_new: bool,
    point: HookPoint,
) -> Result<()> {
    run_hooks_with_metadata(ctx, name, document, is_new, point, &[]).await
}

/// [`run_hooks`] with extra `HookContext` metadata.
///
/// The metadata is only visible to the hooks of *this* point; it is not written
/// to the document. `BeforeRename`/`AfterRename` use it for `old_name` and
/// `new_name` because the document itself can only carry one of them.
pub(in crate::engine) async fn run_hooks_with_metadata(
    ctx: &EngineContext<'_>,
    name: &str,
    document: &mut Value,
    is_new: bool,
    point: HookPoint,
    metadata: &[(&str, Value)],
) -> Result<()> {
    let fields = to_fields(document)?;
    let mut map_doc = MapDocument {
        doctype: ctx.doctype.clone(),
        name: name.to_string(),
        fields,
        is_new,
    };
    let mut hook_ctx = HookContext::new(
        ctx.doctype.clone(),
        name.to_string(),
        // An anonymous save has *no* acting identity, so the context carries an
        // empty one. It used to claim `"Administrator"`, which handed every
        // global hook a privileged author for a request nobody made — the same
        // default-privileged-value mistake C1 was about, in the one place a hook
        // could act on it.
        ctx.user.as_ref().map(|u| u.id.clone()).unwrap_or_default(),
    );
    for (key, value) in metadata {
        hook_ctx = hook_ctx.with_metadata((*key).to_string(), value.clone());
    }
    if ctx.suppress_events {
        // A `write-no-events` write: the plugin-event forwarder stays silent so
        // the guest never re-enters on its own row. Every other global hook
        // still runs.
        hook_ctx.set_suppress_events();
    }
    HookRegistry::execute_global(&ctx.doctype, point, &mut hook_ctx, &mut map_doc).await?;
    *document = fields_to_value(map_doc.fields);
    Ok(())
}
