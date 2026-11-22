//! The tag service: parse the standard field, keep the catalogue and the
//! association registry in step with it.
//!
//! Every function here runs against the caller's pool as a **service identity**
//! ([`crate::service`]), never as the ambient request user: the derived rows
//! are a consequence of saving a document, not an action the caller takes, and
//! the `Tag`/`Tag Link` `DocType`s deliberately grant `Create`/`Delete` to
//! `System Manager` only. Running as the caller would either fail the save or
//! force every tagger to hold catalogue rights.

#![allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
#![allow(clippy::doc_markdown)]

use std::collections::{HashMap, HashSet};

use ferris_core::{DocumentError, Error, Result, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, EngineContext, ListQuery};
use ferris_schema::DoctypeMeta;

use super::TAG_LINK;
use crate::doctypes::TAG;

/// One `(document_type, document_name, tag)` association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagLink {
    /// `DocType` of the tagged document.
    pub document_type: String,
    /// Name (primary key) of the tagged document.
    pub document_name: String,
    /// The tag label.
    pub tag: String,
}

/// How many links one document may carry. Generous for a tag list; it caps the
/// read so a pathological document cannot make the reconcile unbounded.
const MAX_LINKS_PER_DOCUMENT: u32 = 1000;

/// The purpose id tag writes carry, so their rows say which subsystem made them.
const SERVICE_ID: &str = "tags@ferris.local";

/// An engine context for `doctype` authored as the service principal.
///
/// `meta` is `None` when the `DocType` is not registered; the engine then skips
/// metadata-driven validation, which is the right escape hatch for a catalog
/// that has not loaded yet.
fn context<'a>(doctype: &str, meta: Option<&'a DoctypeMeta>) -> EngineContext<'a> {
    crate::service::service_context(doctype, meta, SERVICE_ID, "Tags")
}

/// Split the standard `tags` field into a normalised, de-duplicated label list.
///
/// Two shapes are accepted because the field's storage can evolve without this
/// parser changing:
///
/// * the comma-separated `Data` string the Desk writes today
///   (`"urgent, billing"`), and
/// * the list of row objects a `Table MultiSelect` would produce
///   (`[{"tag": "urgent"}, …]`).
///
/// Labels are trimmed, internal whitespace is collapsed to single spaces, and
/// duplicates (same label twice) keep their first position. Tags are
/// case-sensitive, so `"Urgent"` and `"urgent"` stay distinct.
#[must_use]
pub fn parse_tags(tags: Option<&Value>) -> Vec<String> {
    let raw: Vec<String> = match tags {
        Some(Value::String(s)) => s.split(',').map(str::to_string).collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.clone()),
                Value::Object(row) => row.get("tag").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for label in raw {
        let normalised = label.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalised.is_empty() {
            continue;
        }
        if seen.insert(normalised.clone()) {
            out.push(normalised);
        }
    }
    out
}

/// Create the `Tag` catalogue row for `label` when it is missing.
///
/// The catalogue is keyed by the label itself (document name == label), so a
/// `Tag Link.tag` is the human-readable tag, matching Frappe. An existing tag is
/// the common case; the row is looked up before writing, and a concurrent create
/// that loses the race is treated as success.
async fn ensure_tag(engine: &DocumentEngine, label: &str) -> Result<()> {
    let meta = ferris_schema::get_doctype(TAG);
    let ctx = context(TAG, meta.as_deref());
    match engine.get(&ctx, label).await {
        // Present: the common case, so there is nothing to write.
        Ok(_) => return Ok(()),
        // Absent: fall through to the create below.
        Err(Error::Document(DocumentError::NotFound { .. })) => {}
        // Anything else is a real failure (pool, constraint, permission rule)
        // and must not be papered over as "the tag does not exist yet".
        Err(error) => return Err(error),
    }
    let data = Value::from_json(serde_json::json!({ "name": label, "tag": label }));
    match engine.create(&ctx, data).await {
        Ok(_) | Err(Error::Document(DocumentError::AlreadyExists { .. })) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Make the `Tag Link` rows for `(doctype, name)` match `labels` exactly.
///
/// This is the reconcile step shared by save and delete (delete passes an empty
/// list): existing links for the document are read once, links whose tag is no
/// longer wanted are removed, and the missing ones are added. Because the whole
/// set is read before any write, re-running it is a no-op.
async fn reconcile_links(db: &Db, doctype: &str, name: &str, labels: &[String]) -> Result<()> {
    let meta = ferris_schema::get_doctype(TAG_LINK);
    let ctx = context(TAG_LINK, meta.as_deref());
    let engine = DocumentEngine::new(db.clone());

    let mut filters = HashMap::new();
    filters.insert("document_type".to_string(), Value::string(doctype));
    filters.insert("document_name".to_string(), Value::string(name));
    let query = ListQuery {
        filters,
        fields: Some(vec!["name".to_string(), "tag".to_string()]),
        limit: Some(MAX_LINKS_PER_DOCUMENT),
        ..Default::default()
    };
    let (existing, _total) = engine.list(&ctx, &query).await?;

    // label -> the link row's own name, so a stale link can be deleted.
    let mut linked: HashMap<String, String> = HashMap::new();
    for row in &existing {
        if let (Some(tag), Some(link)) = (
            row.get("tag").and_then(Value::as_str),
            row.get("name").and_then(Value::as_str),
        ) {
            linked.insert(tag.to_string(), link.to_string());
        }
    }

    let wanted: HashSet<&str> = labels.iter().map(String::as_str).collect();
    for (label, link) in &linked {
        if !wanted.contains(label.as_str()) {
            engine.delete(&ctx, link).await?;
        }
    }

    for label in labels {
        if linked.contains_key(label) {
            continue;
        }
        let data = Value::from_json(serde_json::json!({
            "document_type": doctype,
            "document_name": name,
            "tag": label,
        }));
        engine.create(&ctx, data).await?;
    }
    Ok(())
}

/// Bring the catalogue and the `Tag Link` registry in step with `tags`, the
/// value of a saved document's standard `tags` field.
///
/// Safe to call for the `Tag`/`Tag Link` `DocType`s themselves (it returns
/// early) and safe to re-run.
///
/// # Errors
///
/// Returns the storage error of any catalogue/link write. Callers that run this
/// from a document hook should treat it as best-effort and log, so a tag problem
/// never fails the save that triggered it.
pub async fn sync_document(db: &Db, doctype: &str, name: &str, tags: Option<&Value>) -> Result<()> {
    if doctype == TAG || doctype == TAG_LINK {
        return Ok(());
    }
    let labels = parse_tags(tags);
    let engine = DocumentEngine::new(db.clone());
    for label in &labels {
        ensure_tag(&engine, label).await?;
    }
    reconcile_links(db, doctype, name, &labels).await
}

/// Drop every `Tag Link` row belonging to a deleted document.
///
/// # Errors
///
/// Returns the storage error of the reconcile write.
pub async fn remove_document(db: &Db, doctype: &str, name: &str) -> Result<()> {
    if doctype == TAG || doctype == TAG_LINK {
        return Ok(());
    }
    reconcile_links(db, doctype, name, &[]).await
}

/// Every document associated with `tag`.
///
/// Answers the "documentos por tag" query off the `Tag Link` registry rather
/// than by scanning document blobs, so it stays one table read as the install
/// grows.
///
/// # Errors
///
/// Returns the storage error of the read itself (table creation or the query).
/// The read runs as the service user, so permissions never deny it; an
/// unregistered `Tag Link` only costs the metadata-driven index pushdown
/// (`meta: None` has no rows to reject).
pub async fn documents_with_tag(db: &Db, tag: &str) -> Result<Vec<TagLink>> {
    let meta = ferris_schema::get_doctype(TAG_LINK);
    let ctx = context(TAG_LINK, meta.as_deref());
    let engine = DocumentEngine::new(db.clone());

    let mut filters = HashMap::new();
    filters.insert("tag".to_string(), Value::string(tag));
    let query = ListQuery {
        filters,
        fields: Some(vec![
            "document_type".to_string(),
            "document_name".to_string(),
            "tag".to_string(),
        ]),
        limit: Some(MAX_LINKS_PER_DOCUMENT),
        ..Default::default()
    };
    let (rows, _total) = engine.list(&ctx, &query).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        if let (Some(document_type), Some(document_name), Some(tag)) = (
            row.get("document_type").and_then(Value::as_str),
            row.get("document_name").and_then(Value::as_str),
            row.get("tag").and_then(Value::as_str),
        ) {
            out.push(TagLink {
                document_type: document_type.to_string(),
                document_name: document_name.to_string(),
                tag: tag.to_string(),
            });
        }
    }
    Ok(out)
}
