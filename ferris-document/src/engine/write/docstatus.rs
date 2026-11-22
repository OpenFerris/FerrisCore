use super::super::support::*;
use super::super::*;
use super::enforce_workflow;
use ferris_core::hooks::{DocStatus, HookPoint};
use ferris_core::permissions::Permission;
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::Tx;

impl DocumentEngine {
    /// Transition a document's docstatus.
    ///
    /// DOC4: this used to read the row through `self.read_row` (one pool
    /// connection) and write it back through `persist_data` (a *different* one)
    /// with no transaction, so a concurrent write landing between the two was
    /// silently lost — the stale read overwrote the whole authoritative `data`
    /// blob. It also skipped the `modified`/`modified_by` stamp `update_via`
    /// applies on every save (Frappe's `set_user_and_timestamp`). The
    /// read-modify-write now happens in one transaction, on that transaction's
    /// connection, with the timestamp stamped like any other save.
    pub async fn set_docstatus(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        target: DocStatus,
    ) -> Result<Value> {
        let table = self.ensure_table(&ctx.doctype).await?;
        self.ensure_runtime_tables().await?;
        let mut tx = self.db.begin().await?;
        match self
            .set_docstatus_in_tx(&mut tx, ctx, &table, name, target)
            .await
        {
            Ok(document) => {
                tx.commit().await?;
                Ok(document)
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(e)
            }
        }
    }

    /// The body of [`Self::set_docstatus`], inside `tx`. Split out so the single
    /// transaction is either committed or unwound exactly once.
    async fn set_docstatus_in_tx(
        &self,
        tx: &mut Tx,
        ctx: &EngineContext<'_>,
        table: &str,
        name: &str,
        target: DocStatus,
    ) -> Result<Value> {
        let (current, owner_column) =
            read_row_via(&mut WriteSink::Tx(&mut *tx), &ctx.doctype, name).await?;
        let current_status = current
            .get("docstatus")
            .and_then(|v| v.as_str())
            .map(docstatus_from_str)
            .unwrap_or(DocStatus::Draft);
        // A cancelled document is not reopened in place: the auditable flow is
        // `amend` (a new copy with `amended_from`), never `Cancelled → Draft`.
        // `DocStatus::can_transition_to` still lists that pair, so the engine
        // refuses it here as well (DOC-02).
        if current_status == DocStatus::Cancelled && target == DocStatus::Draft {
            return Err(Error::Validation(ValidationError::Invalid(
                "a cancelled document cannot be reopened in place; amend it into a new draft"
                    .to_string(),
            )));
        }
        if !current_status.can_transition_to(target) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "invalid transition {current_status:?} -> {target:?}"
            ))));
        }
        let workflow_state =
            enforce_workflow(ctx, &current, current_status, target)?.map(|outcome| outcome.state);
        let action = match target {
            DocStatus::Submitted => Permission::Submit,
            DocStatus::Cancelled => Permission::Cancel,
            DocStatus::Draft => Permission::Amend,
        };
        self.authorize(ctx, name, authz_owner(ctx, owner_column.as_deref()), action)
            .await?;

        let point = match target {
            DocStatus::Submitted => HookPoint::BeforeSubmit,
            DocStatus::Cancelled => HookPoint::BeforeCancel,
            DocStatus::Draft => HookPoint::BeforeSave,
        };
        let mut document = set_docstatus_value(current, target);
        if let Some(state) = workflow_state {
            // The transition's destination state is part of the persisted row;
            // dropping it left the workflow stuck on the source state (DOC-09).
            document.set("workflow_state".to_string(), Value::from(state));
        }
        run_hooks(ctx, name, &mut document, false, point).await?;

        // Stamp the write like `update_via` does, so a transition is not the one
        // save that leaves `modified` stale (DOC4; Frappe's
        // `set_user_and_timestamp` runs on every transition too).
        let now = chrono::Utc::now().timestamp();
        if let Value::Object(ref mut map) = document {
            map.insert("modified".to_string(), Value::from(rfc3339(now)?));
            if let Some(user) = ctx.user.as_ref() {
                map.insert("modified_by".to_string(), Value::from(user.id.clone()));
            }
        }
        persist_written(&mut WriteSink::Tx(&mut *tx), table, name, &document).await?;
        let change_type = match target {
            DocStatus::Submitted => crate::versioning::ChangeType::Submit,
            DocStatus::Cancelled => crate::versioning::ChangeType::Cancel,
            DocStatus::Draft => crate::versioning::ChangeType::Update,
        };
        self.record_version(
            &mut WriteSink::Tx(&mut *tx),
            ctx,
            name,
            &document,
            change_type,
        )
        .await?;

        let after = match target {
            DocStatus::Submitted => HookPoint::OnSubmit,
            DocStatus::Cancelled => HookPoint::OnCancel,
            DocStatus::Draft => HookPoint::AfterSave,
        };
        run_hooks(ctx, name, &mut document, false, after).await?;
        Ok(redact_read(ctx, document))
    }
}
