use super::super::*;
use super::*;
use ferris_core::hooks::HookPoint;
use ferris_core::{DocumentError, Error, Result, SerializationError, Value};
use ferris_db::{Affected, Db, Dialect, Stmt, Tx};

pub(in crate::engine) async fn read_row_via(
    sink: &mut WriteSink<'_>,
    doctype: &str,
    name: &str,
) -> Result<(Value, Option<String>)> {
    let table = table_name(doctype)?;
    let row = sink
        .fetch_optional(&Stmt::new(
            format!(
                "SELECT data, owner FROM {} WHERE name = ?",
                q(sink.dialect(), &table)?
            ),
            vec![Value::from(name)],
        ))
        .await?;
    match row {
        Some(row) => {
            let data = row.str("data")?;
            let owner = row.opt_str("owner");
            let value = serde_json::from_str(&data)
                .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
            Ok((value, owner))
        }
        None => Err(Error::Document(DocumentError::NotFound {
            doctype: doctype.to_string(),
            name: name.to_string(),
        })),
    }
}

/// A connection or a transaction to run a row write against. Both `Db` and
/// `Tx` execute the same `Stmt`s; routing through one sink is what lets
/// `create_bulk` run its whole batch inside a *single* transaction.
pub(crate) enum WriteSink<'a> {
    Db(&'a Db),
    Tx(&'a mut Tx),
}

impl WriteSink<'_> {
    pub(crate) async fn execute(&mut self, stmt: &Stmt) -> Result<Affected> {
        match self {
            WriteSink::Db(db) => Db::execute(db, stmt).await,
            WriteSink::Tx(tx) => Tx::execute(tx, stmt).await,
        }
    }

    /// Read through the same connection the writes use, so `apply` reads the
    /// row it is about to update inside its open transaction instead of
    /// sneaking a second connection in.
    pub(crate) async fn fetch_optional(&mut self, stmt: &Stmt) -> Result<Option<ferris_db::Row>> {
        match self {
            WriteSink::Db(db) => db.fetch_optional(stmt).await,
            WriteSink::Tx(tx) => tx.fetch_optional(stmt).await,
        }
    }

    /// Read every row `stmt` matches through the sink's connection.
    ///
    /// Document Naming Rules are a *list* the naming path has to scan, and a
    /// second connection there would read a different snapshot than the
    /// transaction about to write the row.
    pub(crate) async fn fetch_all(&mut self, stmt: &Stmt) -> Result<Vec<ferris_db::Row>> {
        match self {
            WriteSink::Db(db) => db.fetch_all(stmt).await,
            WriteSink::Tx(tx) => tx.fetch_all(stmt).await,
        }
    }

    /// [`Self::fetch_optional`], requiring exactly one row. Used by the naming
    /// counter read, which the caller has just made exist.
    pub(crate) async fn fetch_one(&mut self, stmt: &Stmt) -> Result<ferris_db::Row> {
        match self {
            WriteSink::Db(db) => db.fetch_one(stmt).await,
            WriteSink::Tx(tx) => tx.fetch_one(stmt).await,
        }
    }

    pub(crate) fn dialect(&self) -> Dialect {
        match self {
            WriteSink::Db(db) => Db::dialect(db),
            WriteSink::Tx(tx) => Tx::dialect(tx),
        }
    }

    /// Whether `table` exists on the sink's connection.
    ///
    /// The naming path has to ask before reading `tabDocument Naming Rule`:
    /// that DocType is optional metadata, and a database that never migrated it
    /// has no rules rather than a broken insert. A transaction carries the same
    /// catalogue as its database, so the dialect query is repeated here instead
    /// of opening a second connection.
    pub(crate) async fn table_exists(&mut self, table: &str) -> Result<bool> {
        if let WriteSink::Db(db) = &*self {
            return db.table_exists(table).await;
        }
        let sql = match self.dialect() {
            Dialect::SQLite => "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
            Dialect::MySQL => {
                "SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_name = ?"
            }
            Dialect::Postgres => {
                "SELECT 1 FROM pg_catalog.pg_tables \
                 WHERE schemaname = current_schema() AND tablename = ?"
            }
        };
        let row = self
            .fetch_optional(&Stmt::new(sql, vec![Value::from(table)]))
            .await?;
        Ok(row.is_some())
    }
}

/// A document fully prepared for its row write: name, owner, docstatus and
/// the before-insert hook output all resolved. `write_prepared` is the only
/// step left, so `insert` (one row) and `create_bulk` (one transaction) share
/// the exact same write path.
pub(crate) struct PreparedInsert {
    pub(in crate::engine) table: String,
    pub(in crate::engine) name: String,
    /// Full document after the `Validate`/`BeforeInsert` hooks (authoritative).
    pub(in crate::engine) document: Value,
    pub(in crate::engine) dialect: Dialect,
}

/// Run one prepared insert through `sink`, then the after-insert lifecycle.
pub(in crate::engine) async fn write_prepared(
    sink: &mut WriteSink<'_>,
    prepared: &PreparedInsert,
    ctx: &EngineContext<'_>,
) -> Result<Value> {
    let mut document = insert_row(sink, prepared, ctx).await?;
    run_after_insert_hooks(ctx, &prepared.name, &mut document).await?;
    Ok(virtual_read(ctx, document)?)
}

/// Write only the document row (no after-hooks, no virtual projection).
///
/// The transactional create path uses this so `AfterInsert`/`OnUpdate` run
/// **after** the transaction commits: a hook that re-enters the engine to write
/// the same row (the `dashlink` pattern) must see the committed row, and a
/// second connection cannot see an uncommitted one.
pub(in crate::engine) async fn insert_row(
    sink: &mut WriteSink<'_>,
    prepared: &PreparedInsert,
    ctx: &EngineContext<'_>,
) -> Result<Value> {
    let mut fields = to_fields(&prepared.document)?;
    let now = chrono::Utc::now().timestamp();
    fields.insert("creation".to_string(), Value::from(rfc3339(now)?));
    fields.insert("modified".to_string(), Value::from(rfc3339(now)?));
    // `modified_by` on insert, matching Frappe's `set_user_and_timestamp`
    // (`frappe/model/document.py:1058-1059`), which stamps it on every save.
    if let Some(user) = ctx.user.as_ref() {
        fields.insert("modified_by".to_string(), Value::from(user.id.clone()));
    }
    // No `owner` fallback. This branch used to stamp the literal
    // `"Administrator"` when the owner resolved to nothing, which is the
    // *second* instance of the C1 class: a privileged value written at insert
    // time, outside any authorization decision. The owner shortcut in
    // `ferris_core::permissions::check_permission` compares `user.id`
    // against it, so any user whose id was literally `Administrator` was
    // handed `Read|Write|Amend` on the row. An unresolved owner now means
    // exactly what it says — nobody owns the row — and the `owner` column is
    // nullable, so an anonymous insert stores `NULL`.
    let docstatus = fields
        .get("docstatus")
        .and_then(|v| v.as_str())
        .map(docstatus_from_str)
        .unwrap_or(DocStatus::Draft);

    let serialized = canonical_blob(&fields_to_value(fields.clone()))?;
    let quoted = q(prepared.dialect, &prepared.table)?;
    // `docstatus` is a SMALLINT on disk and a word in `data`; the two have to
    // agree, so both are written from the same value.
    sink.execute(&Stmt::new(
        format!(
            "INSERT INTO {quoted} (name, owner, docstatus, creation, modified, data) \
             VALUES (?, ?, ?, ?, ?, ?)"
        ),
        vec![
            Value::from(field_str(&fields, "name").unwrap_or_default()),
            Value::from(fields.get("owner").and_then(|v| v.as_str())),
            Value::from(i64::from(docstatus as i16)),
            Value::from(now),
            Value::from(now),
            Value::from(serialized),
        ],
    ))
    .await
    .map_err(|e| match &e {
        Error::Database(ferris_core::error::DatabaseError::Constraint(_)) => {
            Error::Document(DocumentError::AlreadyExists {
                doctype: ctx.doctype.clone(),
                name: field_str(&fields, "name").unwrap_or_default(),
            })
        }
        _ => e,
    })?;

    Ok(fields_to_value(fields))
}

/// Run the after-insert points against a committed row.
///
/// Kept separate from [`insert_row`] so the transactional create path can call
/// it after `COMMIT`; the batch paths call it through [`write_prepared`] while
/// their transaction is open. The hooks observe the row as inserted; a hook
/// that writes it again (the `dashlink` seal) wins over the in-memory copy.
pub(in crate::engine) async fn run_after_insert_hooks(
    ctx: &EngineContext<'_>,
    name: &str,
    document: &mut Value,
) -> Result<()> {
    run_hooks(ctx, name, document, false, HookPoint::AfterInsert).await?;
    run_hooks(ctx, name, document, false, HookPoint::OnUpdate).await?;
    Ok(())
}

/// The `data` blob every write persists: JSON with **sorted keys**.
///
/// `ferris_core::Value::Object` is a `HashMap`, so `serde_json::to_string` on it
/// writes in iteration order, which is not stable between instances. The stored
/// blob, the `data` a plugin event carries and `docstore::get`'s read-back have
/// to be the *same string*, or the plugin's exact comparison fails although the
/// content is identical ("stored data differs from the data I was given" from
/// the `test` plugin's `after-save`). Going through `serde_json::to_value`
/// reaches that canonical form because its `Map` is a `BTreeMap`.
///
/// Both places that write `data` — the INSERT in [`write_prepared`] and
/// [`persist_written`] — must go through this one function; a second copy of
/// the rule would drift and resurrect the ordering bug.
#[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
pub(in crate::engine) fn canonical_blob(document: &Value) -> Result<String> {
    let canonical: serde_json::Value = serde_json::to_value(document)
        .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
    serde_json::to_string(&canonical).map_err(|e| Error::Serialization(SerializationError::Json(e)))
}
