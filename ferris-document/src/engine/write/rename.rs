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
    /// document. `name` is the primary key and also lives in the `data` blob;
    /// both are written from the same value.
    ///
    /// Only a **draft** may be renamed (DOC-15): a submitted or cancelled
    /// document's key is part of its lifecycle record, so changing it would
    /// rewrite a fact without a `Submit`/`Cancel`/`Amend` step.
    ///
    /// Side tables keyed by `(doctype, docname)` are **moved** with the document
    /// in the same transaction, and every declared `Link` field pointing at this
    /// DocType is rewritten in the linking documents' JSON blobs. History,
    /// grants, comments, likes, assignments and attachments therefore follow the
    /// new key instead of being orphaned (DOC-03).
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

        let table = self.ensure_table(&ctx.doctype).await?;
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
        let (existing, owner) = read_row_via(&mut sink, &ctx.doctype, old).await?;
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
        let serialized = canonical_blob(&document)?;
        sink.execute(&Stmt::new(
            format!("UPDATE {quoted} SET name = ?, data = ?, modified = ? WHERE name = ?"),
            vec![
                Value::from(new_name),
                Value::from(serialized),
                Value::from(now),
                Value::from(old),
            ],
        ))
        .await?;
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
    /// The registry is the only source of the relation: a field is a link to
    /// this DocType when its `fieldtype` is `Link` and its `options` names this
    /// DocType. Values live in the `data` JSON blob (links have no column), so
    /// the update rewrites the blob of every matching row. The scan is scoped to
    /// tables that actually exist and to DocTypes that declare such a field; a
    /// rename is not a hot path.
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
            let link_fields: Vec<String> = meta
                .fields
                .iter()
                .filter(|field| {
                    field.fieldtype == "Link"
                        && field.options.as_deref() == Some(ctx.doctype.as_str())
                })
                .map(|field| field.fieldname.clone())
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
            let rows = sink
                .fetch_all(&Stmt::new(
                    format!("SELECT name, data FROM {quoted}"),
                    Vec::new(),
                ))
                .await?;
            for row in rows {
                let row_name = row.str("name")?;
                let blob = row.str("data")?;
                let mut document: Value = serde_json::from_str(&blob).map_err(|error| {
                    Error::Serialization(ferris_core::SerializationError::Json(error))
                })?;
                let mut changed = false;
                for field in &link_fields {
                    changed |= replace_link(&mut document, field, old, new_name);
                }
                if !changed {
                    continue;
                }
                sink.execute(&Stmt::new(
                    format!("UPDATE {quoted} SET data = ? WHERE name = ?"),
                    vec![
                        Value::from(canonical_blob(&document)?),
                        Value::from(row_name),
                    ],
                ))
                .await?;
            }
        }
        Ok(())
    }
}

/// Replace `old` with `new_name` in one Link field, handling a single value and
/// a multi-value (array) link field. Returns whether anything changed.
fn replace_link(document: &mut Value, field: &str, old: &str, new_name: &str) -> bool {
    let Value::Object(map) = document else {
        return false;
    };
    match map.get_mut(field) {
        Some(Value::String(value)) if value == old => {
            *value = new_name.to_string();
            true
        }
        Some(Value::Array(values)) => {
            let mut changed = false;
            for value in values {
                if let Value::String(item) = value
                    && item == old
                {
                    *item = new_name.to_string();
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
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
