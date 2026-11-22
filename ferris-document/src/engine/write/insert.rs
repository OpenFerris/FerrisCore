use super::super::support::*;
use super::super::*;
use super::mint_name;
use ferris_core::hooks::HookPoint;
use ferris_core::permissions::Permission;
use ferris_core::{Error, Result, ValidationError, Value};

impl DocumentEngine {
    /// Create a document, running validation and hooks.
    ///
    /// The resulting document is owned by the session user. A body that names a
    /// different `owner` is rejected with `400` — see [`Self::create_owned_by`]
    /// for the only supported way to insert a document owned by somebody else.
    pub async fn create(&self, ctx: &EngineContext<'_>, data: Value) -> Result<Value> {
        self.insert(ctx, None, data).await
    }

    /// Insert a document **owned by `owner`**, on behalf of the session user.
    ///
    /// This is the only way to create a document owned by someone other than
    /// its creator, and it is deliberately a separate method rather than a flag
    /// on the context or an `owner` key in `data`, for two reasons:
    ///
    /// * a request body can never reach it, so there is no path from an HTTP
    ///   payload to the ownership decision (§C1); and
    /// * every privileged insert is greppable by name.
    ///
    /// It exists because some documents are legitimately created *for* their
    /// subject rather than *by* it. `ferris-api`'s notification panel is the
    /// live case: a `Notification` addressed to `ada` has to be owned by `ada`,
    /// otherwise the owner rule in `ferris_core::permissions::check_permission`
    /// would stop her marking it read.
    ///
    /// If `data` carries an `owner` that differs from `owner`, the insert is
    /// rejected: the two must agree, so a caller cannot set one and silently
    /// get the other.
    ///
    /// Frappe permits the same thing through the ambient, process-wide
    /// `frappe.flags.in_import` global (`frappe/model/document.py:1063-1066`).
    /// That design is strictly worse — the flag is invisible at the call site and
    /// leaks into unrelated code — which is why it is not copied here.
    pub async fn create_owned_by(
        &self,
        ctx: &EngineContext<'_>,
        owner: &str,
        data: Value,
    ) -> Result<Value> {
        self.insert(ctx, Some(owner), data).await
    }

    /// Record one change-history row for `document`, inside `sink`.
    ///
    /// Always on: the engine is the single writer of `tab_version`, and the row
    /// is written through `sink`, the same connection the document is written
    /// on, so a version can never commit separately from the state it describes
    /// (the DOC1 lesson). The caller must have ensured `tab_version` exists,
    /// exactly as it ensures a doctype table, because this runs a plain `INSERT`
    /// and no DDL.
    ///
    /// `changed_by` is the acting user, or `"system"` for an anonymous/service
    /// write.
    pub(crate) async fn record_version(
        &self,
        sink: &mut WriteSink<'_>,
        ctx: &EngineContext<'_>,
        name: &str,
        document: &Value,
        change_type: crate::versioning::ChangeType,
    ) -> Result<()> {
        let changed_by = ctx.user.as_ref().map_or("system", |user| user.id.as_str());
        crate::runtime::record_version_in_tx(
            sink,
            &ctx.doctype,
            name,
            document,
            changed_by,
            change_type,
        )
        .await
    }

    /// Shared insert path. `forced_owner` is `Some` only via
    /// [`Self::create_owned_by`].
    pub(super) async fn insert(
        &self,
        ctx: &EngineContext<'_>,
        forced_owner: Option<&str>,
        data: Value,
    ) -> Result<Value> {
        self.insert_inner(ctx, forced_owner, data, false).await
    }

    /// Insert a copy that already carries the name the engine chose for it.
    ///
    /// Only [`Self::amend`] uses this: the amendment name (`NAME-1`, …) is part
    /// of the amendment contract, so it must reach the row even when the
    /// `DocType`'s naming strategy would otherwise reject a caller-supplied
    /// `name` (DOC-16). It is `pub(super)` so no request path can reach it.
    pub(super) async fn insert_with_explicit_name(
        &self,
        ctx: &EngineContext<'_>,
        data: Value,
    ) -> Result<Value> {
        self.insert_inner_with(ctx, None, data, false, true).await
    }

    /// Insert path with an explicit Single-DocType escape hatch.
    ///
    /// `allow_single` is `false` for every user-facing create: a Single has no
    /// separate documents, so [`Self::get_single`]/[`Self::save_single`] are the
    /// only way to write one. When it is `true` (only from those two methods)
    /// the insert is allowed and the `Permission::Create` gate is skipped,
    /// because materialising the Single row is a framework concern rather than
    /// the caller's action.
    pub(crate) async fn insert_inner(
        &self,
        ctx: &EngineContext<'_>,
        forced_owner: Option<&str>,
        data: Value,
        allow_single: bool,
    ) -> Result<Value> {
        self.insert_inner_with(
            ctx,
            forced_owner,
            data,
            allow_single,
            ctx.allow_explicit_name,
        )
        .await
    }

    /// [`Self::insert_inner`] plus the engine's own explicit-name escape hatch.
    async fn insert_inner_with(
        &self,
        ctx: &EngineContext<'_>,
        forced_owner: Option<&str>,
        data: Value,
        allow_single: bool,
        allow_explicit_name: bool,
    ) -> Result<Value> {
        // A Virtual DocType owns its rows: delegate the whole write to its
        // provider and never open the document table. `allow_single` is
        // irrelevant here because there is no materialised Single row to create.
        if is_virtual(ctx) {
            if forced_owner.is_some() {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "`{}` is virtual; its provider owns `owner` (create_owned_by is not supported)",
                    ctx.doctype
                ))));
            }
            let provider = virtual_doctype_provider(&ctx.doctype).ok_or_else(|| {
                Error::Document(ferris_core::DocumentError::VirtualDoctypeProviderMissing {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            self.authorize(ctx, "", None, Permission::Create).await?;
            let document = provider.create(ctx, data)?;
            return finish_virtual_write(ctx, document);
        }
        if is_single(ctx) && !allow_single {
            return Err(Error::Document(
                ferris_core::DocumentError::SingleNotCreatable {
                    doctype: ctx.doctype.clone(),
                },
            ));
        }
        // Ensure the history and share tables before the write, like the doctype
        // table, so recording and share-lookups run no DDL on the write's own
        // connection (DOC7/DOC8). DDL must never run inside the transaction.
        self.ensure_runtime_tables().await?;
        let prepared = self
            .prepare_insert_checked_with(
                ctx,
                forced_owner,
                data,
                !allow_single,
                allow_explicit_name,
                &mut WriteSink::Db(&self.db),
            )
            .await?;
        // The document row and its history row are one unit: if the version
        // insert fails (a duplicate `(doctype, docname, version)` under
        // concurrency, a database error) the document must not stay behind
        // without its audit row (DOC-04/SEC-06). Both writes share this
        // transaction, so leaving this block by error rolls both back.
        let mut tx = self.db.begin().await?;
        let outcome = self.insert_prepared_in_tx(&mut tx, ctx, &prepared).await;
        match outcome {
            Ok(mut document) => {
                tx.commit().await?;
                // After-hooks run on the committed row so a hook that re-enters
                // the engine to write the same document sees it (the `dashlink`
                // seal). They are outside the transaction by design now.
                run_after_insert_hooks(ctx, &prepared.name, &mut document).await?;
                virtual_read(ctx, document)
            }
            Err(error) => {
                // Keep the original failure; a failed rollback must not hide it.
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    /// Write a prepared insert and its `Create` version on `tx`.
    async fn insert_prepared_in_tx(
        &self,
        tx: &mut ferris_db::Tx,
        ctx: &EngineContext<'_>,
        prepared: &PreparedInsert,
    ) -> Result<Value> {
        let mut sink = WriteSink::Tx(tx);
        let document = insert_row(&mut sink, prepared, ctx).await?;
        let name = document
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        self.record_version(
            &mut sink,
            ctx,
            &name,
            &document,
            crate::versioning::ChangeType::Create,
        )
        .await?;
        Ok(document)
    }

    /// Insert many documents **inside a single transaction**.
    ///
    /// This is the `bulk-create` backing used by app plugins for mass data
    /// (sensor telemetry, imports): a list of bodies is prepared — naming,
    /// owner resolution, validation, permissions and before-insert hooks, all
    /// per document and all identical to `create` — and then *all* row writes
    /// are committed together. One doc failing (e.g. a duplicate name) rolls
    /// the whole batch back, so a sensor burst cannot leave a half-written
    /// page behind.
    ///
    /// Anonymous and empty batches are no-ops; an empty list of rows still
    /// commits an empty transaction, which keeps the caller's "everything is
    /// stored" contract unambiguous.
    pub async fn create_bulk(
        &self,
        ctx: &EngineContext<'_>,
        values: Vec<Value>,
    ) -> Result<Vec<Value>> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        // A virtual DocType has no shared SQL transaction to run the batch in:
        // its provider owns storage, so each body is delegated to `create`
        // sequentially. Unlike the SQL batch this is not atomic; a provider that
        // needs batch atomicity exposes it through its own API rather than a
        // fabricated promise here.
        if is_virtual(ctx) {
            let mut out = Vec::with_capacity(values.len());
            for data in values {
                out.push(self.create(ctx, data).await?);
            }
            return Ok(out);
        }
        if is_single(ctx) {
            return Err(Error::Document(
                ferris_core::DocumentError::SingleNotCreatable {
                    doctype: ctx.doctype.clone(),
                },
            ));
        }
        // Prepare (naming/permissions/hooks) *before* opening the transaction:
        // a validation error aborts the batch with nothing written and the
        // least amount of work wasted.
        let mut prepared = Vec::with_capacity(values.len());
        for data in values {
            prepared.push(
                self.prepare_insert(ctx, None, data, &mut WriteSink::Db(&self.db))
                    .await?,
            );
        }
        // The history table is ensured before the transaction, like `apply` does
        // for the doctype tables, so recording inside it runs no DDL (DOC7).
        self.ensure_runtime_tables().await?;
        let mut tx = self.db.begin().await?;
        let mut out = Vec::with_capacity(prepared.len());
        for batch in &prepared {
            let mut sink = WriteSink::Tx(&mut tx);
            let document = write_prepared(&mut sink, batch, ctx).await?;
            let name = document
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            self.record_version(
                &mut sink,
                ctx,
                &name,
                &document,
                crate::versioning::ChangeType::Create,
            )
            .await?;
            out.push(document);
        }
        tx.commit().await?;
        Ok(out)
    }

    /// Everything `insert` computes before the row write: name resolution,
    /// owner resolution, `validate`, permission check and the `Validate` /
    /// `BeforeInsert` lifecycle hooks. Split out of [`Self::insert`] so
    /// [`Self::create_bulk`] reuses the exact same per-document lifecycle.
    pub(super) async fn prepare_insert(
        &self,
        ctx: &EngineContext<'_>,
        forced_owner: Option<&str>,
        data: Value,
        naming: &mut WriteSink<'_>,
    ) -> Result<PreparedInsert> {
        self.prepare_insert_checked(ctx, forced_owner, data, true, naming)
            .await
    }

    /// [`Self::prepare_insert`] with an explicit `check_create` flag.
    ///
    /// The `Permission::Create` gate is skipped only for the framework's own
    /// Single-DocType materialisation (`check_create = false`); every other
    /// insert checks it, exactly as before.
    ///
    /// `naming` is the sink a minted `name` is drawn from. Pass the pool
    /// (`WriteSink::Db`) for a standalone insert; when the insert happens inside
    /// `apply`'s open transaction, pass that same transaction so the naming
    /// counter participates in the batch instead of opening a second connection
    /// (the DOC1 deadlock on a one-connection pool). The caller must have ensured
    /// `tab_series` before opening the transaction.
    pub(super) async fn prepare_insert_checked(
        &self,
        ctx: &EngineContext<'_>,
        forced_owner: Option<&str>,
        data: Value,
        check_create: bool,
        naming: &mut WriteSink<'_>,
    ) -> Result<PreparedInsert> {
        self.prepare_insert_checked_with(ctx, forced_owner, data, check_create, false, naming)
            .await
    }

    /// [`Self::prepare_insert_checked`] plus the engine's own explicit-name
    /// escape hatch (`allow_explicit_name`, only for the amend copy).
    async fn prepare_insert_checked_with(
        &self,
        ctx: &EngineContext<'_>,
        forced_owner: Option<&str>,
        data: Value,
        check_create: bool,
        allow_explicit_name: bool,
        naming: &mut WriteSink<'_>,
    ) -> Result<PreparedInsert> {
        // Authorization comes **first**: `Create` is a DocType-level grant, so it
        // needs neither the body nor the resolved owner. Asking before any
        // lifecycle work means a caller who may not create cannot make a
        // `BeforeNaming`/`BeforeValidate` hook run (hooks have effects) and
        // cannot tell a 400 from a 403 by shaping the payload. For a caller who
        // *is* allowed, the Frappe order below is unchanged: naming →
        // `before_validate` → `validate` → `before_insert`.
        if check_create {
            self.authorize(ctx, "", None, Permission::Create).await?;
        }
        let mut fields = to_fields(&data)?;
        // Field-level write permissions next: drop the fields the caller may
        // not write and hash any `Password` it did supply, before naming, hooks
        // or the row write ever see them.
        apply_field_permissions_write(ctx, &mut fields)?;
        // A virtual field is computed on read; a value here would be discarded
        // by the next read, so reject it before any hook can act on it.
        reject_virtual_field_writes(ctx, &fields)?;
        let table = self.ensure_table(&ctx.doctype).await?;

        // Frappe's `before_naming`: the controller may set the document's own
        // `name` before the engine mints one. Run it over the fields, then mint
        // only when the hook left `name` empty.
        let mut naming_document = fields_to_value(fields);
        let naming_name = naming_document
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        run_hooks(
            ctx,
            &naming_name,
            &mut naming_document,
            true,
            HookPoint::BeforeNaming,
        )
        .await?;
        fields = to_fields(&naming_document)?;

        // DOC-16: a body-supplied `name` is only authoritative for the naming
        // strategies that are *supposed* to take it from the caller (`Prompt`,
        // `Field`, and `Script` for importers/controllers). For a series,
        // naming-series field, counter or expression it would let a client mint
        // the next official invoice number before the counter does, so it is
        // refused instead of silently winning. UUID/random names are opaque
        // identifiers (no counter to steal), so an explicit one is allowed. `allow_explicit_name` is the engine's own
        // escape hatch for the amend copy, whose `NAME-1` is part of the
        // amendment contract. A Single is exempt: its name is the DocType name.
        // Anonymous contexts are the framework's own metadata/system writes
        // (`DocTypeStore` names `tabDocType` rows after the declared doctype),
        // not an identified client, so they keep the historical behaviour.
        if !allow_explicit_name
            && !is_single(ctx)
            && ctx.user.is_some()
            && field_str(&fields, "name").is_some_and(|name| !name.is_empty())
            && matches!(
                ctx.naming_type,
                NamingType::Series
                    | NamingType::NamingSeries
                    | NamingType::AutoIncrement
                    | NamingType::Expression
                    | NamingType::ExpressionOldStyle
            )
        {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "`{}` derives its name from {:?}; a client-supplied `name` is not accepted",
                ctx.doctype, ctx.naming_type
            ))));
        }

        if field_str(&fields, "name").is_none_or(|n| n.is_empty()) {
            if is_single(ctx) {
                // A Single's one row is named after the DocType (see
                // `DoctypeMeta::single_name`); a naming series would mint a
                // second name and the fixed-name read would miss it.
                fields.insert("name".to_string(), Value::from(ctx.doctype.clone()));
            } else {
                let name = mint_name(ctx, &fields, naming).await?;
                fields.insert("name".to_string(), Value::from(name));
            }
        }
        let name = field_str(&fields, "name").unwrap_or_default();

        // §C5: `docstatus` is a workflow state, not a payload field. A fresh
        // document always enters as `draft` — every transition goes through
        // `set_docstatus`, which enforces `can_transition_to` and the
        // Submit/Cancel/Amend permission. Before C5 a body could post
        // `docstatus: "submitted"` and mint an already-submitted row with no
        // permission check at all. The numeric `0` passes because the amend
        // copy (`ferris-api` `DocStore::amend`) ships it while clearing the
        // engine-owned fields; the word `draft` is accepted as the no-op it is.
        match fields.get("docstatus") {
            None => {}
            Some(value) if value == &Value::from("draft") || value.as_i64() == Some(0) => {}
            Some(other) => {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "`docstatus` transitions are a workflow action, not a \
                     payload field (got {other:?})"
                ))));
            }
        }
        fields.insert("docstatus".to_string(), Value::from("draft"));
        // `owner` comes from the session, never from the body. Frappe does the
        // same (`self.owner = self.modified_by` on insert,
        // `frappe/model/document.py:1065-1067`); the previous `or_insert_with`
        // let the caller choose the owner of the row it was creating, which
        // handed `Read|Write|Amend` over that row to whoever it named.
        let session_user = ctx.user.as_ref().map(|u| u.id.clone()).unwrap_or_default();
        let resolved_owner: String = match forced_owner {
            Some(owner) => {
                reject_system_field_overrides(&fields, &[("owner", owner)])?;
                owner.to_string()
            }
            None if session_user.is_empty() => {
                // Anonymous insert (only reachable with `allow_anonymous`):
                // drop `owner` rather than stamping an empty string over a
                // client-provided value.
                fields.remove("owner");
                String::new()
            }
            None => {
                reject_system_field_overrides(&fields, &[("owner", session_user.as_str())])?;
                session_user
            }
        };
        if !resolved_owner.is_empty() {
            fields.insert("owner".to_string(), Value::from(resolved_owner.clone()));
        }

        // A Single is materialised by the framework, not authored field by
        // field, so its declared defaults fill whatever the caller left unset
        // (see `apply_single_defaults`).
        if is_single(ctx) {
            apply_single_defaults(ctx.meta, &mut fields);
        }

        // Frappe's `before_validate` runs before schema validation, so it can
        // fill a field the validator would otherwise reject as missing. The
        // name is re-read afterwards: a hook that changes it must be what the
        // row and the after-hooks see.
        let mut document = fields_to_value(fields);
        run_hooks(ctx, &name, &mut document, true, HookPoint::BeforeValidate).await?;
        let fields = to_fields(&document)?;
        let name = field_str(&fields, "name").unwrap_or_default();

        validate(ctx.meta, &document)?;

        run_hooks(ctx, &name, &mut document, true, HookPoint::Validate).await?;
        run_hooks(ctx, &name, &mut document, true, HookPoint::BeforeInsert).await?;

        Ok(PreparedInsert {
            table,
            name,
            document,
            dialect: self.db.dialect(),
        })
    }
}

// Tests live next to the code because the A7 patch scope lists only this file;
// the shared `engine::tests` modules are outside it.
#[cfg(test)]
mod tests {
    use super::*;

    /// DOC-16: a series name is minted by the counter, so a caller-supplied
    /// `name` is refused instead of letting a client mint an official number.
    #[tokio::test]
    async fn a_series_doctype_refuses_a_client_supplied_name() {
        let db = ferris_db::memory_db().await.unwrap();
        crate::naming::NamingSeriesStore::ensure_table(&db)
            .await
            .unwrap();
        let engine = DocumentEngine::new(db);
        let mut user = ferris_core::User::system_user("a7@local".to_string(), "A7".to_string());
        user.id = "a7@local".to_string();
        user.roles = vec!["System Manager".to_string()];
        let ctx = EngineContext::new("A7SeriesDoc", "A7-.#####").with_user(&user);

        let err = engine
            .create(
                &ctx,
                ferris_core::value!({ "name": "A7-00001", "title": "stolen" }),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("client-supplied"),
            "a series name must not come from the body: {err:?}"
        );
    }

    /// DOC-04/SEC-06: the document row and its version row share one
    /// transaction. A version insert failure must leave no document behind.
    #[tokio::test]
    async fn a_version_failure_rolls_the_document_back() {
        let db = ferris_db::memory_db().await.unwrap();
        crate::naming::NamingSeriesStore::ensure_table(&db)
            .await
            .unwrap();
        crate::runtime::ensure_version_table(&db).await.unwrap();
        // Force the version INSERT to fail after the document INSERT has run.
        db.execute(&ferris_db::Stmt::new(
            "CREATE TRIGGER a7_fail_version BEFORE INSERT ON tab_version \
             BEGIN SELECT RAISE(ABORT, 'a7 forced version failure'); END",
            Vec::new(),
        ))
        .await
        .unwrap();

        let engine = DocumentEngine::new(db.clone());
        let ctx = EngineContext::new("A7AtomicCreate", "A7A-.#####").allow_anonymous(true);
        assert!(
            engine
                .create(&ctx, ferris_core::value!({ "title": "must roll back" }))
                .await
                .is_err()
        );

        let table = ferris_schema::migration::document_table("A7AtomicCreate").unwrap();
        let rows = db
            .fetch_one(&ferris_db::Stmt::new(
                format!("SELECT COUNT(*) AS n FROM {table}"),
                Vec::new(),
            ))
            .await
            .unwrap()
            .count("n");
        assert_eq!(
            rows, 0,
            "the document must roll back with the version that failed"
        );
    }
}
