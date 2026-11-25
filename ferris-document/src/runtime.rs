//! In-transaction change-history recording for the document engine.
//!
//! Versioning, sharing and the workflow state machine shipped as complete
//! modules with **no production caller**: the
//! engine wrote documents without ever recording a version, the permission check
//! never consulted `tab_share`, and a declared workflow never gated a transition.
//! `tab_version` was even cascaded on delete (`engine/write.rs`), so a document
//! deleted and recreated started its history again from nothing.
//!
//! The engine now owns both behaviours with **no opt-in flag**: every
//! `DocumentEngine::new` records history and consults shares, and the side
//! tables are created lazily once per database. `VersionManager` remains the
//! **reader** (and an import tool) — no request path writes versions through it,
//! so there is exactly one writer and a write cannot produce two rows.
//!
//! The workflow runtime needs no switch: it is data-driven and reachable from
//! anywhere through [`crate::workflow::global_workflow_engine`], so
//! [`crate::engine::DocumentEngine::set_docstatus`] reads it directly.
//!
//! Recording lives here, and takes a [`WriteSink`], because it must run *inside*
//! the caller's transaction — the same requirement the naming series had (DOC1).
//! A version row that committed separately from the document it describes could
//! claim a state the document never reached, and a rolled-back batch would leave
//! phantom history behind.
//!
//! Every row is written by [`write_version`], the single writer, which links it
//! to its predecessor with a SHA-256 `hash`/`prev_hash` chain; the reader side is
//! [`crate::versioning::VersionManager::verify_chain`]. The chain makes a
//! modified or removed middle row evident, but it is not WORM: the table still
//! accepts `UPDATE`/`DELETE`, `tab_version` is still cascaded when a document is
//! deleted (`engine/write.rs`), and there is no anchor outside the database.
//! Retention and signing are a later change; until then this is the
//! cryptographic base, not an audit guarantee.

// The module is private, so `pub(crate)` items here are unreachable from outside
// the crate anyway; spelling them `pub` would suggest an external API this
// module deliberately does not have.
#![allow(clippy::redundant_pub_crate)]

use chrono::{DateTime, Utc};
use ferris_core::{Error, Result, SerializationError, ValidationError, Value};
use ferris_db::Stmt;
use std::collections::HashMap;

use crate::engine::support::WriteSink;
use crate::versioning::{ChangeType, DocumentVersion, TAB_VERSION, change_type_json};

/// The RFC 3339 timestamp the side tables store.
///
/// One spelling for `created_at`/`changed_at` across the stores and the engine,
/// so a row any of them writes reads back the same way.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Ensure `tab_version` exists, creating it lazily like any doctype table.
///
/// Called *before* the transaction opens, so the in-transaction
/// [`record_version_in_tx`] runs no DDL — which matters because the engine may
/// hold the write on an open transaction that must stay DDL-free (DOC1). The
/// schema step also adds the hash-chain columns to a table created before them.
pub(crate) async fn ensure_version_table(db: &ferris_db::Db) -> Result<()> {
    crate::versioning::ensure_version_schema(db).await
}

/// Record one version row for `(doctype, docname)` inside `sink`.
///
/// The row is written by [`write_version_row`], the single writer used by both
/// the engine and the version manager, so numbering and the `hash`/`prev_hash`
/// chain are computed once. `changed_at` is taken here, on the same connection,
/// together with the row it describes.
///
/// The engine path borrows the document: the snapshot is serialized straight
/// from it, so a save does not deep-clone every field and child array just to
/// hand the map to a row writer that hashes it and drops it.
pub(crate) async fn record_version_in_tx(
    sink: &mut WriteSink<'_>,
    doctype: &str,
    docname: &str,
    data: &Value,
    changed_by: &str,
    change_type: ChangeType,
) -> Result<()> {
    let Some(fields) = data.as_object() else {
        // A version row describes a document, which is always an object. Falling
        // back to an empty snapshot would record history that cannot be trusted.
        return Err(Error::Validation(ValidationError::Invalid(
            "document version data must be an object".to_string(),
        )));
    };
    write_version_row(
        sink,
        doctype,
        docname,
        fields,
        &VersionMeta {
            changed_by,
            changed_at: Utc::now(),
            change_type,
            comment: None,
        },
    )
    .await?;
    Ok(())
}

/// The version-row fields that are not the document snapshot.
///
/// Bundled so [`write_version`] keeps a readable arity: the snapshot and its
/// chain link are positional, the metadata travels together.
pub(crate) struct VersionMeta<'a> {
    pub(crate) changed_by: &'a str,
    pub(crate) changed_at: DateTime<Utc>,
    pub(crate) change_type: ChangeType,
    pub(crate) comment: Option<String>,
}

/// A version row that was written: its number and its position in the chain.
///
/// The snapshot itself is not echoed back. The engine records history on every
/// save and never reads it back from here, so returning a [`DocumentVersion`]
/// would clone the document it just serialized.
pub(crate) struct VersionStamp {
    pub(crate) version: u32,
    pub(crate) hash: String,
    pub(crate) prev_hash: String,
}

/// Insert one `tab_version` row on `sink`'s connection, chained to its
/// predecessor.
///
/// The only writer of the table: [`record_version_in_tx`] calls it inside the
/// document's transaction and [`write_version`] wraps it for
/// [`crate::versioning::VersionManager`]. One query reads the tip of the chain
/// (number and hash together — the table's primary key is
/// `(doctype, docname, version)`, so it is an index lookup), and the hash covers
/// exactly the text stored in `data` plus the previous row's hash.
#[allow(clippy::result_large_err)] // the crate-wide `Error` type is intentionally unchanged
pub(crate) async fn write_version_row(
    sink: &mut WriteSink<'_>,
    doctype: &str,
    docname: &str,
    data: &HashMap<String, Value>,
    meta: &VersionMeta<'_>,
) -> Result<VersionStamp> {
    let tip = sink
        .fetch_optional(&Stmt::new(
            format!(
                "SELECT version, hash FROM {TAB_VERSION} \
                 WHERE doctype = ? AND docname = ? ORDER BY version DESC LIMIT 1"
            ),
            vec![Value::from(doctype), Value::from(docname)],
        ))
        .await?;
    let (version_num, prev_hash) = match &tip {
        Some(row) => (
            // DOC15: saturate instead of wrapping a huge counter value.
            row.i64("version")?.saturating_add(1).max(1),
            row.opt_str("hash").unwrap_or_default(),
        ),
        // The first version anchors the chain; it has nothing to point at.
        None => (1, String::new()),
    };
    let version = u32::try_from(version_num).unwrap_or(u32::MAX);
    let data_json = canonical_data(data)?;
    let hash = version_hash_parts(
        &prev_hash,
        doctype,
        docname,
        version,
        &data_json,
        meta.changed_by,
        meta.changed_at,
        meta.change_type,
    );

    sink.execute(&Stmt::new(
        format!(
            "INSERT INTO {TAB_VERSION} \
             (doctype, docname, version, data, changed_by, changed_at, change_type, comment, \
              hash, prev_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ),
        vec![
            Value::from(doctype),
            Value::from(docname),
            Value::from(version_num),
            // The exact text the hash covers, stored as the JSON blob.
            Value::from(data_json.as_str()),
            Value::from(meta.changed_by),
            Value::from(meta.changed_at.to_rfc3339()),
            Value::from(change_type_json(meta.change_type)),
            meta.comment.as_deref().map_or(Value::Null, Value::from),
            Value::from(hash.as_str()),
            Value::from(prev_hash.as_str()),
        ],
    ))
    .await?;
    Ok(VersionStamp {
        version,
        hash,
        prev_hash,
    })
}

/// [`write_version_row`] for a caller that wants the full [`DocumentVersion`]
/// back. The manager runs outside the engine's hot path, so it owns the map and
/// can take it back after the row is written.
pub(crate) async fn write_version(
    sink: &mut WriteSink<'_>,
    doctype: &str,
    docname: &str,
    data: HashMap<String, Value>,
    meta: VersionMeta<'_>,
) -> Result<DocumentVersion> {
    let stamp = write_version_row(sink, doctype, docname, &data, &meta).await?;
    Ok(DocumentVersion {
        version: stamp.version,
        doctype: doctype.to_string(),
        docname: docname.to_string(),
        data,
        changed_by: meta.changed_by.to_string(),
        changed_at: meta.changed_at,
        change_type: meta.change_type,
        comment: meta.comment,
        hash: stamp.hash,
        prev_hash: stamp.prev_hash,
    })
}

/// The canonical JSON text a version stores and hashes.
///
/// [`Value::Object`] is a `HashMap`, so serializing it directly writes keys in
/// iteration order, which differs between processes and would make the hash
/// unverifiable after a read-back. `serde_json::to_value` goes through a map with
/// sorted keys, the same canonical form the engine writes documents in
/// (`engine::support::canonical_blob`).
#[allow(clippy::result_large_err)] // the crate-wide `Error` type is intentionally unchanged
pub(crate) fn canonical_data(data: &HashMap<String, Value>) -> Result<String> {
    let canonical = serde_json::to_value(Value::Object(data.clone()))
        .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
    serde_json::to_string(&canonical).map_err(|e| Error::Serialization(SerializationError::Json(e)))
}

/// SHA-256 over the previous hash and every persisted column of `version`.
///
/// Field order is fixed and the concatenation has no separator; the caller
/// always supplies the same fields, and `version`, `data` and `changed_at` are
/// the stored spellings, so a read-back recomputes the identical input.
pub(crate) fn version_hash(version: &DocumentVersion, data_json: &str) -> String {
    version_hash_parts(
        &version.prev_hash,
        &version.doctype,
        &version.docname,
        version.version,
        data_json,
        &version.changed_by,
        version.changed_at,
        version.change_type,
    )
}

/// [`version_hash`] over the parts, for a writer that hashes a row before it
/// builds a [`DocumentVersion`].
fn version_hash_parts(
    prev_hash: &str,
    doctype: &str,
    docname: &str,
    version: u32,
    data_json: &str,
    changed_by: &str,
    changed_at: DateTime<Utc>,
    change_type: ChangeType,
) -> String {
    ferris_db::exec::sha256_hex(&format!(
        "{prev_hash}{doctype}{docname}{version}{data_json}{changed_by}{}{}",
        changed_at.to_rfc3339(),
        change_type_json(change_type),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioning::{VersionChainError, VersionManager};

    async fn db() -> ferris_db::Db {
        ferris_db::memory_db().await.unwrap()
    }

    /// Build a history of `count` rows through the manager, the reader-side
    /// writer, so the chain covers the same `write_version` the engine uses.
    async fn history(manager: &VersionManager, count: usize) {
        for index in 0..count {
            let mut data = HashMap::new();
            data.insert("step".to_string(), Value::from(index));
            manager
                .create_version(
                    "Test",
                    "doc-1",
                    data,
                    "user1".to_string(),
                    ChangeType::Update,
                    None,
                )
                .await
                .unwrap();
        }
    }

    /// Three rows link into a chain that verifies, and each row after the first
    /// anchors on its predecessor's hash.
    #[tokio::test]
    async fn verify_chain_accepts_an_intact_history() {
        let manager = VersionManager::with_db(db().await);
        manager.init().await.unwrap();
        history(&manager, 3).await;

        assert_eq!(manager.verify_chain("Test", "doc-1").await.unwrap(), 3);

        let versions = manager.get_versions("Test", "doc-1").await.unwrap();
        assert!(versions[0].prev_hash.is_empty());
        assert_eq!(versions[0].hash.len(), 64);
        assert_eq!(versions[1].prev_hash, versions[0].hash);
        assert_eq!(versions[2].prev_hash, versions[1].hash);
    }

    /// The in-memory fallback hashes the same fields, so a restore or an import
    /// held in memory verifies too.
    #[tokio::test]
    async fn verify_chain_covers_the_in_memory_fallback() {
        let manager = VersionManager::new();
        history(&manager, 2).await;
        assert_eq!(manager.verify_chain("Test", "doc-1").await.unwrap(), 2);
    }

    /// Changing a stored snapshot breaks its own hash: the proof that the table
    /// is tamper-evident.
    #[tokio::test]
    async fn verify_chain_detects_an_altered_row() {
        let db = db().await;
        let manager = VersionManager::with_db(db.clone());
        manager.init().await.unwrap();
        history(&manager, 3).await;

        db.execute(&Stmt::raw(format!(
            "UPDATE {TAB_VERSION} SET data = '{{\"step\":99}}' \
             WHERE doctype = 'Test' AND docname = 'doc-1' AND version = 2"
        )))
        .await
        .unwrap();

        match manager.verify_chain("Test", "doc-1").await {
            Err(VersionChainError::HashMismatch { version }) => assert_eq!(version, 2),
            other => panic!("expected a hash mismatch, got {other:?}"),
        }
    }

    /// Deleting a middle row leaves a gap in the numbering, which the chain
    /// check treats as a broken history rather than a shorter one.
    #[tokio::test]
    async fn verify_chain_detects_a_removed_row() {
        let db = db().await;
        let manager = VersionManager::with_db(db.clone());
        manager.init().await.unwrap();
        history(&manager, 3).await;

        db.execute(&Stmt::raw(format!(
            "DELETE FROM {TAB_VERSION} \
             WHERE doctype = 'Test' AND docname = 'doc-1' AND version = 2"
        )))
        .await
        .unwrap();

        match manager.verify_chain("Test", "doc-1").await {
            Err(VersionChainError::VersionGap { expected, found }) => {
                assert_eq!((expected, found), (2, 3));
            }
            other => panic!("expected a version gap, got {other:?}"),
        }
    }
}
