use ferris_core::{Result, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, LikeStore, ListQuery, parse_list_filters};
use ferris_schema::contract::Neighbors;

use crate::types::{ListParams, ListRequest, ListResponse};

use super::meta::context;
use super::query::{neighbor, normalize_order_by};
use super::{effective_meta, ensure_readable, validate_doctype, validate_name};

/// `DocTypes` whose rows change the *effective* metadata.
///
/// `Custom Field` and `Property Setter` normally go through
/// `CustomizationStore`, which invalidates the cache itself. The Desk can also
/// write them through this generic path, and `Custom DocPerm` only exists here,
/// so a successful write must drop the metadata cache rather than leave a
/// merge that no longer matches the rows (PERF-03).
const META_DOCTYPES: &[&str] = &["Custom Field", "Property Setter", "Custom DocPerm"];

/// Drop the cached effective metadata after a write to [`META_DOCTYPES`].
///
/// The cache is keyed per `DocType`, but a `Custom DocPerm` body carries its
/// parent only inside the document, and update/delete do not always have it;
/// invalidating everything is rare and cheap next to serving a stale merge.
fn invalidate_meta_after_write(doctype: &str) {
    if META_DOCTYPES.contains(&doctype) {
        ferris_schema::CustomizationStore::invalidate_all_meta();
    }
}

/// Generic document store.
pub struct DocStore;

impl DocStore {
    /// The documents adjacent to `name` in the doctype's list order, for the
    /// form's previous/next controls.
    ///
    /// Neighbours are resolved through the engine's own list path (the same
    /// `EffectiveMeta` + `ListQuery` the list view uses), so the ordering and —
    /// critically — the `Read` permission filter are the engine's, not a second
    /// policy restated here. A document the caller cannot read is not a
    /// neighbour, so prev/next can never link to one.
    ///
    /// # Errors
    ///
    /// Returns a validation error for a malformed doctype/name, the engine's
    /// authorization error when the caller cannot read the document the
    /// neighbours belong to, and its metadata/storage error while listing.
    pub async fn neighbors(pool: &Db, doctype: &str, name: &str) -> Result<Neighbors> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        // The neighbours belong to a document the caller is looking at, so the
        // same `Read` gate as `GET /api/resource/{doctype}/{name}` applies: a
        // caller who cannot read the document does not get to enumerate what is
        // next to it either.
        ensure_readable(pool, doctype, name).await?;
        let prev = neighbor(pool, doctype, name, "lt", "name desc").await?;
        let next = neighbor(pool, doctype, name, "gt", "name asc").await?;
        Ok(Neighbors { prev, next })
    }

    /// Kept for API compatibility; tables are now created on demand.
    ///
    /// Deliberately *not* `async`: it has no `.await` and the
    /// `unused_async` lint fired on every build. The signature change is
    /// source-compatible for a plain call (`DocStore::init(&pool)?`); callers
    /// that awaited it drop the `.await`.
    ///
    /// # Errors
    ///
    /// Never returns an error; the `Result` is kept only so existing callers
    /// keep compiling.
    pub const fn init(_pool: &Db) -> Result<()> {
        Ok(())
    }

    pub async fn list(
        pool: &Db,
        doctype: &str,
        params: &ListParams,
    ) -> Result<ListResponse<Value>> {
        validate_doctype(doctype)?;
        // Clamped like `list_requested`: the engine materialises and orders
        // every row before applying `limit`, so an unbounded client-supplied
        // limit is an amplification factor, not a page size (§A24/§C2).
        let limit = params.limit.unwrap_or(20).clamp(1, 1000);
        let offset = params.offset.unwrap_or(0);
        let meta = effective_meta(pool, doctype).await?;

        let mut query = ListQuery {
            fields: params.fields.clone(),
            limit: Some(limit),
            offset: Some(offset),
            order_by: normalize_order_by(params),
            search: params.search.clone(),
            ..Default::default()
        };
        if let Some(filters) = &params.filters {
            query.filters = filters.clone();
        }

        // The engine now filters by `Permission::Read` against the caller's
        // identity and reports `total` over what survived, so this route no
        // longer hands out every document in the table.
        let (data, total) = DocumentEngine::new(pool.clone())
            .list(&context(doctype, "", meta.as_ref()), &query)
            .await?;

        Ok(ListResponse {
            data,
            total,
            limit,
            offset,
            page: (offset as u64) / (limit.max(1) as u64) + 1,
            page_size: limit,
        })
    }

    /// Desk list with structural filters, server-side ordering and search.
    ///
    /// This is the endpoint the hydrated frontend uses; it evaluates the
    /// operator filters (`like`, `in`, ranges, AND/OR groups) and the
    /// `order_by` expression in the engine so pagination is authoritative.
    ///
    /// `user` is the caller's identity, needed only to resolve
    /// [`ListRequest::only_liked`]: a like lives in `tab_like` and not in a
    /// document field, so the predicate is turned into a set of docnames here
    /// and the engine does the actual filtering. Without a user the flag is
    /// ignored rather than treated as "match nothing", so an unidentified caller
    /// sees the unfiltered list instead of an empty one.
    pub async fn list_requested(
        pool: &Db,
        doctype: &str,
        req: &ListRequest,
        user: Option<&str>,
    ) -> Result<ListResponse<Value>> {
        validate_doctype(doctype)?;
        let limit = req.limit_page_length.clamp(1, 1000);
        // `limit_start` is a `u64` on the wire, but the engine and the response
        // both address rows with a `u32`; an out-of-range value saturates
        // instead of truncating to a wrong page.
        let offset = u32::try_from(req.limit_start).unwrap_or(u32::MAX);
        let meta = effective_meta(pool, doctype).await?;

        let only_names = match (req.only_liked, user) {
            (Some(true), Some(user)) => Some(
                LikeStore::new(pool.clone())
                    .liked_names(doctype, user)
                    .await?,
            ),
            _ => None,
        };

        let query = ListQuery {
            filter_ops: req
                .filters
                .as_deref()
                .map(parse_list_filters)
                .unwrap_or_default(),
            fields: req.fields.clone(),
            limit: Some(limit as u32),
            offset: Some(offset),
            order_by: req.order_by.clone(),
            search: req.search.clone(),
            only_names,
            ..Default::default()
        };

        let (data, total) = DocumentEngine::new(pool.clone())
            .list(&context(doctype, "", meta.as_ref()), &query)
            .await?;

        Ok(ListResponse {
            data,
            total,
            limit: limit as u32,
            offset,
            page: u64::from(offset) / limit.max(1) + 1,
            page_size: limit as u32,
        })
    }

    /// Read one document, authorized as the ambient caller.
    ///
    /// This is the single-document IDOR route: it used to read the row with no
    /// context at all, so `GET /api/resource/<DocType>/<name>` returned any
    /// document in the system to any authenticated caller. It now resolves the
    /// caller's identity the same way the write routes do and the engine
    /// enforces `Permission::Read`.
    ///
    /// A caller without `Read` gets `PermissionDenied`, not `404`. That mirrors
    /// Frappe, and it does confirm the row exists — an existence oracle worth
    /// weighing against Frappe's own behaviour in a follow-up.
    pub async fn get(pool: &Db, doctype: &str, name: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        let meta = effective_meta(pool, doctype).await?;
        DocumentEngine::new(pool.clone())
            .get(&context(doctype, "", meta.as_ref()), name)
            .await
    }

    pub async fn create(pool: &Db, doctype: &str, data: Value) -> Result<Value> {
        validate_doctype(doctype)?;
        // M7: unlike update/delete, `create` takes the name from the request
        // body, so the client-chosen primary key goes through the same gates as
        // a route parameter — a malicious body could otherwise mint `/../`,
        // control-char or over-long names that no handler would ever produce.
        if let Some(name) = data.get("name").and_then(|v| v.as_str()) {
            if !name.is_empty() {
                validate_name(name)?;
            }
        }
        let meta = effective_meta(pool, doctype).await?;
        let naming_series = meta
            .as_ref()
            .map(|m| m.naming_series.clone())
            .unwrap_or_default();
        let value = DocumentEngine::new(pool.clone())
            .create(&context(doctype, &naming_series, meta.as_ref()), data)
            .await?;
        invalidate_meta_after_write(doctype);
        if let Some(name) = value.get("name").and_then(Value::as_str) {
            crate::notify::notify_doc_insert(doctype, name, &value);
        }
        Ok(value)
    }

    pub async fn update(pool: &Db, doctype: &str, name: &str, data: Value) -> Result<Value> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        let meta = effective_meta(pool, doctype).await?;
        let value = DocumentEngine::new(pool.clone())
            .update(&context(doctype, "", meta.as_ref()), name, data)
            .await?;
        invalidate_meta_after_write(doctype);
        // The changed-field diff is not available here; the emitted event still
        // carries the full new document so subscribers can re-render.
        crate::notify::notify_doc_update(doctype, name, &value, &Value::Null, &Value::Null);
        Ok(value)
    }

    pub async fn delete(pool: &Db, doctype: &str, name: &str) -> Result<()> {
        validate_doctype(doctype)?;
        validate_name(name)?;
        let meta = effective_meta(pool, doctype).await?;
        DocumentEngine::new(pool.clone())
            .delete(&context(doctype, "", meta.as_ref()), name)
            .await?;
        invalidate_meta_after_write(doctype);
        // The document is gone, so its own activity feed is no longer readable
        // (the endpoint gates on `Read`); the engine recorded the tombstone in
        // the delete transaction, so a later re-creation keeps a continuous
        // history.
        // The row is gone, so the previous document is no longer readable; the
        // event still tells subscribers that this name disappeared.
        crate::notify::notify_doc_delete(doctype, name, None);
        Ok(())
    }
}
