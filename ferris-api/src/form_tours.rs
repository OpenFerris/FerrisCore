//! `GET /api/doctype/form-tours/{doctype}` — the guided tours of a form.
//!
//! Frappe stores a tour as a `Form Tour` document with `Form Tour Step` rows;
//! the Desk reads them and renders the overlay. Keeping the read here, next to
//! the Actions/Links readers, means a tour is ordinary site metadata: a missing
//! `Form Tour` DocType is an empty list, never a boot failure.
//!
//! Reference: <https://docs.frappe.io/framework/user/en/form-tours>.

// `ferris_core::Error` is the shared taxonomy; boxing it would make every
// caller match on a Box it never inspects.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use ferris_core::Value;

use crate::router::{ApiState, error_response};
use crate::store::DocStore;
use crate::types::ListParams;

/// The `DocType` that stores tours.
const TOUR_DOCTYPE: &str = "Form Tour";
/// Hard cap: a form with hundreds of tours is a metadata mistake, and the Desk
/// only ever offers the first one for a doctype.
const MAX_TOURS: u32 = 50;

/// `GET /api/doctype/form-tours/{doctype}` — the tours declared for `doctype`,
/// each with its `steps` array, in the stored order.
pub async fn list(State(state): State<Arc<ApiState>>, Path(doctype): Path<String>) -> Response {
    match tours(&state.pool, &doctype).await {
        Ok(tours) => Json(serde_json::json!({ "tours": tours })).into_response(),
        Err(e) => error_response(e),
    }
}

/// The tours declared for `doctype`, as the endpoint's `tours` array.
///
/// Shared by the REST handler and the SSR bridge so both render the same list.
/// A database that never created the metadata DocType has no tours: absent
/// table is empty, never an error.
///
/// # Errors
///
/// A failed list read (authorization, database or serialization).
pub async fn tours(pool: &ferris_db::Db, doctype: &str) -> ferris_core::Result<Vec<Value>> {
    if ferris_schema::get_doctype(TOUR_DOCTYPE).is_none() {
        return Ok(Vec::new());
    }
    let params = ListParams {
        filters: Some(HashMap::from([(
            "reference_doctype".to_string(),
            Value::from(doctype),
        )])),
        limit: Some(MAX_TOURS),
        ..ListParams::default()
    };
    let page = DocStore::list(pool, TOUR_DOCTYPE, &params).await?;
    Ok(page.data)
}
