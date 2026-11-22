//! `GET /api/search` — bounded, permission-aware global search.

use axum::{
    Json,
    extract::{Query, State},
    response::{IntoResponse, Response},
};
use ferris_core::{Error, Value};
use serde::Deserialize;

use crate::ApiState;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
}

const SEARCH_PER_DOCTYPE: u32 = 4;
pub(super) const SEARCH_MAX_TOTAL: usize = 40;

/// Hard ceiling on `Global Search Settings.max_results`.
///
/// The stored value is operator input, and one keystroke must not be able to
/// turn a global search into an unbounded response. A configured value above
/// this is clamped, not trusted.
pub(super) const SEARCH_MAX_TOTAL_CAP: usize = 200;

/// Hard ceiling on the number of DocTypes one query walks.
///
/// The search term is never pushed into SQL: `DocumentEngine::list` filters the
/// term in Rust, so every candidate DocType costs a table read (bounded by
/// [`SEARCH_PER_DOCTYPE`] only after the page is materialised). An install with
/// hundreds of searchable DocTypes must not pay for all of them per keystroke.
const SEARCH_MAX_DOCTYPES: usize = 32;

/// `GET /api/search?q=...` — case-insensitive search across the DocTypes that
/// opt in with `in_global_search`, returning name/title/route hits grouped by
/// the client.
///
/// This is a **bounded SQL scan, not an index**: the term is matched by the
/// document engine's residual filter (`ferris-document`'s list plan keeps a
/// substring search in Rust because it walks every string leaf), and the
/// response is capped by `Global Search Settings.max_results`,
/// [`SEARCH_MAX_TOTAL_CAP`] and [`SEARCH_MAX_DOCTYPES`]. The framework does not
/// maintain a search index, so a hit set is always consistent with the live
/// documents and honours `Permission::Read`.
///
/// Each DocType is read as the ambient caller, with its metadata attached to the
/// context so field-level redaction applies to anything the engine returns; a
/// DocType the caller cannot read is skipped silently (indistinguishable from
/// "no match", which leaks nothing), while any other engine failure aborts the
/// request instead of pretending there were no results.
pub async fn search(
    State(state): State<std::sync::Arc<ApiState>>,
    Query(qs): Query<SearchQuery>,
) -> Response {
    let q = qs.q.trim().to_string();
    if q.is_empty() {
        return Json(Vec::<Value>::new()).into_response();
    }

    // `Global Search Settings` (Build) is the install-wide switch for this
    // route: `enabled` turns the whole search off and `max_results` caps the
    // total hits returned. Read with the read check bypassed — it is the same
    // setting for everyone, so a caller without `Read` on it must still feel
    // its effect. A missing row simply means "use the compiled defaults".
    // A search is a GET: never materialise the settings row here.
    let settings =
        crate::store::DocStore::read_single_system(&state.pool, "Global Search Settings")
            .await
            .ok();
    if settings
        .as_ref()
        .and_then(|s| s.get("enabled"))
        .and_then(Value::as_bool)
        == Some(false)
    {
        return Json(Vec::<Value>::new()).into_response();
    }
    let max_total = resolve_max_total(settings.as_ref());

    let mut out: Vec<Value> = Vec::new();
    let mut scanned = 0usize;
    for name in ferris_schema::list_doctypes() {
        if out.len() >= max_total || scanned >= SEARCH_MAX_DOCTYPES {
            break;
        }
        let Some(meta) = ferris_schema::get_doctype(&name) else {
            continue;
        };
        if meta.hidden || meta.is_child_table || meta.is_virtual || meta.is_single {
            continue;
        }
        if !meta.in_global_search {
            continue;
        }
        scanned += 1;
        let params = ferris_document::ListQuery {
            search: Some(q.clone()),
            limit: Some(SEARCH_PER_DOCTYPE),
            ..Default::default()
        };
        // Search reads as the ambient caller so the engine's `Permission::Read`
        // check applies (§C2). This route iterates *every* searchable DocType,
        // so without it a single query returned matches from the whole
        // installation to anyone authenticated. A DocType the caller cannot
        // read simply yields no hits, which is indistinguishable from "no
        // match" and leaks nothing.
        //
        // The identity comes from the ambient task user, which
        // `bind_ambient_user` scopes on this router. It is absent only if the
        // route is reached without that layer, and then every row is refused —
        // fail closed, which is the right way round for a search endpoint.
        let mut read_ctx = ferris_document::EngineContext::new(name.clone(), "");
        // Attach the DocType metadata so `redact_read` masks `Password`/`Secret`
        // fields (and drops fields the caller may not read) on everything the
        // engine hands back. The hit response only exposes name/title, but the
        // engine's intermediate documents must still be redacted.
        read_ctx.meta = Some(meta.as_ref());
        read_ctx.user = ferris_document::current_user();
        let docs = match ferris_document::DocumentEngine::new(state.pool.clone())
            .list(&read_ctx, &params)
            .await
        {
            Ok((docs, _)) => docs,
            // A denied DocType is "no match"; leaking that it exists is worse.
            Err(Error::Authz(_)) => continue,
            // A storage/validation failure is not "no results": report it.
            Err(e) => return crate::router::error_response(e),
        };
        for doc in docs {
            if out.len() >= max_total {
                break;
            }
            let rec_name = doc
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = search_title(&meta, &doc, &rec_name);
            let route = format!("/desk/{}/{}/{}", meta.module, name, rec_name);
            out.push(Value::from(serde_json::json!({
                "doctype": name.clone(),
                "name": rec_name,
                "title": title,
                "module": meta.module.clone(),
                "route": route,
            })));
        }
    }
    Json(out).into_response()
}

fn search_title(meta: &ferris_schema::DoctypeMeta, doc: &Value, name: &str) -> String {
    if let Some(field) = &meta.title_field {
        if let Some(v) = doc.get(field).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    for field in &meta.search_fields {
        if let Some(v) = doc.get(field).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    name.to_string()
}

/// Apply the stored cap and the hard ceiling to `Global Search Settings`.
pub(super) fn resolve_max_total(settings: Option<&Value>) -> usize {
    settings
        .and_then(|s| s.get("max_results"))
        .and_then(Value::as_i64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0)
        .unwrap_or(SEARCH_MAX_TOTAL)
        .min(SEARCH_MAX_TOTAL_CAP)
}
