use super::super::support::*;
use super::super::*;
use super::workflow::{enforce_state_edit, resolve_workflow_update};
use ferris_core::hooks::HookPoint;
use ferris_core::permissions::Permission;
use ferris_core::{Result, Value};

impl DocumentEngine {
    /// Update a document by name.
    ///
    /// System fields (`owner`, `creation`, `docstatus`) are taken from the
    /// stored row and cannot be moved by the request body — lifecycle changes
    /// go through `set_docstatus` (which enforces `can_transition_to` and the
    /// matching permission). `modified` and `modified_by` are re-stamped from
    /// the session. Once a document is submitted only `allow_on_submit` fields
    /// may change, and a cancelled document is fully locked (§C5). `owner` is
    /// resolved **before** authorization so the decision is made against
    /// stored state.
    pub async fn update(&self, ctx: &EngineContext<'_>, name: &str, data: Value) -> Result<Value> {
        // A Virtual DocType owns its rows: the provider performs the update, the
        // engine only authorizes the action first.
        if is_virtual(ctx) {
            let provider = virtual_doctype_provider(&ctx.doctype).ok_or_else(|| {
                Error::Document(ferris_core::DocumentError::VirtualDoctypeProviderMissing {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            self.authorize(ctx, name, None, Permission::Write).await?;
            let document = provider.update(ctx, name, data)?;
            return finish_virtual_write(ctx, document);
        }
        let table = self.ensure_table(&ctx.doctype).await?;
        self.ensure_runtime_tables().await?;
        // Authorize against stored state *before* the transaction opens, after
        // rejecting the same system-field overrides `update_via` rejects, so the
        // established 400-before-403 order is preserved. The decision needs the
        // stored owner anyway, and a denied caller must not hold the write
        // connection while `authorize` consults `tab_share` on another one (a
        // one-connection pool would exhaust). `update_via` re-checks inside the
        // transaction, where its read is authoritative.
        let (existing, owner) = self.read_row(&ctx.doctype, name).await?;
        let stored = to_fields(&existing)?;
        let Some(incoming) = data.as_object() else {
            return Err(Error::Validation(ValidationError::Invalid(
                "document must be an object".to_string(),
            )));
        };
        // A `workflow_state` in the body is a transition request, so it is
        // validated here as well as inside the transaction: the 400-before-403
        // order holds, and a denied caller still learns nothing new.
        let workflow_move = resolve_workflow_update(ctx, &stored, incoming)?;
        reject_system_overrides(
            incoming,
            &stored,
            owner.as_deref(),
            ctx.user.as_ref().map(|user| user.id.as_str()),
            workflow_move
                .as_ref()
                .map(|moved| moved.outcome.state.as_str()),
        )?;
        self.authorize(
            ctx,
            name,
            authz_owner(ctx, owner.as_deref()),
            Permission::Write,
        )
        .await?;
        // The row write and its version row commit together (DOC-04/SEC-06):
        // the pool used to autocommit the document and then try the version, so
        // a version failure reported an error for a document that was already
        // stored. The transaction is opened after the lazily-created tables, so
        // no DDL runs on it.
        let mut tx = self.db.begin().await?;
        let outcome = {
            let mut sink = WriteSink::Tx(&mut tx);
            self.update_via_with(
                &mut sink,
                &table,
                ctx,
                name,
                data,
                Some(PreAuthorized {
                    owner: owner.clone(),
                }),
                false,
            )
            .await
        };
        match outcome {
            Ok(mut document) => {
                tx.commit().await?;
                // After-hooks run once the row is committed so a hook that
                // re-enters the engine (milestone, dashlink) can see it.
                let submitted = document
                    .get("docstatus")
                    .and_then(Value::as_str)
                    .map(docstatus_from_str)
                    .is_some_and(DocStatus::is_submitted);
                let changed = document_changed(&stored, &document);
                run_update_after_hooks(ctx, name, &mut document, submitted, changed).await?;
                Ok(document)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    /// [`Self::update`] through an explicit [`WriteSink`]: the pool for a
    /// single call, `apply`'s open transaction for a batch. `table` is already
    /// ensured by the caller.
    #[allow(clippy::unused_self)] // kept a method so `update`/`apply` share one path
    pub(super) async fn update_via(
        &self,
        sink: &mut WriteSink<'_>,
        table: &str,
        ctx: &EngineContext<'_>,
        name: &str,
        data: Value,
    ) -> Result<Value> {
        self.update_via_with(sink, table, ctx, name, data, None, true)
            .await
    }

    /// [`Self::update_via`] with an authorization decision already made.
    ///
    /// `pre_authorized` is the decision the caller made against a stored owner
    /// before opening the transaction. When the row read on the transaction's
    /// own connection still has that owner, the decision is reused: re-running
    /// `authorize` there would ask `tab_share` on a *second* connection while
    /// this transaction holds the first, which deadlocks a one-connection pool.
    /// An owner that moved in between forces a fresh check against the stored
    /// state, so the shortcut can never outlive the row it was made for.
    #[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
    async fn update_via_with(
        &self,
        sink: &mut WriteSink<'_>,
        table: &str,
        ctx: &EngineContext<'_>,
        name: &str,
        data: Value,
        pre_authorized: Option<PreAuthorized>,
        run_after: bool,
    ) -> Result<Value> {
        let (existing, stored_owner) = read_row_via(sink, &ctx.doctype, name).await?;

        // Creation time is engine-stamped and only ever written here, so the
        // blob is as authoritative as the column. `owner` deliberately comes
        // from the column (already returned above) rather than the blob.
        let stored = to_fields(&existing)?;
        let stored_creation = field_str(&stored, "creation");

        let mut merged = match existing {
            Value::Object(map) => map,
            _ => ferris_core::Map::new(),
        };

        let mut incoming_fields = match data {
            Value::Object(map) => map,
            _ => ferris_core::Map::new(),
        };

        // Reject any attempt to move a system field, *before* the merge, so the
        // authorization below can never observe a caller-supplied `owner`. The
        // same check runs in `update` before the transaction opens, so a denied
        // caller still gets this 400 rather than a 403 (C1 ordering).
        //
        // `workflow_state` is resolved first: it is the one engine field a body
        // may *request*, and only by naming a state the workflow transitions to.
        let workflow_move = resolve_workflow_update(ctx, &stored, &incoming_fields)?;
        // DOC-09: a state whose `allow_edit` does not name one of the caller's
        // roles locks the document's data; only the workflow move is accepted.
        enforce_state_edit(ctx, &stored, &incoming_fields)?;
        reject_system_overrides(
            &incoming_fields,
            &stored,
            stored_owner.as_deref(),
            ctx.user.as_ref().map(|user| user.id.as_str()),
            workflow_move
                .as_ref()
                .map(|moved| moved.outcome.state.as_str()),
        )?;
        // The engine writes the state it validated, never the raw body value:
        // `resolve_workflow_update` already refused anything undeclared.
        incoming_fields.remove("workflow_state");
        // Mass assignment (DOC-08): a key the metadata does not declare is not
        // a document field, so it is refused rather than merged into the blob.
        reject_undeclared_fields(ctx, &incoming_fields)?;

        // Field-level write permissions, applied before the merge: a field the
        // caller may not write is dropped (the stored value survives), a
        // `Password` mask means "unchanged" and is dropped, and a new password
        // is hashed.
        apply_field_permissions_write(ctx, &mut incoming_fields)?;
        // Virtual fields are computed on read; accepting a value would be
        // silently overwritten by the next read.
        reject_virtual_field_writes(ctx, &incoming_fields)?;

        for (key, value) in incoming_fields {
            merged.insert(key, value);
        }
        merged.insert("name".to_string(), Value::from(name));
        apply_workflow_state(&mut merged, workflow_move.as_ref());

        // Re-assert the stored system fields on the merged map. This is
        // belt-and-braces: the rejection above already failed the request, so
        // this only runs for requests that did not try to touch them.
        match &stored_owner {
            Some(owner) => {
                merged.insert("owner".to_string(), Value::from(owner.clone()));
            }
            None => {
                merged.remove("owner");
            }
        }
        match &stored_creation {
            Some(creation) => {
                merged.insert("creation".to_string(), Value::from(creation.clone()));
            }
            None => {
                merged.remove("creation");
            }
        }

        // §C5: post-submit lock. `docstatus` was already pinned above; this
        // stops a submitted row from being silently rewritten (only
        // `allow_on_submit` fields survive) and a cancelled row from being
        // reopened in place — amendment is the copy-and-create flow.
        guard_update_after_submit(ctx.meta, &stored, &merged)?;

        // Authorization before the lifecycle: a caller who may not write must
        // not run a `before_validate` hook (hooks have effects) nor learn
        // whether their payload would have validated (400) or not been allowed
        // (403). `stored_owner` is the owner read from the row, never the body.
        // For an allowed caller the order below is the Frappe one.
        self.authorize_update(ctx, name, stored_owner.as_deref(), pre_authorized.as_ref())
            .await?;

        // Frappe's `before_validate` runs before schema validation so a
        // controller can compute the fields validation then checks.
        let mut document = fields_to_value(merged);
        run_hooks(ctx, name, &mut document, false, HookPoint::BeforeValidate).await?;
        validate(ctx.meta, &document)?;

        // Stamped after the permission check so hooks observe the final state.
        // Frappe does this in `set_user_and_timestamp` for every save.
        let now = chrono::Utc::now().timestamp();
        document.set("modified".to_string(), Value::from(rfc3339(now)?));
        if let Some(user) = ctx.user.as_ref() {
            document.set("modified_by".to_string(), Value::from(user.id.clone()));
        }

        // The document was already `submitted` when this save started. That
        // decision comes from stored state, never from the (untrusted) merged
        // map, so a body cannot opt itself into or out of the after-submit
        // lifecycle.
        let submitted = stored
            .get("docstatus")
            .and_then(Value::as_str)
            .map(docstatus_from_str)
            .is_some_and(DocStatus::is_submitted);

        run_hooks(ctx, name, &mut document, false, HookPoint::Validate).await?;
        run_hooks(ctx, name, &mut document, false, HookPoint::BeforeSave).await?;

        // Measured after the before-save hooks: they are allowed to compute
        // fields, and only the values about to be persisted are a real change.
        // Engine-owned stamps (`modified`, `modified_by`, `name`, …) never make
        // a document "changed" — the same rule the post-submit lock uses.
        let changed = document_changed(&stored, &document);

        // Frappe fires `before_update_after_submit` before the write and
        // `on_update_after_submit` after it. Both only apply to a row that was
        // already submitted and whose non-engine fields actually moved; the
        // field lock above has already refused anything not `allow_on_submit`.
        if submitted && changed {
            run_hooks(
                ctx,
                name,
                &mut document,
                false,
                HookPoint::BeforeUpdateAfterSubmit,
            )
            .await?;
        }

        // `AfterSave`/`OnUpdate` run *after* the write (as in Frappe): a hook
        // (the `test` plugin's `doctype_event`) reads the document back and
        // compares it with what it was handed; if the hook ran before
        // `persist_written` it would read the previous state and the save would
        // be rejected ("stored data differs from the data I was given").
        persist_written(sink, table, name, &document).await?;
        // Record history on the same connection, before the after-hooks run so a
        // hook that reads back the version sees the write that just happened.
        self.record_version(
            sink,
            ctx,
            name,
            &document,
            crate::versioning::ChangeType::Update,
        )
        .await?;
        // DOC-09: listeners (`notify`, `auto_assign` consumers) hear about the
        // transition only once the row is written, so a rolled-back save cannot
        // announce a state change that never happened.
        emit_workflow_move(ctx, workflow_move.as_ref());
        if run_after {
            run_update_after_hooks(ctx, name, &mut document, submitted, changed).await?;
        }
        virtual_read(ctx, document)
    }

    /// Reuse a matching pre-transaction authorization, or authorize now.
    #[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
    async fn authorize_update(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        stored_owner: Option<&str>,
        pre_authorized: Option<&PreAuthorized>,
    ) -> Result<()> {
        if pre_authorized.is_some_and(|decision| decision.owner.as_deref() == stored_owner) {
            return Ok(());
        }
        self.authorize(ctx, name, authz_owner(ctx, stored_owner), Permission::Write)
            .await
    }
}

/// Run the after-save points against a committed row.
///
/// Shared by the transactional [`DocumentEngine::update`] (after `COMMIT`) and
/// [`DocumentEngine::update_via`] (inside the batch transaction). Keeping one
/// implementation means the event order cannot drift between the two paths.
async fn run_update_after_hooks(
    ctx: &EngineContext<'_>,
    name: &str,
    document: &mut Value,
    submitted: bool,
    changed: bool,
) -> Result<()> {
    run_hooks(ctx, name, document, false, HookPoint::AfterSave).await?;
    run_hooks(ctx, name, document, false, HookPoint::OnUpdate).await?;
    if submitted && changed {
        run_hooks(ctx, name, document, false, HookPoint::OnUpdateAfterSubmit).await?;
    }
    // Ferris has no per-field `on_change` metadata yet, so the document-level
    // point runs at most once per save, and only when a non-engine field
    // actually changed. It runs last so a listener observes the persisted state
    // and any after-submit work.
    if changed {
        run_hooks(ctx, name, document, false, HookPoint::OnChange).await?;
    }
    Ok(())
}

/// An authorization already made against a stored owner.
#[derive(Clone)]
struct PreAuthorized {
    owner: Option<String>,
}

/// Write the state the workflow validated into the merged document.
///
/// The body never reaches the blob for this key: `update_via_with` removed it
/// and kept the validated [`WorkflowMove`], so the persisted value is the one
/// `apply`/`apply_to` produced.
fn apply_workflow_state(
    merged: &mut ferris_core::Map,
    moved: Option<&super::workflow::WorkflowMove>,
) {
    if let Some(moved) = moved {
        merged.insert(
            "workflow_state".to_string(),
            Value::from(moved.outcome.state.clone()),
        );
    }
}

/// Announce a transition that has already been persisted.
fn emit_workflow_move(ctx: &EngineContext<'_>, moved: Option<&super::workflow::WorkflowMove>) {
    if let Some(moved) = moved {
        crate::workflow::global_workflow_engine().emit(
            &ctx.doctype,
            &moved.from_state,
            &moved.outcome,
        );
    }
}

/// Reject a body that moves `owner`, `creation`, `docstatus`, `amended_from`,
/// `modified_by` or `workflow_state`.
///
/// Shared by the pre-transaction authorization in [`DocumentEngine::update`]
/// and the authoritative merge in `update_via`, so both enforce one policy and
/// a denied caller cannot learn a different error than an allowed one.
///
/// Each of these accepts exactly the value already stored (or the session's id
/// for `modified_by`, or the state the workflow validated for
/// `workflow_state`), because a read-modify-write round trip legitimately sends
/// the document back unchanged. A different value is a 400: `amended_from` would
/// forge lineage, and the other five would move bookkeeping the client does not
/// own.
#[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
fn reject_system_overrides(
    incoming: &HashMap<String, Value>,
    stored: &HashMap<String, Value>,
    owner: Option<&str>,
    modified_by: Option<&str>,
    workflow_state: Option<&str>,
) -> Result<()> {
    reject_system_field_overrides(incoming, &[("owner", owner.unwrap_or(""))])?;
    // Every other engine-owned key may only repeat the stored value: a
    // read-modify-write round trip is legitimate, a moved `docstatus` would
    // skip `set_docstatus` and the workflow, a moved `amended_from` would forge
    // lineage, and a moved `idx`/`parent*` would re-file the row. Comparison is
    // by `Value` because `idx` is numeric.
    for key in [
        "creation",
        "docstatus",
        "amended_from",
        "parent",
        "parenttype",
        "parentfield",
        "idx",
    ] {
        match (stored.get(key), incoming.get(key)) {
            (_, None) => {}
            (Some(expected), Some(actual)) if expected == actual => {}
            (expected, Some(actual)) => {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "`{key}` is set by the engine and cannot be changed by the client \
                     (stored {expected:?}, got {actual:?})"
                ))));
            }
        }
    }
    if let Some(modified_by) = modified_by {
        reject_system_field_overrides(incoming, &[("modified_by", modified_by)])?;
    }
    if let Some(workflow_state) = workflow_state {
        reject_system_field_overrides(incoming, &[("workflow_state", workflow_state)])?;
    }
    Ok(())
}

/// Whether any non-engine field of `document` differs from `stored`.
///
/// A field absent from `stored` and present as `Null` in `document` is not a
/// change: both mean "no value". The engine stamps (`modified`, `docstatus`,
/// `name`, …) are excluded through [`is_engine_field`], the same predicate the
/// post-submit lock and the activity feed use.
fn document_changed(stored: &HashMap<String, Value>, document: &Value) -> bool {
    let Value::Object(map) = document else {
        return false;
    };
    map.iter().any(|(key, value)| {
        if is_engine_field(key) {
            return false;
        }
        match (stored.get(key), value) {
            (None, Value::Null) => false,
            (Some(stored_value), value) => stored_value != value,
            (None, value) => !value.is_null(),
        }
    })
}

// DOC-08 (mass assignment) and DOC-09 (workflow on update) live here because
// this patch's scope lists only this file for the update path; the shared
// `engine::tests` groups are outside it.
#[cfg(test)]
mod tests {
    use super::super::workflow::{enforce_state_edit, resolve_workflow_update};
    use super::*;
    use crate::workflow::{
        StateStyle, Workflow, WorkflowState, WorkflowTransition, global_workflow_engine,
    };
    use ferris_core::DocStatus;
    use ferris_schema::{DoctypeMeta, FieldMeta};

    const FLOW_DOCTYPE: &str = "B2 Workflow Update Doc";
    const LOCK_DOCTYPE: &str = "B2 Workflow Lock Doc";

    fn meta(doctype: &str) -> DoctypeMeta {
        DoctypeMeta {
            name: doctype.to_string(),
            module: "test".to_string(),
            fields: vec![FieldMeta {
                fieldname: "title".to_string(),
                fieldtype: "Data".to_string(),
                ..Default::default()
            }],
            hidden: true,
            ..Default::default()
        }
    }

    async fn engine() -> DocumentEngine {
        let db = ferris_db::memory_db().await.unwrap();
        crate::naming::NamingSeriesStore::ensure_table(&db)
            .await
            .unwrap();
        DocumentEngine::new(db)
    }

    fn user(id: &str, roles: &[&str]) -> ferris_core::User {
        let mut user = ferris_core::User::system_user(id.to_string(), id.to_string());
        user.id = id.to_string();
        user.roles = roles.iter().map(|role| (*role).to_string()).collect();
        user
    }

    fn state(name: &str, doc_status: DocStatus, allow_edit: &[&str]) -> WorkflowState {
        WorkflowState {
            state: name.to_string(),
            label: None,
            style: StateStyle::Default,
            doc_status,
            allow_edit: allow_edit.iter().map(|role| (*role).to_string()).collect(),
            allowed_actions: Vec::new(),
            auto_assign: None,
        }
    }

    /// One `Open → Approved` workflow, registered once per process (the
    /// registry has no deregister, so each test uses its own doctype).
    fn register(doctype: &str, allow_edit: &[&str]) {
        // The registry has no deregister and two tests share `FLOW_DOCTYPE` in
        // one process, so an `AlreadyExists` from a sibling test is expected
        // (and harmless: the definition is identical).
        let _ = global_workflow_engine().register(Workflow {
            name: format!("{doctype} workflow"),
            doctype: doctype.to_string(),
            initial_state: "Open".to_string(),
            states: vec![
                state("Open", DocStatus::Draft, allow_edit),
                state("Approved", DocStatus::Submitted, allow_edit),
            ],
            transitions: vec![WorkflowTransition {
                name: None,
                action: "Approve".to_string(),
                from_state: "Open".to_string(),
                to_state: "Approved".to_string(),
                allowed_roles: Vec::new(),
                condition: None,
                required_fields: Vec::new(),
                notify: false,
            }],
        });
    }

    fn stored_fields(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::from(*value)))
            .collect()
    }

    /// DOC-08: a body is a payload, not a document. Undeclared keys, the
    /// engine's bookkeeping (`workflow_state`, `idx`, `parent*`) and a
    /// caller-chosen `modified_by` are refused before the merge.
    #[tokio::test]
    async fn a_payload_cannot_assign_undeclared_or_engine_fields() {
        let doctype = "B2 MassAssignment Doc";
        let meta = meta(doctype);
        let engine = engine().await;
        let admin = user("admin@local", &["System Manager"]);
        let ctx = EngineContext::new(doctype.to_string(), "B2M-.#####")
            .with_meta(&meta)
            .with_user(&admin);
        let created = engine
            .create(&ctx, ferris_core::value!({ "title": "first" }))
            .await
            .unwrap();
        let name = created
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        for payload in [
            ferris_core::value!({ "workflow_state": "Approved" }),
            ferris_core::value!({ "modified_by": "mallory@example.com" }),
            ferris_core::value!({ "idx": 7 }),
            ferris_core::value!({ "parent": "OTHER" }),
            ferris_core::value!({ "amended_from": "OTHER-1" }),
            ferris_core::value!({ "not_a_declared_field": 1 }),
        ] {
            let err = engine
                .update(&ctx, &name, payload.clone())
                .await
                .unwrap_err();
            assert!(
                matches!(err, Error::Validation(_)),
                "payload {payload:?} must be refused, got {err:?}"
            );
        }

        // A declared field still updates, and `modified_by` is the session's.
        let updated = engine
            .update(&ctx, &name, ferris_core::value!({ "title": "second" }))
            .await
            .unwrap();
        assert_eq!(updated.get("title").and_then(Value::as_str), Some("second"));
        assert_eq!(
            updated.get("modified_by").and_then(Value::as_str),
            Some("admin@local")
        );
    }

    /// DOC-09: an update naming `workflow_state` takes a declared transition,
    /// and the document stores the state the workflow produced.
    #[tokio::test]
    async fn an_update_persists_the_validated_workflow_state() {
        register(FLOW_DOCTYPE, &[]);
        let meta = meta(FLOW_DOCTYPE);
        let engine = engine().await;
        let admin = user("admin@local", &["System Manager"]);
        let ctx = EngineContext::new(FLOW_DOCTYPE.to_string(), "B2W-.#####")
            .with_meta(&meta)
            .with_user(&admin);
        let created = engine
            .create(&ctx, ferris_core::value!({ "title": "flow" }))
            .await
            .unwrap();
        let name = created
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let moved = engine
            .update(
                &ctx,
                &name,
                ferris_core::value!({ "workflow_state": "Approved" }),
            )
            .await
            .expect("Open -> Approved is a declared transition");
        assert_eq!(
            moved.get("workflow_state").and_then(Value::as_str),
            Some("Approved")
        );
        let stored = engine.get(&ctx, &name).await.unwrap();
        assert_eq!(
            stored.get("workflow_state").and_then(Value::as_str),
            Some("Approved"),
            "the validated state must be persisted, not just returned"
        );

        // A state no transition reaches is refused, and the row keeps the
        // state the last transition produced.
        let err = engine
            .update(
                &ctx,
                &name,
                ferris_core::value!({ "workflow_state": "Rejected" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Workflow(_)), "{err:?}");
        let stored = engine.get(&ctx, &name).await.unwrap();
        assert_eq!(
            stored.get("workflow_state").and_then(Value::as_str),
            Some("Approved")
        );
    }

    /// DOC-09: `allow_edit` names the roles that may edit the document in a
    /// state; a caller outside the list may only move the workflow.
    #[test]
    fn allow_edit_locks_the_document_for_other_roles() {
        register(LOCK_DOCTYPE, &["Editor"]);
        let stored = stored_fields(&[("workflow_state", "Open")]);
        let other = user("bob@local", &["Other"]);
        let ctx = EngineContext::new(LOCK_DOCTYPE.to_string(), "B2L-.#####").with_user(&other);

        let err = enforce_state_edit(&ctx, &stored, &stored_fields(&[("title", "edited")]))
            .expect_err("a role outside allow_edit may not edit the data");
        assert!(matches!(err, Error::Validation(_)), "{err:?}");

        // The workflow move itself is the one thing that role may still do.
        enforce_state_edit(
            &ctx,
            &stored,
            &stored_fields(&[("workflow_state", "Approved")]),
        )
        .expect("the transition is not data editing");

        let editor = user("ada@local", &["Editor"]);
        let editor_ctx =
            EngineContext::new(LOCK_DOCTYPE.to_string(), "B2L-.#####").with_user(&editor);
        enforce_state_edit(&editor_ctx, &stored, &stored_fields(&[("title", "edited")]))
            .expect("allow_edit names this role");
    }

    /// The state machine validates on the edge, not on the action label: an
    /// update naming a state that is not a declared target is refused.
    #[test]
    fn an_undeclared_target_state_is_refused() {
        register(FLOW_DOCTYPE, &[]);
        let admin = user("admin@local", &["System Manager"]);
        let ctx = EngineContext::new(FLOW_DOCTYPE.to_string(), "B2W-.#####").with_user(&admin);
        let stored = stored_fields(&[("workflow_state", "Open")]);
        let err = resolve_workflow_update(
            &ctx,
            &stored,
            &stored_fields(&[("workflow_state", "Rejected")]),
        )
        .expect_err("Open -> Rejected is not declared");
        assert!(matches!(err, Error::Workflow(_)), "{err:?}");

        let moved = resolve_workflow_update(
            &ctx,
            &stored,
            &stored_fields(&[("workflow_state", "Approved")]),
        )
        .expect("Open -> Approved is declared")
        .expect("a move is reported");
        assert_eq!(moved.outcome.state, "Approved");
    }
}
