use super::super::support::*;
use super::super::*;
use ferris_core::hooks::{DocStatus, HookPoint};
use ferris_core::permissions::Permission;
use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_db::Stmt;

impl DocumentEngine {
    /// Rename a document, running the `before_rename`/`after_rename`
    /// controller points around the write
    /// (<https://docs.frappe.io/framework/user/en/basics/doctypes/controllers>).
    ///
    /// The rename is one transaction on one connection: a later hook failure
    /// rolls the primary-key move back, so a hook never observes a half-renamed
    /// document. `name` is the primary key, and the child rows that point at it
    /// through `parent` move in the same statement pair.
    ///
    /// Only a **draft** may be renamed (DOC-15): a submitted or cancelled
    /// document's key is part of its lifecycle record, so changing it would
    /// rewrite a fact without a `Submit`/`Cancel`/`Amend` step.
    ///
    /// Side tables keyed by `(doctype, docname)` are **moved** with the document
    /// in the same transaction, and every declared `Link` field pointing at this
    /// DocType is rewritten in its own column. History, grants, comments, likes,
    /// assignments and attachments therefore follow the new key instead of
    /// being orphaned (DOC-03).
    pub async fn rename(
        &self,
        ctx: &EngineContext<'_>,
        old: &str,
        new_name: &str,
    ) -> Result<Value> {
        if ctx.meta.is_none_or(|meta| !meta.allow_rename) {
            return Err(Error::Document(DocumentError::RenameNotAllowed {
                doctype: ctx.doctype.clone(),
            }));
        }
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "a rename needs a non-empty `new_name`".to_string(),
            )));
        }

        let table = self.ensure_table_with(&ctx.doctype, ctx.meta).await?;
        self.ensure_runtime_tables().await?;
        let side_tables = self.existing_side_tables().await?;
        let mut tx = self.db.begin().await?;
        let outcome = self
            .rename_in_tx(&mut tx, ctx, &table, old, new_name, &side_tables)
            .await;
        match outcome {
            Ok(document) => {
                tx.commit().await?;
                Ok(document)
            }
            Err(error) => {
                // Preserve the original error: a failed rollback must not hide
                // why the rename was refused.
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn rename_in_tx(
        &self,
        tx: &mut ferris_db::Tx,
        ctx: &EngineContext<'_>,
        table: &str,
        old: &str,
        new_name: &str,
        side_tables: &[String],
    ) -> Result<Value> {
        let mut sink = WriteSink::Tx(tx);
        let (existing, owner) =
            read_row_via(&mut sink, &self.db, &ctx.doctype, old, ctx.meta).await?;
        self.authorize(
            ctx,
            old,
            authz_owner(ctx, owner.as_deref()),
            Permission::Write,
        )
        .await?;

        let status = existing
            .get("docstatus")
            .and_then(|v| v.as_str())
            .map(docstatus_from_str)
            .unwrap_or(DocStatus::Draft);
        if status != DocStatus::Draft {
            return Err(Error::Validation(ValidationError::Invalid(
                "only a draft document can be renamed".to_string(),
            )));
        }
        // A no-op rename still authorizes: the caller is reading a document it
        // asked to rename, and pretending otherwise would make `rename` the one
        // read path with no permission check.
        if new_name == old {
            return virtual_read(ctx, existing);
        }

        let quoted = q(sink.dialect(), table)?;
        let target = sink
            .fetch_optional(&Stmt::new(
                format!("SELECT name FROM {quoted} WHERE name = ?"),
                vec![Value::from(new_name)],
            ))
            .await?;
        if target.is_some() {
            return Err(Error::Document(DocumentError::AlreadyExists {
                doctype: ctx.doctype.clone(),
                name: new_name.to_string(),
            }));
        }

        let mut document = existing;
        let metadata = [
            ("old_name", Value::from(old)),
            ("new_name", Value::from(new_name)),
        ];
        run_hooks_with_metadata(
            ctx,
            old,
            &mut document,
            false,
            HookPoint::BeforeRename,
            &metadata,
        )
        .await?;

        let now = chrono::Utc::now().timestamp();
        document.set("name".to_string(), Value::from(new_name));
        document.set("modified".to_string(), Value::from(rfc3339(now)?));
        if let Some(user) = ctx.user.as_ref() {
            document.set("modified_by".to_string(), Value::from(user.id.clone()));
        }
        // The declared columns are part of the row; a rename that moved only
        // the key would leave the old name in a `Link` column and the next read
        // would resurrect it.
        let shape = DocShape::resolve(&self.db, &ctx.doctype, sink.dialect(), ctx.meta).await?;
        let extra = update_set(&shape.columns, sink.dialect())?;
        let mut params = vec![
            Value::from(new_name),
            Value::from(now),
            Value::from(document.get("modified_by").and_then(Value::as_str)),
        ];
        params.extend(column_values(&shape.columns, &document));
        params.push(Value::from(old));
        sink.execute(&Stmt::new(
            format!(
                "UPDATE {quoted} SET name = ?, modified = ?, \
                 modified_by = COALESCE(?, modified_by){extra} WHERE name = ?"
            ),
            params,
        ))
        .await?;
        // The index is keyed by name, so the row just moved: drop the old key
        // and write the new one in this same transaction. `document` already
        // carries `name = new_name`.
        if search::ready_for(&self.db, &ctx.doctype) {
            if let Err(error) = search::remove(&mut sink, &ctx.doctype, old).await {
                tracing::warn!(
                    %error,
                    doctype = %ctx.doctype,
                    name = old,
                    "renamed document still present under its old name in the full-text index"
                );
            }
            if let Err(error) = search::index(&mut sink, &ctx.doctype, &document).await {
                tracing::warn!(
                    %error,
                    doctype = %ctx.doctype,
                    name = %new_name,
                    "renamed document not indexed for full-text search"
                );
            }
        }
        // The child rows key on `parent`; move them with the parent in this
        // same transaction or a read after the rename loses every line item.
        // The shape is the effective `Table` list, so a child declared by a
        // `Custom Field` moves too, and it is resolved once for the whole
        // operation instead of walking the compiled declaration again.
        for child in &shape.tables {
            let child_table = table_name(&child.child)?;
            sink.execute(&Stmt::new(
                format!(
                    "UPDATE {} SET parent = ? WHERE parent = ? AND parenttype = ?",
                    q(sink.dialect(), &child_table)?
                ),
                vec![
                    Value::from(new_name),
                    Value::from(old),
                    Value::from(ctx.doctype.clone()),
                ],
            ))
            .await?;
        }
        // Every side table keyed by the document name moves with it in this
        // same transaction, so history, grants, comments, likes, assignments and
        // attachments cannot keep answering for a name that no longer exists
        // (DOC-03). The set is discovered before the transaction, so only tables
        // that exist are touched.
        for side in side_tables {
            sink.execute(&Stmt::new(
                format!(
                    "UPDATE {} SET docname = ? WHERE doctype = ? AND docname = ?",
                    q(sink.dialect(), side)?
                ),
                vec![
                    Value::from(new_name),
                    Value::from(ctx.doctype.clone()),
                    Value::from(old),
                ],
            ))
            .await?;
        }

        // A rename that left other documents pointing at the old key would be a
        // silent dangling reference. Rewrite every declared Link field whose
        // target is this DocType, in the same transaction (DOC-03).
        self.rename_links_in_tx(&mut sink, ctx, old, new_name)
            .await?;

        // AfterRename sees the new name in the hook context; the metadata keeps
        // both identities available to listeners.
        run_hooks_with_metadata(
            ctx,
            new_name,
            &mut document,
            false,
            HookPoint::AfterRename,
            &metadata,
        )
        .await?;
        virtual_read(ctx, document)
    }

    /// Rewrite every declared `Link` field that points at `ctx.doctype` so a
    /// rename does not leave the linking document pointing at a vanished key.
    ///
    /// A link lives in a real column; the scan reads only that column and
    /// updates it in place. A multi-value link column stores a JSON array of
    /// names, which is rewritten element by element. The scan is scoped to
    /// tables that actually exist; a rename is not a hot path.
    #[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
    async fn rename_links_in_tx(
        &self,
        sink: &mut WriteSink<'_>,
        ctx: &EngineContext<'_>,
        old: &str,
        new_name: &str,
    ) -> Result<()> {
        for meta in ferris_schema::list_doctypes_metas() {
            if meta.is_virtual {
                continue;
            }
            // Compiled metadata only: this runs inside the rename's transaction
            // and must not touch the customization store (a second connection
            // would deadlock a one-connection pool). A custom `Link` field that
            // is not in the compiled declaration is not rewritten here.
            let link_fields: Vec<&ferris_schema::FieldMeta> = meta
                .fields
                .iter()
                .filter(|field| {
                    normalized_fieldtype(field) == "Link"
                        && field.options.as_deref() == Some(ctx.doctype.as_str())
                })
                .collect();
            if link_fields.is_empty() {
                continue;
            }
            let Ok(table) = table_name(&meta.name) else {
                continue;
            };
            if !sink.table_exists(&table).await? {
                continue;
            }
            let quoted = q(sink.dialect(), &table)?;
            let mut select = vec![q(sink.dialect(), "name")?];
            select.extend(
                link_fields
                    .iter()
                    .map(|field| q(sink.dialect(), &field.fieldname))
                    .collect::<Result<Vec<_>>>()?,
            );
            let rows = sink
                .fetch_all(&Stmt::new(
                    format!("SELECT {} FROM {quoted}", select.join(", ")),
                    Vec::new(),
                ))
                .await?;
            for row in rows {
                let row_name = row.str("name")?;
                for field in &link_fields {
                    let Some(current) = row.opt_str(&field.fieldname) else {
                        continue;
                    };
                    let Some(replaced) = replace_link_text(&current, old, new_name) else {
                        continue;
                    };
                    sink.execute(&Stmt::new(
                        format!(
                            "UPDATE {quoted} SET {} = ? WHERE name = ?",
                            q(sink.dialect(), &field.fieldname)?
                        ),
                        vec![Value::from(replaced), Value::from(row_name.clone())],
                    ))
                    .await?;
                }
            }
        }
        Ok(())
    }
}

/// Replace `old` with `new_name` in one Link column's text.
///
/// A single-value link is the name; a multi-value link is a JSON array of names.
/// Returns the new text, or `None` when the column does not name `old`.
fn replace_link_text(text: &str, old: &str, new_name: &str) -> Option<String> {
    if text == old {
        return Some(new_name.to_string());
    }
    let parsed: Value = serde_json::from_str(text).ok()?;
    let Value::Array(items) = parsed else {
        return None;
    };
    let mut changed = false;
    let items: Vec<Value> = items
        .into_iter()
        .map(|item| match item {
            Value::String(name) if name == old => {
                changed = true;
                Value::from(new_name)
            }
            other => other,
        })
        .collect();
    changed.then(|| serde_json::to_string(&Value::Array(items)).ok())?
}

// Tests live next to the code because the A7 patch scope lists only this file;
// the shared `engine::tests` modules are outside it.
#[cfg(test)]
mod tests {
    use super::*;
    use ferris_schema::contract::NamingType;

    /// DOC-03/DOC-15: a draft rename moves the side tables and rewrites every
    /// declared Link to the renamed DocType; a non-draft rename is refused.
    #[tokio::test]
    async fn rename_moves_side_tables_updates_links_and_requires_draft() {
        ferris_schema::register_doctype(DoctypeMeta {
            name: "A7LinkTarget".to_string(),
            allow_rename: true,
            ..Default::default()
        });
        ferris_schema::register_doctype(DoctypeMeta {
            name: "A7LinkSource".to_string(),
            fields: vec![ferris_schema::FieldMeta {
                fieldname: "target".to_string(),
                fieldtype: "Link".to_string(),
                options: Some("A7LinkTarget".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });

        let db = ferris_db::memory_db().await.unwrap();
        crate::naming::NamingSeriesStore::ensure_table(&db)
            .await
            .unwrap();
        let engine = DocumentEngine::new(db.clone());
        let target_meta = ferris_schema::get_doctype("A7LinkTarget").unwrap();
        let source_meta = ferris_schema::get_doctype("A7LinkSource").unwrap();
        let target_ctx = EngineContext::new("A7LinkTarget", "")
            .with_naming(NamingType::Prompt, None)
            .with_meta(target_meta.as_ref())
            .allow_anonymous(true);
        let source_ctx = EngineContext::new("A7LinkSource", "")
            .with_naming(NamingType::Prompt, None)
            .with_meta(source_meta.as_ref())
            .allow_anonymous(true);

        engine
            .create(&target_ctx, ferris_core::value!({ "name": "A7-TGT-1" }))
            .await
            .unwrap();
        engine
            .create(
                &source_ctx,
                ferris_core::value!({ "name": "A7-SRC-1", "target": "A7-TGT-1" }),
            )
            .await
            .unwrap();

        engine
            .rename(&target_ctx, "A7-TGT-1", "A7-TGT-2")
            .await
            .unwrap();

        let source = engine.get(&source_ctx, "A7-SRC-1").await.unwrap();
        assert_eq!(
            source.get("target").and_then(|v| v.as_str()),
            Some("A7-TGT-2"),
            "the declared Link must follow the rename"
        );

        // Version history moved with the document instead of staying orphaned.
        let manager = crate::versioning::VersionManager::with_db(db.clone());
        assert!(
            manager
                .get_versions("A7LinkTarget", "A7-TGT-1")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            manager
                .get_versions("A7LinkTarget", "A7-TGT-2")
                .await
                .unwrap()
                .len(),
            1
        );

        // A submitted document cannot be renamed.
        engine
            .create(&target_ctx, ferris_core::value!({ "name": "A7-TGT-3" }))
            .await
            .unwrap();
        engine
            .set_docstatus(&target_ctx, "A7-TGT-3", DocStatus::Submitted)
            .await
            .unwrap();
        let err = engine
            .rename(&target_ctx, "A7-TGT-3", "A7-TGT-4")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("draft"), "{err:?}");
    }
}
