//! The document engine's side of the full-text index.
//!
//! [`ferris_schema::search`] builds the per-engine SQL; this module decides
//! *when* it runs and what a failure means. The index is derived data, never the
//! source of truth: every operation here is best effort, and the list path
//! keeps its Rust scan over the reconstructed document as the fallback whenever
//! the index is unavailable.
//!
//! | Concern | Policy |
//! |---|---|
//! | DDL | once per [`ferris_db::Db`] identity, like the side tables; a failure (FTS5 compiled out) is remembered, not retried |
//! | Writes | the caller resolves [`ready`] and passes it down; a failed statement is logged and never fails the document write |
//! | Search | [`names`] returns the matching names; any error sends `list` back to the documented scan |
//!
//! The caller passes readiness explicitly because a [`WriteSink::Tx`] cannot
//! name the database it belongs to, and a failing statement inside a Postgres
//! transaction would abort it. The engine therefore probes once on its `Db` and
//! only lets index statements reach a transaction once the table is known to
//! exist.

use super::support::{DocShape, WriteSink, read_row_with_shape_via};
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::Db;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// How much of a document's text one row may carry.
///
/// A document's text can be arbitrarily large; the index only needs enough to
/// find it. 1 MiB is far above any realistic field set and keeps one
/// pathological document from doubling the database.
const BODY_LIMIT: usize = 1024 * 1024;

/// Whether the index exists, keyed by [`Db::id`].
///
/// `true` means the DDL succeeded; `false` means it was attempted and failed,
/// so every later write skips the index instead of paying for a failing
/// statement. A missing entry means "not probed yet".
static READY: LazyLock<Mutex<HashMap<u64, bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached(db: &Db) -> Option<bool> {
    READY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&db.id())
        .copied()
}

fn remember(db: &Db, ready: bool) {
    READY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(db.id(), ready);
}

/// Whether [`ensure`] has succeeded for this database.
///
/// Cheap and synchronous: the write paths call it once per operation, outside
/// any transaction, and use the answer to decide whether index statements may
/// run on a [`WriteSink`].
pub(in crate::engine) fn ready(db: &Db) -> bool {
    cached(db).unwrap_or(false)
}

/// Whether `doctype` participates in the full-text index.
///
/// The index is created by [`ensure_table`](super::DocumentEngine::ensure_table)
/// only for a `DocType` the registry knows, so that is also the only case where a
/// write may index and a list may trust the index. A `DocType` without metadata
/// (a test or a runtime name) keeps the Rust scan on both sides, which is what
/// makes the two agree: no row of it ever enters the index.
pub(in crate::engine) fn ready_for(db: &Db, doctype: &str) -> bool {
    ready(db) && ferris_schema::get_doctype(doctype).is_some()
}

/// Create the search table and its index, once per database.
///
/// A failure is cached and returned; the engine logs it and keeps the Rust
/// scan. FTS5 is a compile-time module of SQLite, so the interesting failure is
/// a build without it (`no such module: fts5`), which must not stop a boot.
#[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
pub(in crate::engine) async fn ensure(db: &Db) -> Result<()> {
    if let Some(ready) = cached(db) {
        return if ready { Ok(()) } else { Err(unavailable()) };
    }
    for sql in ferris_schema::search::ensure_statements(db.dialect()) {
        if let Err(error) = db.execute_script(&sql).await {
            // Another task may have won the race and created it; keep that
            // answer rather than demoting a working index.
            if cached(db) == Some(true) {
                return Ok(());
            }
            if matches!(
                db.table_exists(ferris_schema::search::TABLE).await,
                Ok(true)
            ) {
                remember(db, true);
                return Ok(());
            }
            remember(db, false);
            return Err(error);
        }
    }
    remember(db, true);
    Ok(())
}

/// Index one written document, replacing whatever that name had before.
///
/// The caller must have checked [`ready`]. The body is every string leaf of the
/// document (child arrays included, because they are part of the document),
/// capped at [`BODY_LIMIT`].
#[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
pub(in crate::engine) async fn index(
    sink: &mut WriteSink<'_>,
    doctype: &str,
    document: &Value,
) -> Result<()> {
    let Some(name) = document.get("name").and_then(Value::as_str) else {
        return Ok(());
    };
    if name.is_empty() {
        return Ok(());
    }
    let body = body_of(document);
    for stmt in ferris_schema::search::upsert(sink.dialect(), doctype, name, &body) {
        sink.execute(&stmt).await?;
    }
    Ok(())
}

/// Drop one name from the index. The caller must have checked [`ready`].
#[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
pub(in crate::engine) async fn remove(
    sink: &mut WriteSink<'_>,
    doctype: &str,
    name: &str,
) -> Result<()> {
    sink.execute(&ferris_schema::search::remove(
        sink.dialect(),
        doctype,
        name,
    ))
    .await?;
    Ok(())
}

/// Rebuild the index for every stored row of `doctype`.
///
/// Used by [`super::DocumentEngine::reindex_search`] (a migration or repair
/// step): rows written before the index existed, or while FTS was unavailable,
/// are otherwise invisible to indexed search. One read + one upsert per row, and
/// the physical shape is resolved once for the whole table instead of per row;
/// it is a migration, not a request path.
pub(in crate::engine) async fn rebuild(db: &Db, doctype: &str) -> Result<usize> {
    ensure(db).await?;
    if ferris_schema::get_doctype(doctype).is_none() {
        return Ok(0);
    }
    let table = super::table_name(doctype)?;
    let quoted = super::q(db.dialect(), &table)?;
    let rows = db
        .fetch_all(&ferris_db::Stmt::raw(format!("SELECT name FROM {quoted}")))
        .await?;
    let shape = DocShape::resolve(db, doctype, db.dialect(), None).await?;
    let mut rebuilt = 0usize;
    for row in &rows {
        let name = row.str("name")?;
        let (document, _) =
            read_row_with_shape_via(&mut WriteSink::Db(db), &shape, doctype, &name).await?;
        index(&mut WriteSink::Db(db), doctype, &document).await?;
        rebuilt += 1;
    }
    Ok(rebuilt)
}

/// The names in `doctype` whose indexed text matches `term`.
///
/// An `Err` — no FTS5, a term the backend rejects — is the signal for `list` to
/// fall back to the Rust scan; it is not a document-level failure.
#[allow(clippy::result_large_err)] // crate-wide `Error`; boxing it would leak into every caller
pub(in crate::engine) async fn names(
    db: &Db,
    doctype: &str,
    term: &str,
) -> Result<HashSet<String>> {
    ensure(db).await?;
    let rows = db
        .fetch_all(&ferris_schema::search::search(db.dialect(), doctype, term))
        .await?;
    let mut names = HashSet::with_capacity(rows.len());
    for row in &rows {
        names.insert(row.str("name")?);
    }
    Ok(names)
}

/// Every string leaf of `value`, one per line, truncated at [`BODY_LIMIT`].
///
/// Newlines keep two adjacent fields from fusing into one token; array items
/// (child rows) are walked like any other value. Non-strings carry no text and
/// are skipped rather than stringified, so a number does not become a token.
fn body_of(value: &Value) -> String {
    let mut body = String::new();
    collect(value, &mut body);
    body
}

fn collect(value: &Value, body: &mut String) {
    if body.len() >= BODY_LIMIT {
        return;
    }
    match value {
        Value::String(text) => {
            let room = BODY_LIMIT.saturating_sub(body.len());
            let take = text.len().min(room);
            let mut end = take;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            body.push_str(&text[..end]);
            body.push('\n');
        }
        Value::Object(map) => {
            for value in map.values() {
                collect(value, body);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect(value, body);
            }
        }
        _ => {}
    }
}

fn unavailable() -> Error {
    Error::Validation(ValidationError::Invalid(
        "the full-text search index is unavailable on this database".to_string(),
    ))
}
