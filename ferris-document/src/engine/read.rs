use super::support::*;
use super::*;
use crate::filters::{self};
use crate::list_sql;
use ferris_core::permissions::Permission;
use ferris_core::permissions::ReadScope;
use ferris_core::{DocumentError, Error, Result, ValidationError, Value};
use ferris_db::Db;
use ferris_schema::DoctypeMeta;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock, RwLock};

impl DocumentEngine {
    /// The document engine over `db`.
    ///
    /// Every engine records change history and consults document shares: there
    /// is no opt-out flag. `tab_version` and `tab_share` are created lazily (and
    /// cached process-wide per database), so a caller never has to know whether
    /// a site ran the side-table migration.
    pub fn new(db: Db) -> Self {
        Self {
            db,
            ensured: Arc::new(OnceLock::new()),
        }
    }

    /// The backing database, for callers that need the same connection.
    pub fn db(&self) -> &Db {
        &self.db
    }

    pub(super) fn ensured_tables(&self) -> &RwLock<HashSet<String>> {
        self.ensured.get_or_init(|| RwLock::new(HashSet::new()))
    }

    /// Create the table for `doctype` if needed (cached per process).
    pub async fn ensure_table(&self, doctype: &str) -> Result<String> {
        let table = table_name(doctype)?;
        if self
            .ensured_tables()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&table)
        {
            return Ok(table);
        }
        // The shared definition, so a lazily-created table and a migrated one
        // are the same table. `ensure_table` is already `IF NOT EXISTS`, so this
        // is a no-op when the migration already ran.
        self.db
            .ensure_table(&table, &ferris_schema::migration::document_columns())
            .await?;
        for (name, columns, unique) in ferris_schema::migration::document_indexes() {
            self.db.ensure_index(name, &table, columns, unique).await?;
        }
        // The DocType's own `meta.indexes` joined the DDL in
        // [`ferris_schema::migration::create_table_statements`], so the lazy
        // path has to materialize the same declarations or a table created here
        // and one created by `ferris migrate` stop being the same table. The
        // migration runner sees `meta.indexes` from the in-process registry, so
        // this looks the same declaration up there; both DDL paths classify an
        // entry identically and both ensures are idempotent, so running them
        // after a migration is a no-op. A DocType with no registered metadata
        // (a test, a runtime name) declares no indexes, which is the status quo.
        if let Some(meta) = ferris_schema::get_doctype(doctype) {
            for index in &meta.indexes {
                match ferris_schema::migration::classify_index(index) {
                    ferris_schema::migration::DeclaredIndex::Columns => {
                        let columns: Vec<&str> = index.fields.iter().map(String::as_str).collect();
                        self.db
                            .ensure_index(&index.name, &table, &columns, index.unique)
                            .await?;
                    }
                    ferris_schema::migration::DeclaredIndex::JsonField(field) => {
                        // A blob field has no column; its index is over the JSON
                        // expression, which is also the expression `list` pushes
                        // filters and ordering into — the two are the same
                        // `json_field_target` spelling by construction.
                        self.db
                            .ensure_json_index(&index.name, &table, field, index.unique)
                            .await?;
                    }
                    ferris_schema::migration::DeclaredIndex::Unsupported => {}
                }
            }
            // `FieldMeta::unique` is a constraint, not only migration DDL: a
            // table created lazily must get the same unique index the migration
            // path builds, or the first write to a fresh install can duplicate
            // a business key (LOGIC-01).
            for field in &meta.fields {
                if !field.unique
                    || field.r#virtual
                    || ferris_schema::fieldtype::is_layout(&field.fieldtype)
                {
                    continue;
                }
                let name = format!("{}_unique", field.fieldname);
                let column = ferris_schema::migration::document_columns()
                    .iter()
                    .any(|column| column.name == field.fieldname);
                if column {
                    self.db
                        .ensure_index(&name, &table, &[&field.fieldname], true)
                        .await?;
                } else {
                    self.db
                        .ensure_json_index(&name, &table, &field.fieldname, true)
                        .await?;
                }
            }
        }
        self.ensured_tables()
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(table.clone());
        Ok(table)
    }

    /// Read a row **without any authorization**.
    ///
    /// Private on purpose. `get()` and `list()` are the only public reads, and
    /// both authorize; this one exists for the lifecycle operations
    /// (`update`/`delete`/`set_docstatus`), which already authorized against
    /// the *stored* owner with the correct action — `Write`, `Delete`,
    /// `Submit` — and must not be re-gated on `Read` first, or a principal
    /// granted `Write` without `Read` would be unable to save.
    ///
    /// Returns the document together with the `owner` **column**. The column,
    /// not the `owner` key inside the `data` blob, is the authoritative owner:
    /// nothing in a request body can write a column directly, which is the same
    /// reason C1's fix refuses to trust the payload.
    pub(super) async fn read_row(
        &self,
        doctype: &str,
        name: &str,
    ) -> Result<(Value, Option<String>)> {
        self.ensure_table(doctype).await?;
        read_row_via(&mut WriteSink::Db(&self.db), doctype, name).await
    }

    /// Fetch a document by name, authorized as `ctx.user`.
    ///
    /// The DocType comes from the context, not from a separate argument: a
    /// mismatch between the two used to be possible, and a permission check
    /// against the wrong DocType is a permission check against nothing.
    ///
    /// A principal with no `Read` right gets `PermissionDenied` rather than
    /// `NotFound`, matching Frappe (`frappe/permissions.py::has_permission`).
    /// That does confirm the row exists, which is an existence oracle a
    /// `404` would not have. Kept deliberately for parity with Frappe; it is
    /// recorded as a residual in §C2 of the audit.
    pub async fn get(&self, ctx: &EngineContext<'_>, name: &str) -> Result<Value> {
        self.get_for(ctx, name, Permission::Read).await
    }

    /// Read a row while requiring `action` instead of `Read`.
    ///
    /// Used by operations that must read a document they are not merely
    /// allowed to see — `amend` clones a submitted document into a fresh draft,
    /// so it needs `Permission::Amend` (Frappe lists `amend` among the
    /// per-DocType rights, `frappe/permissions.py:22`). Gating that on `Read`
    /// alone would let any principal who can see a submitted document duplicate
    /// it, which is the same escalation C1 was about, one level down.
    pub async fn get_for(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        action: Permission,
    ) -> Result<Value> {
        if is_virtual(ctx) {
            // A Virtual DocType has no row to read, so the permission decision
            // is made against the DocType's rules first and the provider is the
            // only data source. The engine never invents a `tab…` read here.
            self.authorize(ctx, name, None, action).await?;
            let provider = virtual_doctype_provider(&ctx.doctype).ok_or_else(|| {
                Error::Document(DocumentError::VirtualDoctypeProviderMissing {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            return virtual_read(ctx, provider.get(ctx, name)?);
        }
        let (value, owner) = self.read_row(&ctx.doctype, name).await?;
        self.ensure_runtime_tables().await?;
        self.authorize(ctx, name, authz_owner(ctx, owner.as_deref()), action)
            .await?;
        // FRA-03: the role rules admit the *action*; a `User Permission` narrows
        // which rows. The decision needs the stored document, so it runs here,
        // once per read, and never as a second policy path.
        enforce_user_permission_read(ctx, &value)?;
        Ok(virtual_read(ctx, value)?)
    }

    /// Read the one document of a Single `DocType`, creating it with its field
    /// defaults on first access.
    ///
    /// A Single (`is_single = true`, Frappe `Single DocType`) has exactly one
    /// document, named after the `DocType` itself, with no list view and no
    /// caller-chosen name. Frappe materialises that row during `migrate`; here
    /// the row is created lazily so a freshly registered Single exists the first
    /// time the Desk opens it, whether or not a migration has run since.
    ///
    /// Materialising the row is a framework concern, not a user action, so the
    /// auto-create **skips `Permission::Create`**: a principal with only `Read`
    /// can open the Single and the defaults are written on their behalf. Every
    /// subsequent read and write is authorized normally (role rules, no owner
    /// shortcut — see [`authz_owner`]).
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Invalid`] when the `DocType` is not a Single,
    /// and a permission or storage error from the read/insert it performs.
    pub async fn get_single(&self, ctx: &EngineContext<'_>) -> Result<Value> {
        if !is_single(ctx) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "`{}` is not a Single DocType",
                ctx.doctype
            ))));
        }
        // A Single is not a public document by virtue of being single: a read
        // needs an identity (`Read`), and the lazy materialisation below is a
        // framework write on that identity's behalf (DOC-01).
        require_authenticated(ctx, &ctx.doctype, Permission::Read)?;
        let name = ctx.doctype.clone();
        match self.read_row(&ctx.doctype, &name).await {
            Ok((value, owner)) => {
                self.ensure_runtime_tables().await?;
                self.authorize(
                    ctx,
                    &name,
                    authz_owner(ctx, owner.as_deref()),
                    Permission::Read,
                )
                .await?;
                enforce_user_permission_read(ctx, &value)?;
                Ok(virtual_read(ctx, value)?)
            }
            Err(Error::Document(DocumentError::NotFound { .. })) => {
                // Materialising the row is a write. It used to skip permission
                // entirely, so any authenticated caller (even one with no grant
                // on the DocType) could create the Single and receive its
                // defaults (DOC-01). A DocType-level `Read` is the minimum for
                // asking to see it.
                self.ensure_runtime_tables().await?;
                self.authorize(ctx, &name, None, Permission::Read).await?;
                self.insert_inner(ctx, None, Value::Object(ferris_core::Map::new()), true)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    /// Save the one document of a Single `DocType` (upsert).
    ///
    /// The document is created with its defaults if it does not exist yet and
    /// updated in place otherwise, so a client never has to know whether the
    /// row has been materialised. `name` is always the `DocType` name; a body
    /// that tries to name the document something else is rejected by the update
    /// path (the engine re-asserts `name`).
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Invalid`] when the `DocType` is not a Single,
    /// and a permission, validation or storage error from the update/insert it
    /// performs.
    pub async fn save_single(&self, ctx: &EngineContext<'_>, data: Value) -> Result<Value> {
        if !is_single(ctx) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "`{}` is not a Single DocType",
                ctx.doctype
            ))));
        }
        // Saving a Single is a write even when the row does not exist yet. An
        // anonymous context has no identity to authorize, so it is refused
        // before the create-vs-update branch (DOC-01).
        require_authenticated(ctx, &ctx.doctype, Permission::Create)?;
        let name = ctx.doctype.clone();
        match self.read_row(&ctx.doctype, &name).await {
            Ok(_) => self.update(ctx, &name, data).await,
            Err(Error::Document(DocumentError::NotFound { .. })) => {
                // Creating a Single is still a create: it must be authorized as
                // such before the defaults are written (DOC-01).
                self.ensure_runtime_tables().await?;
                self.authorize(ctx, &name, None, Permission::Create).await?;
                let mut data = match data {
                    Value::Object(map) => map,
                    _ => ferris_core::Map::new(),
                };
                data.insert("name".to_string(), Value::from(name));
                self.insert_inner(ctx, None, Value::Object(data), true)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    /// List documents.
    ///
    /// Filtering, ordering and pagination are pushed into SQL wherever the
    /// storage layer can answer them — see [`crate::list_sql`] for what that
    /// covers and what it cannot. Two things have to hold before SQL may page:
    ///
    /// * the read decision must be known for the whole DocType up front, or
    ///   `LIMIT` would be applied to rows that are about to be dropped and the
    ///   page would come back short with a `total` counting unreadable rows;
    /// * every filter must have reached SQL, or the same `total` would be a
    ///   count of the pushed-down subset only.
    ///
    /// When either is false the list falls back to filtering in Rust, and
    /// `total` counts what survived so the two always agree. Frappe reaches the
    /// same agreement differently: `DatabaseQuery.execute()` folds the permission
    /// condition into the SQL (`add_user_permissions` /
    /// `get_permission_query_conditions`) so it never has a fallback — it has
    /// real columns for every field, which is the difference
    /// `ferris-schema/src/migration.rs:41-50` makes for a blob-backed store.
    pub async fn list(
        &self,
        ctx: &EngineContext<'_>,
        params: &crate::ListQuery,
    ) -> Result<(Vec<Value>, u64)> {
        if is_virtual(ctx) {
            // The provider owns filtering, ordering and pagination (see
            // `VirtualDoctypeProvider::list`); the engine still enforces the
            // DocType-level `Read` rule before handing it the query.
            self.authorize(ctx, ctx.doctype.as_str(), None, Permission::Read)
                .await?;
            let provider = virtual_doctype_provider(&ctx.doctype).ok_or_else(|| {
                Error::Document(DocumentError::VirtualDoctypeProviderMissing {
                    doctype: ctx.doctype.clone(),
                })
            })?;
            let documents = provider.list(ctx, params)?;
            let total = documents.len() as u64;
            let documents = documents
                .into_iter()
                .map(|value| virtual_read(ctx, value))
                .collect::<Result<Vec<_>>>()?;
            return Ok((documents, total));
        }
        let doctype = ctx.doctype.as_str();
        let table = self.ensure_table(doctype).await?;
        let dialect = self.db.dialect();

        let scope = match ctx.user.as_ref() {
            // An anonymous context opted into reads sees every row by
            // definition — `can_read` returns `Ok` for all of them — so there
            // is nothing to filter.
            None if ctx.allow_anonymous => ReadScope::All,
            None => ReadScope::None,
            Some(user) => ferris_core::permissions::read_scope(user, doctype),
        };

        // FRA-03: with a `User Permission` installed every row is a function of
        // its own `Link` values, which SQL cannot express (the fields live in
        // the JSON blob). `read_scope` already answers `PerRow`; this keeps the
        // "SQL may not page it" guarantee independent of the policy
        // classification, so `LIMIT` can never shorten the page or inflate
        // `total` with rows the caller cannot read.
        let value_restricted = ctx.user.as_ref().is_some_and(|user| {
            ferris_core::permissions::has_user_permissions_for(&user.id, doctype)
        });

        // Documents the caller may read only because of a share (DOC8): the role
        // scope in `plan`/`can_read` does not know about `tab_share`, so a shared
        // document would otherwise be missing from the caller's list. Loaded once
        // per page — a set of names, not a query per row.
        self.ensure_runtime_tables().await?;
        let shared = self.readable_share_names(ctx).await?;
        let scope = if scope == ReadScope::None && !shared.is_empty() {
            // No role policy grants a read at all, but a share does. Force the
            // per-row path, where `can_read` admits the shared names by name.
            ReadScope::PerRow
        } else {
            scope
        };
        if scope == ReadScope::None {
            return Ok((Vec::new(), 0));
        }

        // The fields this DocType declares a single-field index over, which the
        // plan may push into SQL. Prefer the context's metadata (the API layer
        // hands over the customized one); fall back to the registry so callers
        // that build a bare context (e.g. the plugin host) still get the
        // pushdown for registered DocTypes. A DocType with no metadata just has
        // no pushable fields, which is the previous behaviour.
        let json_indexed_fields: Vec<String> = ctx
            .meta
            .map(DoctypeMeta::json_indexed_fields)
            .or_else(|| ferris_schema::get_doctype(doctype).map(|m| m.json_indexed_fields()))
            .unwrap_or_default();

        let (mut plan, residual) = list_sql::Plan::build(
            &params.filters,
            &params.filter_ops,
            params.search.as_deref(),
            params.order_by.as_deref(),
            &json_indexed_fields,
        );
        if let Some(only) = &params.only_names {
            // A name set is a `name IN (...)`: the one predicate the engine's
            // callers use to hand it a decision reached elsewhere (see
            // `ListQuery::only_names`), and it is always column-backed.
            if only.is_empty() {
                // An empty allow-list matches nothing. Short-circuit rather than
                // emitting `IN ()`, which is a syntax error on every dialect.
                return Ok((Vec::new(), 0));
            }
            plan = plan.with_in("name", only.iter().cloned().collect())?;
        }
        match scope {
            // `All` needs no narrowing; `PerRow` must not be narrowed here,
            // because a `WHERE owner = ?` would drop rows the policy allows to
            // somebody who merely can also read their own. `Owner` likewise must
            // not be narrowed when shares exist: a `WHERE owner = ?` would drop
            // the documents the caller only reaches through a share (DOC8).
            ReadScope::Owner => {
                let user = ctx.user.as_ref();
                if let Some(user) = user
                    && shared.is_empty()
                {
                    plan = plan.with_equality("owner", Value::string(&user.id))?;
                }
            }
            ReadScope::All | ReadScope::PerRow => {}
            ReadScope::None => unreachable!("handled above"),
        }
        if !residual.is_empty() {
            tracing::debug!(
                doctype,
                residual = ?residual,
                "filters stayed in Rust: the plan records what could not be pushed into SQL"
            );
        }

        let offset = params.offset.unwrap_or(0);
        let limit = params.limit.unwrap_or(20);
        // SQL may own the page only when it also owns the count.
        let page_is_exhaustive = plan.is_complete()
            && scope != ReadScope::PerRow
            && shared.is_empty()
            && !value_restricted;

        let (documents, total) = if page_is_exhaustive {
            let rows = self
                .db
                .fetch_all(&plan.select_page(&table, dialect, limit, offset)?)
                .await?;
            let total = self
                .db
                .fetch_one(&plan.count(&table, dialect)?)
                .await?
                .count("n");
            let documents: Vec<Value> = rows
                .iter()
                .filter_map(|row| {
                    let data = row.str("data").ok()?;
                    serde_json::from_str(&data).ok()
                })
                .collect();
            (documents, total)
        } else {
            let rows = self
                .db
                .fetch_all(&plan.select_all(&table, dialect)?)
                .await?;
            let mut documents: Vec<Value> = rows
                .iter()
                .filter_map(|row| {
                    let owner = row.opt_str("owner");
                    let owner = owner.as_deref();
                    let data = row.str("data").ok()?;
                    let value: Value = serde_json::from_str(&data).ok()?;
                    // The resource identity the policy is evaluated against is the
                    // row's own name. Read it from the document, which the engine
                    // guarantees carries `name`.
                    let doc_name = value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    // A refused row is dropped, not fatal: one unreadable document
                    // must not empty the whole page. A row the caller can only
                    // read through a share is admitted by name (DOC8).
                    if !shared.contains(&doc_name) {
                        can_read(ctx, &doc_name, owner).ok()?;
                    }
                    // FRA-03: a `User Permission` narrows the rows even when the
                    // role/share decision admits them.
                    if !user_permission_allows(ctx, &value) {
                        return None;
                    }
                    Some(value)
                })
                .filter(|value| params.matches(value))
                .collect();

            // A `WHERE` cannot order by a field it has no column for, so the
            // sort stays in Rust whenever the plan could not express it.
            let order_by = params
                .order_by
                .as_deref()
                .filter(|expr| !expr.trim().is_empty())
                .filter(|_| plan.orders_in_rust());
            if let Some(order_by) = order_by {
                documents = filters::order_documents(documents, order_by);
            }

            let total = documents.len() as u64;
            let documents = documents
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            (documents, total)
        };

        // Field-level read permissions and `Password` masking happen last, on
        // the rows that survived the filter and the page, so filtering and
        // search still see the real values.
        let mut documents: Vec<Value> = documents
            .into_iter()
            .map(|value| virtual_read(ctx, value))
            .collect::<Result<Vec<_>>>()?;
        if let Some(fields) = &params.fields {
            documents = documents
                .into_iter()
                .map(|value| project(value, fields))
                .collect();
        }
        Ok((documents, total))
    }
}

/// Refuse an operation whose context has no acting user.
///
/// `allow_anonymous` is not an identity: a Single's lazy row (and any other
/// operation that needs an owner, a share or a role decision) cannot be
/// authorized without one. The error is the same typed denial a failed
/// permission check produces, so callers keep matching one variant.
#[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
fn require_authenticated(ctx: &EngineContext<'_>, name: &str, action: Permission) -> Result<()> {
    if ctx.user.is_some() {
        return Ok(());
    }
    Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
        action: action.as_str().to_string(),
        resource: format!("{} {name}", ctx.doctype),
    }))
}

// Tests live next to the code because the A7 patch scope lists only this file;
// the shared `engine::tests` modules are outside it.
#[cfg(test)]
mod tests {
    use super::*;

    fn single_meta() -> DoctypeMeta {
        DoctypeMeta {
            name: "A7SingleSettings".to_string(),
            is_single: true,
            fields: vec![ferris_schema::FieldMeta {
                fieldname: "company".to_string(),
                fieldtype: "Data".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    async fn row_count(db: &Db, doctype: &str) -> u64 {
        let table = ferris_schema::migration::document_table(doctype).unwrap();
        if !db.table_exists(&table).await.unwrap() {
            return 0;
        }
        db.fetch_one(&ferris_db::Stmt::new(
            format!("SELECT COUNT(*) AS n FROM {table}"),
            Vec::new(),
        ))
        .await
        .unwrap()
        .count("n")
    }

    /// DOC-01: materialising a Single is a write, and an anonymous context is
    /// not an identity — even when it opted into anonymous access.
    #[tokio::test]
    async fn single_materialisation_denies_anonymous_read_and_create() {
        let db = ferris_db::memory_db().await.unwrap();
        let engine = DocumentEngine::new(db.clone());
        let meta = single_meta();
        let ctx = EngineContext::new("A7SingleSettings", "")
            .with_meta(&meta)
            .allow_anonymous(true);

        let err = engine.get_single(&ctx).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
            ),
            "anonymous get_single must be denied, got {err:?}"
        );
        let err = engine
            .save_single(&ctx, ferris_core::value!({ "company": "x" }))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
            ),
            "anonymous save_single must be denied, got {err:?}"
        );
        assert_eq!(
            row_count(&db, "A7SingleSettings").await,
            0,
            "a denied request must not materialise the row"
        );
    }

    /// DOC-01: an authenticated principal with the DocType grant materialises
    /// the Single normally, so the guard closes the anonymous hole only.
    #[tokio::test]
    async fn single_materialisation_allows_an_authenticated_reader() {
        let db = ferris_db::memory_db().await.unwrap();
        let engine = DocumentEngine::new(db.clone());
        let meta = single_meta();
        let mut user = ferris_core::User::system_user("a7@local".to_string(), "A7".to_string());
        user.id = "a7@local".to_string();
        user.roles = vec!["System Manager".to_string()];
        let ctx = EngineContext::new("A7SingleSettings", "")
            .with_meta(&meta)
            .with_user(&user);

        let created = engine.get_single(&ctx).await.unwrap();
        assert_eq!(
            created.get("name").and_then(|v| v.as_str()),
            Some("A7SingleSettings")
        );
        assert_eq!(row_count(&db, "A7SingleSettings").await, 1);
    }

    /// PERF-05: a lifecycle filter is converted to the `SMALLINT` column, so the
    /// list is a `WHERE docstatus = ?` instead of a full scan and a Rust word
    /// comparison. The behavioural half lives here; the generated SQL is pinned
    /// in `list_sql::plan::tests`.
    #[tokio::test]
    async fn a_docstatus_filter_returns_only_the_lifecycle_state() {
        let db = ferris_db::memory_db().await.unwrap();
        let engine = DocumentEngine::new(db);
        let ctx = EngineContext::new("C3StatusList", "C3-.#####").allow_anonymous(true);
        let draft = engine
            .create(&ctx, ferris_core::value!({ "title": "draft" }))
            .await
            .unwrap();
        let submitted = engine
            .create(&ctx, ferris_core::value!({ "title": "submitted" }))
            .await
            .unwrap();
        let name = |value: &Value| {
            value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        engine
            .set_docstatus(&ctx, &name(&submitted), crate::DocStatus::Submitted)
            .await
            .unwrap();

        let params = crate::ListQuery {
            filter_ops: vec![crate::filters::ListFilter {
                field: "docstatus".to_string(),
                op: crate::filters::FilterOp::Equals,
                value: Value::string("submitted"),
                or: false,
            }],
            ..Default::default()
        };
        let (documents, total) = engine.list(&ctx, &params).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(name(&documents[0]), name(&submitted));
        assert_ne!(name(&documents[0]), name(&draft));
    }

    /// FRA-03, end to end: a user limited to `Customer A` sees only `Customer A`
    /// in a list and is refused `Customer B` by name; with no rule installed the
    /// same two calls return both rows and the document.
    #[tokio::test]
    async fn a_user_permission_narrows_the_list_and_the_single_read() {
        use ferris_core::permissions::{
            UserPermissionRule, UserPermissionRules, UserPermissionSource,
            clear_user_permission_source, set_user_permission_source,
        };
        use std::sync::Arc;

        struct Static(Vec<UserPermissionRule>);
        impl UserPermissionSource for Static {
            fn rules_for(&self, user: &str) -> UserPermissionRules {
                UserPermissionRules::from(
                    self.0
                        .iter()
                        .filter(|rule| rule.user == user)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            }
        }

        static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        struct Installed {
            _guard: std::sync::MutexGuard<'static, ()>,
        }
        impl Drop for Installed {
            fn drop(&mut self) {
                clear_user_permission_source();
            }
        }
        fn install(rules: Vec<UserPermissionRule>) -> Installed {
            let guard = SERIALISE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            set_user_permission_source(Arc::new(Static(rules)));
            Installed { _guard: guard }
        }

        let meta = DoctypeMeta {
            name: "D1Customer".to_string(),
            fields: vec![ferris_schema::FieldMeta::data("title")],
            ..Default::default()
        };
        let db = ferris_db::memory_db().await.unwrap();
        let engine = DocumentEngine::new(db);
        // Anonymous creation is opted in; the row is owned by `ada` so the
        // owner shortcut is what admits it before the narrowing applies.
        let create_ctx = EngineContext::new("D1Customer", "")
            .with_meta(&meta)
            .with_explicit_name()
            .allow_anonymous(true);
        for (name, title) in [("CUST-A", "a"), ("CUST-B", "b")] {
            engine
                .create_owned_by(
                    &create_ctx,
                    "ada@local",
                    ferris_core::value!({ "name": name, "title": title }),
                )
                .await
                .unwrap();
        }

        let mut user = ferris_core::User::system_user("ada@local".to_string(), "Ada".to_string());
        user.id = "ada@local".to_string();
        user.roles = vec!["D1 Reader".to_string()];
        let ctx = EngineContext::new("D1Customer", "")
            .with_meta(&meta)
            .with_user(&user);

        // No rule: the previous behaviour, both rows.
        {
            let _installed = install(Vec::new());
            let (documents, total) = engine
                .list(&ctx, &crate::ListQuery::default())
                .await
                .unwrap();
            assert_eq!(total, 2);
            assert_eq!(documents.len(), 2);
            engine.get(&ctx, "CUST-B").await.unwrap();
        }

        // Restricted to CUST-A: one row, and the other name is a denial.
        let _installed = install(vec![UserPermissionRule::new(
            "ada@local",
            "D1Customer",
            "CUST-A",
        )]);
        let (documents, total) = engine
            .list(&ctx, &crate::ListQuery::default())
            .await
            .unwrap();
        assert_eq!(total, 1, "the total must count only readable rows");
        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].get("name").and_then(Value::as_str),
            Some("CUST-A")
        );
        engine.get(&ctx, "CUST-A").await.unwrap();
        let error = engine
            .get(&ctx, "CUST-B")
            .await
            .expect_err("CUST-B must be denied");
        assert!(
            matches!(
                error,
                Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
            ),
            "{error:?}"
        );
    }
}
