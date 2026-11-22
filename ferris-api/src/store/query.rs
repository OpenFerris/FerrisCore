use ferris_core::Result;
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery, parse_list_filters};

use crate::types::{ListParams, Order};

use super::effective_meta;
use super::meta::context;

/// Widen and normalise the legacy `order_by` expression (R-8).
///
/// The engine accepts any document field, so this only translates the
/// historical REST spellings into the real columns (`created_at` →
/// `creation`, `updated_at` → `modified`) and folds the separate `order`
/// parameter into the expression when the caller did not write a direction.
/// Returns `None` for an empty expression so the engine keeps its default
/// (`modified desc`).
pub(super) fn normalize_order_by(params: &ListParams) -> Option<String> {
    let raw = params.order_by.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let mut parts = raw.split_whitespace();
    let field = parts.next()?;
    let field = match field.to_ascii_lowercase().as_str() {
        "created_at" => "creation",
        "updated_at" => "modified",
        _ => field,
    };
    let direction = parts.next().map(str::to_string).or_else(|| {
        params.order.map(|order| match order {
            Order::Asc => "asc".to_string(),
            Order::Desc => "desc".to_string(),
        })
    });
    Some(match direction {
        Some(direction) => format!("{field} {direction}"),
        None => field.to_string(),
    })
}

/// The single neighbour of `name` in one direction: the first document whose
/// `name` compares `op` ("lt" for the one before, "gt" for the one after) when
/// listed in `order`.
///
/// Resolved through the engine's list path so permissions filter it; only the
/// docname is kept, because that is all a previous/next button needs.
pub(super) async fn neighbor(
    pool: &Db,
    doctype: &str,
    name: &str,
    op: &str,
    order: &str,
) -> Result<Option<String>> {
    let meta = effective_meta(pool, doctype).await?;
    let filters = serde_json::json!([["name", op, name]]).to_string();
    let query = ListQuery {
        filter_ops: parse_list_filters(&filters),
        order_by: Some(order.to_string()),
        limit: Some(1),
        offset: Some(0),
        ..Default::default()
    };
    let (rows, _total) = DocumentEngine::new(pool.clone())
        .list(&context(doctype, "", meta.as_ref()), &query)
        .await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("name"))
        .and_then(ferris_core::Value::as_str)
        .map(str::to_string))
}
