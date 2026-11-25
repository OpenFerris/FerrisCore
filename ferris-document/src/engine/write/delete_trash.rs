//! Tombstone storage and the retention lifecycle (`SEC-07`/`DX-01`).
//!
//! A cancelled document is booked evidence; deleting it must not erase it. The
//! snapshot moves to `tabDeletedDocument` (a lazily-created side table, like
//! `tab_version`) and the version chain is kept. This module owns the trash
//! table, restoring a snapshot and the explicit/retention purges; the delete
//! lifecycle itself stays in the parent module.

use super::super::super::support::*;
use super::super::super::*;
use chrono::{DateTime, Duration, Utc};
use ferris_core::hooks::{DocStatus, HookPoint};
use ferris_core::permissions::Permission;
use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_db::{Stmt, sql::Column};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// The trash table. Deliberately not a normal doctype table: it has its own
/// columns and the engine, not the REST document store, is its only writer.
const TAB_DELETED_DOCUMENT: &str = "tabDeletedDocument";

/// Databases whose tombstone schema was already ensured, keyed by `Db::id`.
static TOMBSTONE_TABLES: LazyLock<Mutex<HashSet<u64>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn tombstone_ready(db: &ferris_db::Db) -> bool {
    TOMBSTONE_TABLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&db.id())
}

fn mark_tombstone_ready(db: &ferris_db::Db) {
    TOMBSTONE_TABLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(db.id());
}

/// Plain SQL types only, so the same definition creates the table on SQLite,
/// PostgreSQL and MySQL. `ensure_table` is already `IF NOT EXISTS`, so running
/// this on every boot is a no-op; there is no `ADD COLUMN` path because the
/// table did not exist before this change.
fn deleted_document_table() -> Vec<Column> {
    vec![
        Column::new("restore_token", "VARCHAR(64)")
            .not_null()
            .primary_key(),
        Column::new("doctype", "VARCHAR(255)").not_null(),
        Column::new("docname", "VARCHAR(255)").not_null(),
        Column::new("data", "TEXT").not_null(),
        Column::new("deleted_by", "VARCHAR(255)").not_null(),
        Column::new("deleted_at", "VARCHAR(64)").not_null(),
        Column::new("reason", "TEXT"),
        Column::new("restored_at", "VARCHAR(64)"),
        Column::new("legal_hold", "SMALLINT")
            .not_null()
            .default("0"),
    ]
}

/// Whether the operator placed a global legal hold on deletions.
///
/// Read from the environment with the same spelling figment uses for the
/// section (`FERRIS_RETENTION__…`), because the engine receives a pool and no
/// configuration object. `ferris-core` carries the typed `[retention]` shape;
/// wiring it into the merged `Config` is a one-line follow-up that then feeds
/// the value here without changing this contract.
pub(super) fn legal_hold_from_env() -> bool {
    std::env::var("FERRIS_RETENTION__LEGAL_HOLD")
        .ok()
        .is_some_and(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub(super) fn legal_hold_error() -> Error {
    Error::Validation(ValidationError::Invalid(
        "legal hold is active; document deletion is refused".to_string(),
    ))
}

fn actor(ctx: &EngineContext<'_>) -> String {
    ctx.user
        .as_ref()
        .map_or_else(|| "system".to_string(), |user| user.id.clone())
}

fn snapshot_timestamp(value: &Value, field: &str, fallback: i64) -> i64 {
    value
        .get(field)
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map_or(fallback, |stamp| stamp.timestamp())
}

fn row_to_deleted(row: &ferris_db::Row) -> Result<crate::versioning::DeletedDocument> {
    let parsed = |raw: String| {
        DateTime::parse_from_rfc3339(&raw)
            .ok()
            .map(|stamp| stamp.with_timezone(&Utc))
    };
    Ok(crate::versioning::DeletedDocument {
        restore_token: row.str("restore_token")?,
        doctype: row.str("doctype")?,
        docname: row.str("docname")?,
        data: row.json_object("data").unwrap_or_default(),
        deleted_by: row.str("deleted_by")?,
        deleted_at: row
            .opt_str("deleted_at")
            .and_then(parsed)
            .unwrap_or_else(Utc::now),
        reason: row.opt_str("reason"),
        restored_at: row.opt_str("restored_at").and_then(parsed),
        legal_hold: row.i64("legal_hold").unwrap_or(0) != 0,
    })
}

impl DocumentEngine {
    pub(super) async fn insert_tombstone_via(
        &self,
        sink: &mut WriteSink<'_>,
        ctx: &EngineContext<'_>,
        docname: &str,
        data_json: &str,
        reason: Option<&str>,
    ) -> Result<String> {
        let restore_token = uuid::Uuid::new_v4().to_string();
        let table = q(sink.dialect(), TAB_DELETED_DOCUMENT)?;
        sink.execute(&Stmt::new(
            format!(
                "INSERT INTO {table} \
                 (restore_token, doctype, docname, data, deleted_by, deleted_at, reason, \
                  restored_at, legal_hold) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            vec![
                Value::from(restore_token.as_str()),
                Value::from(ctx.doctype.as_str()),
                Value::from(docname),
                Value::from(data_json),
                Value::from(actor(ctx)),
                Value::from(Utc::now().to_rfc3339()),
                reason.map_or(Value::Null, Value::from),
                Value::Null,
                Value::from(0_i64),
            ],
        ))
        .await?;
        Ok(restore_token)
    }

    /// Permanently remove a document, tombstone and all, recording who did it
    /// and why. Without this explicit call a cancelled document is recoverable.
    /// A submitted document is still refused and a legal hold blocks the purge.
    ///
    /// The `tab_version` chain is kept: the purge marker carries the actor
    /// (`changed_by`) and the reason (`comment`), the forensic record that the
    /// document existed and was destroyed on purpose.
    pub async fn purge(&self, ctx: &EngineContext<'_>, name: &str, reason: &str) -> Result<()> {
        if reason.trim().is_empty() {
            return Err(Error::Validation(ValidationError::Invalid(
                "purge requires a reason".to_string(),
            )));
        }
        if is_virtual(ctx) || is_single(ctx) {
            return Err(Error::Validation(ValidationError::Invalid(
                "Virtual and Single DocTypes are not stored in the trash".to_string(),
            )));
        }
        if legal_hold_from_env() {
            return Err(Error::Validation(ValidationError::Invalid(
                "legal hold is active; purge is refused".to_string(),
            )));
        }
        let table = self.ensure_table_with(&ctx.doctype, ctx.meta).await?;
        self.ensure_runtime_tables().await?;
        let side_tables = self.existing_side_tables().await?;
        // Resolved before the transaction opens: the read and the child cascade
        // share it, and no metadata query may run on a second connection while
        // this one is checked out.
        let shape = DocShape::resolve(&self.db, &ctx.doctype, self.db.dialect(), ctx.meta).await?;
        let mut tx = self.db.begin().await?;
        let result = async {
            let (mut existing, owner_column) =
                read_row_with_shape_via(&mut WriteSink::Tx(&mut tx), &shape, &ctx.doctype, name)
                    .await?;
            let status = existing
                .get("docstatus")
                .and_then(|v| v.as_str())
                .map(docstatus_from_str)
                .unwrap_or(DocStatus::Draft);
            if status == DocStatus::Submitted {
                return Err(Error::Validation(ValidationError::Invalid(
                    "Submitted Record cannot be deleted. You must Cancel it first.".to_string(),
                )));
            }
            let trash = q(tx.dialect(), TAB_DELETED_DOCUMENT)?;
            let held = tx
                .fetch_optional(&Stmt::new(
                    format!(
                        "SELECT 1 FROM {trash} WHERE doctype = ? AND docname = ? \
                         AND legal_hold = 1 AND restored_at IS NULL"
                    ),
                    vec![Value::from(ctx.doctype.clone()), Value::from(name)],
                ))
                .await?;
            if held.is_some() {
                return Err(Error::Validation(ValidationError::Invalid(
                    "legal hold on the deleted document; purge is refused".to_string(),
                )));
            }
            self.authorize(
                ctx,
                name,
                authz_owner(ctx, owner_column.as_deref()),
                Permission::Delete,
            )
            .await?;
            run_hooks(ctx, name, &mut existing, false, HookPoint::BeforeDelete).await?;
            let affected = tx
                .execute(&Stmt::new(
                    format!("DELETE FROM {} WHERE name = ?", q(tx.dialect(), &table)?),
                    vec![Value::from(name)],
                ))
                .await?;
            if affected.is_empty() {
                return Err(Error::Document(DocumentError::NotFound {
                    doctype: ctx.doctype.clone(),
                    name: name.to_string(),
                }));
            }
            // A purge is the explicit, audited destruction of the document: its
            // child rows go with it, or they would outlive it forever.
            delete_children_via(&mut WriteSink::Tx(&mut tx), &shape, &ctx.doctype, name).await?;
            if search::ready_for(&self.db, &ctx.doctype) {
                let mut sink = WriteSink::Tx(&mut tx);
                if let Err(error) = search::remove(&mut sink, &ctx.doctype, name).await {
                    // Derived data: the purge itself must not fail because the
                    // index could not forget the row.
                    tracing::warn!(
                        %error,
                        doctype = %ctx.doctype,
                        name,
                        "purged document still present in the full-text index"
                    );
                }
            }
            for side in &side_tables {
                if side == crate::versioning::TAB_VERSION {
                    continue;
                }
                tx.execute(&Stmt::new(
                    format!(
                        "DELETE FROM {} WHERE doctype = ? AND docname = ?",
                        q(tx.dialect(), side)?
                    ),
                    vec![Value::from(ctx.doctype.clone()), Value::from(name)],
                ))
                .await?;
            }
            tx.execute(&Stmt::new(
                format!("DELETE FROM {trash} WHERE doctype = ? AND docname = ?"),
                vec![Value::from(ctx.doctype.clone()), Value::from(name)],
            ))
            .await?;
            crate::runtime::write_version(
                &mut WriteSink::Tx(&mut tx),
                &ctx.doctype,
                name,
                std::collections::HashMap::new(),
                crate::runtime::VersionMeta {
                    changed_by: &actor(ctx),
                    changed_at: Utc::now(),
                    change_type: crate::versioning::ChangeType::Delete,
                    comment: Some(format!("force purge: {}", reason.trim())),
                },
            )
            .await?;
            Ok(existing)
        }
        .await;
        match result {
            Ok(mut existing) => {
                tx.commit().await?;
                run_hooks(ctx, name, &mut existing, false, HookPoint::OnTrash).await?;
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(e)
            }
        }
    }

    /// Reinsert a soft-deleted document from its tombstone.
    ///
    /// The original name is re-used only when it is still free; a document
    /// created with the same name in the meantime is a conflict, not something
    /// to overwrite. A `Restore` version is appended so the chain explains the
    /// reappearance.
    pub async fn restore(&self, ctx: &EngineContext<'_>, restore_token: &str) -> Result<Value> {
        if is_virtual(ctx) || is_single(ctx) {
            return Err(Error::Validation(ValidationError::Invalid(
                "only regular documents are stored in the trash".to_string(),
            )));
        }
        let table = self.ensure_table_with(&ctx.doctype, ctx.meta).await?;
        self.ensure_runtime_tables().await?;
        let shape = DocShape::resolve(&self.db, &ctx.doctype, self.db.dialect(), ctx.meta).await?;
        let mut tx = self.db.begin().await?;
        match self
            .restore_via(
                &mut WriteSink::Tx(&mut tx),
                &table,
                ctx,
                restore_token,
                &shape,
            )
            .await
        {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(e)
            }
        }
    }

    async fn restore_via(
        &self,
        sink: &mut WriteSink<'_>,
        table: &str,
        ctx: &EngineContext<'_>,
        restore_token: &str,
        shape: &DocShape,
    ) -> Result<Value> {
        let trash = q(sink.dialect(), TAB_DELETED_DOCUMENT)?;
        let row = sink
            .fetch_optional(&Stmt::new(
                format!("SELECT * FROM {trash} WHERE restore_token = ?"),
                vec![Value::from(restore_token)],
            ))
            .await?
            .ok_or_else(|| {
                Error::Document(DocumentError::NotFound {
                    doctype: "Deleted Document".to_string(),
                    name: restore_token.to_string(),
                })
            })?;
        let tombstone = row_to_deleted(&row)?;
        if tombstone.restored_at.is_some() {
            return Err(Error::Validation(ValidationError::Invalid(
                "deleted document was already restored".to_string(),
            )));
        }
        if tombstone.doctype != ctx.doctype {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "tombstone belongs to `{}`, not `{}`",
                tombstone.doctype, ctx.doctype
            ))));
        }
        if read_row_with_shape_via(sink, shape, &ctx.doctype, &tombstone.docname)
            .await
            .is_ok()
        {
            return Err(Error::Document(DocumentError::AlreadyExists {
                doctype: ctx.doctype.clone(),
                name: tombstone.docname.clone(),
            }));
        }
        let snapshot = Value::Object(tombstone.data.clone());
        let now = Utc::now().timestamp();
        let creation = snapshot_timestamp(&snapshot, "creation", now);
        let owner = snapshot.get("owner").and_then(Value::as_str);
        let status = snapshot
            .get("docstatus")
            .and_then(Value::as_str)
            .map(docstatus_from_str)
            .unwrap_or(DocStatus::Draft);
        let extra_columns = column_list(&shape.columns, sink.dialect())?;
        let extra_placeholders = column_placeholders(&shape.columns, sink.dialect());
        let mut params = vec![
            Value::from(tombstone.docname.as_str()),
            owner.map_or(Value::Null, Value::from),
            Value::from(i64::from(status as i16)),
            Value::from(creation),
            Value::from(now),
            Value::from(snapshot.get("modified_by").and_then(Value::as_str)),
            Value::from(snapshot.get("amended_from").and_then(Value::as_str)),
            Value::from(snapshot.get("workflow_state").and_then(Value::as_str)),
        ];
        params.extend(column_values(&shape.columns, &snapshot));
        sink.execute(&Stmt::new(
            format!(
                "INSERT INTO {} \
                 (name, owner, docstatus, creation, modified, modified_by, amended_from, \
                  workflow_state{extra_columns}) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?{extra_placeholders})",
                q(sink.dialect(), table)?
            ),
            params,
        ))
        .await
        .map_err(|e| match &e {
            Error::Database(ferris_core::error::DatabaseError::Constraint(_)) => {
                Error::Document(DocumentError::AlreadyExists {
                    doctype: ctx.doctype.clone(),
                    name: tombstone.docname.clone(),
                })
            }
            _ => e,
        })?;
        // The snapshot carries the child arrays; restore their real rows too.
        write_children(sink, shape, &ctx.doctype, &tombstone.docname, &snapshot).await?;
        // The restored row is live again, so it must be findable again.
        if search::ready_for(&self.db, &ctx.doctype)
            && let Err(error) = search::index(sink, &ctx.doctype, &snapshot).await
        {
            tracing::warn!(
                %error,
                doctype = %ctx.doctype,
                name = %tombstone.docname,
                "restored document not indexed for full-text search"
            );
        }
        self.record_version(
            sink,
            ctx,
            &tombstone.docname,
            &snapshot,
            crate::versioning::ChangeType::Restore,
        )
        .await?;
        sink.execute(&Stmt::new(
            format!("UPDATE {trash} SET restored_at = ? WHERE restore_token = ?"),
            vec![
                Value::from(Utc::now().to_rfc3339()),
                Value::from(restore_token),
            ],
        ))
        .await?;
        Ok(snapshot)
    }

    /// The trash rows for one document, newest first.
    pub async fn deleted_documents(
        &self,
        doctype: &str,
        docname: &str,
    ) -> Result<Vec<crate::versioning::DeletedDocument>> {
        self.ensure_deleted_document_schema().await?;
        let trash = q(self.db.dialect(), TAB_DELETED_DOCUMENT)?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT * FROM {trash} WHERE doctype = ? AND docname = ? \
                     ORDER BY deleted_at DESC"
                ),
                vec![Value::from(doctype), Value::from(docname)],
            ))
            .await?;
        rows.iter().map(row_to_deleted).collect()
    }

    /// Remove tombstones whose retention window has elapsed.
    ///
    /// `deleted_documents_days == 0` means "keep forever" (the default), and a
    /// legal hold disables every automatic purge. The version chain is left in
    /// place: removing the trash row must not erase the audit history.
    pub async fn purge_expired(
        &self,
        deleted_documents_days: u32,
        legal_hold: bool,
    ) -> Result<u64> {
        if legal_hold || deleted_documents_days == 0 {
            return Ok(0);
        }
        self.ensure_deleted_document_schema().await?;
        let trash = q(self.db.dialect(), TAB_DELETED_DOCUMENT)?;
        let rows = self
            .db
            .fetch_all(&Stmt::new(
                format!(
                    "SELECT restore_token, deleted_at FROM {trash} \
                     WHERE restored_at IS NULL AND legal_hold = 0"
                ),
                Vec::new(),
            ))
            .await?;
        let cutoff = Utc::now() - Duration::days(i64::from(deleted_documents_days));
        let mut tx = self.db.begin().await?;
        let mut purged = 0_u64;
        for row in rows {
            let deleted_at = row
                .opt_str("deleted_at")
                .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
                .map_or_else(Utc::now, |stamp| stamp.with_timezone(&Utc));
            if deleted_at >= cutoff {
                continue;
            }
            let token = row.str("restore_token")?;
            tx.execute(&Stmt::new(
                format!("DELETE FROM {trash} WHERE restore_token = ?"),
                vec![Value::from(token.as_str())],
            ))
            .await?;
            purged = purged.saturating_add(1);
        }
        tx.commit().await?;
        Ok(purged)
    }

    /// Ensure the trash table exists, once per database.
    ///
    /// Called from [`Self::existing_side_tables`] too, because `apply` resolves
    /// its cascade set before opening the transaction and a delete op must not
    /// run DDL inside it.
    pub(super) async fn ensure_deleted_document_schema(&self) -> Result<()> {
        let db = self.db();
        if tombstone_ready(db) {
            return Ok(());
        }
        db.ensure_table(TAB_DELETED_DOCUMENT, &deleted_document_table())
            .await?;
        db.ensure_index(
            "idx_deleted_doc",
            TAB_DELETED_DOCUMENT,
            &["doctype", "docname"],
            false,
        )
        .await?;
        mark_tombstone_ready(db);
        Ok(())
    }
}
