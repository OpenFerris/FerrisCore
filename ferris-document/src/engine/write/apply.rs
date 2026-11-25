use super::super::support::*;
use super::super::*;
use ferris_core::{DocumentError, Error, Result, Value};
use ferris_db::Tx;

impl DocumentEngine {
    /// Apply a batch of writes across DocTypes as **one** transaction.
    ///
    /// Every op runs against a single open transaction: the first failure
    /// (validation, permission, name conflict, missing row) aborts the batch
    /// and rolls the whole thing back, so a multi-DocType hook cannot leave a
    /// half-written page behind. Per-document before-hooks run as usual;
    /// after-hooks (`AfterInsert`/`AfterSave`/`OnUpdate`) run inside the
    /// transaction, exactly like [`Self::create_bulk`]. Returns the documents
    /// written by `Upsert`/`Update` ops, in op order (`Delete` contributes
    /// none).
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Invalid`] for an unknown op kind or a
    /// Single-DocType `Upsert` (a Single is materialised through `save_single`),
    /// and whatever the failing op itself returns.
    pub async fn apply(&self, ops: Vec<ApplyOp<'_>>) -> Result<Vec<Value>> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        // A Virtual DocType has no shared SQL transaction: its provider owns
        // storage and may not be re-entrant. Delegating the batch one call at a
        // time would silently lose the atomicity `apply` promises, so refuse it
        // and send the caller to `create`/`update`/`delete`.
        if let Some(op) = ops.iter().find(|op| is_virtual(&op.ctx)) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "`{}` is a Virtual DocType; `apply` cannot batch it because its provider owns storage",
                op.ctx.doctype
            ))));
        }
        // Ensure every table and resolve the shared cascade set *before*
        // opening the transaction: a DDL statement must never run inside it.
        for op in &ops {
            self.ensure_table_with(&op.ctx.doctype, op.ctx.meta).await?;
        }
        // `tab_series` too: an `Upsert` that lands as an insert mints a name
        // inside the transaction (see `prepare_insert_checked`), and the naming
        // counter's table must already exist because `bump_via` deliberately
        // runs no DDL.
        crate::naming::NamingSeriesStore::ensure_table(&self.db).await?;
        // `tab_version` as well, when history is recorded: created here so it is
        // seen by `existing_side_tables` (a delete op then cascades it) and so
        // the in-transaction recording runs no DDL (DOC7).
        self.ensure_runtime_tables().await?;
        let side_tables = self.existing_side_tables().await?;

        let mut tx = self.db.begin().await?;
        let mut out = Vec::with_capacity(ops.len());
        let result = self.apply_all(&mut tx, &ops, &side_tables, &mut out).await;
        match result {
            Ok(()) => {
                tx.commit().await?;
                Ok(out)
            }
            Err(e) => {
                // The whole batch rolls back: nothing an op wrote survives.
                tx.rollback().await.ok();
                Err(e)
            }
        }
    }

    /// The write loop of [`Self::apply`], split out so the transaction stays
    /// borrowed for the whole batch and is unwound exactly once.
    pub(super) async fn apply_all(
        &self,
        tx: &mut Tx,
        ops: &[ApplyOp<'_>],
        side_tables: &[String],
        out: &mut Vec<Value>,
    ) -> Result<()> {
        for op in ops {
            let mut sink = WriteSink::Tx(&mut *tx);
            match &op.op {
                ApplyKind::Upsert(data) => {
                    out.push(self.apply_upsert(&mut sink, &op.ctx, data.clone()).await?);
                }
                ApplyKind::Update { name, data } => {
                    let table = table_name(&op.ctx.doctype)?;
                    out.push(
                        self.update_via(&mut sink, &table, &op.ctx, name, data.clone())
                            .await?,
                    );
                }
                ApplyKind::Delete { name } => {
                    let table = table_name(&op.ctx.doctype)?;
                    self.delete_via(&mut sink, &table, &op.ctx, name, side_tables)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// The `upsert` body of an [`ApplyKind::Upsert`] op, inside `sink`.
    ///
    /// Reads first (inside the transaction): an existing row updates, a missing
    /// one inserts — the same create-or-update contract as `docstore::upsert`.
    pub(super) async fn apply_upsert(
        &self,
        sink: &mut WriteSink<'_>,
        ctx: &EngineContext<'_>,
        data: Value,
    ) -> Result<Value> {
        if is_single(ctx) {
            return Err(Error::Document(
                ferris_core::DocumentError::SingleNotCreatable {
                    doctype: ctx.doctype.clone(),
                },
            ));
        }
        let name = data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match read_row_via(sink, &self.db, &ctx.doctype, &name, ctx.meta).await {
            Ok(_) => {
                let table = table_name(&ctx.doctype)?;
                self.update_via(sink, &table, ctx, &name, data).await
            }
            Err(Error::Document(DocumentError::NotFound { .. })) => {
                let prepared = self
                    .prepare_insert_checked(ctx, None, data, true, sink)
                    .await?;
                let document = write_prepared(sink, &prepared, ctx).await?;
                let name = document
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                self.record_version(
                    sink,
                    ctx,
                    &name,
                    &document,
                    crate::versioning::ChangeType::Create,
                )
                .await?;
                Ok(document)
            }
            Err(e) => Err(e),
        }
    }
}
