//! Document versioning

use chrono::{DateTime, Utc};
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::{Db, Row, Stmt, sql::Column};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

pub(crate) const TAB_VERSION: &str = "tab_version";

/// The columns the hash chain added after `tab_version` first shipped.
///
/// Nullable on purpose: there is no portable `ADD COLUMN ... NOT NULL` without a
/// default, and a row written before the chain existed has no hash to backfill.
/// Such rows read back with empty strings and [`VersionManager::verify_chain`]
/// reports them as unverifiable rather than as tampered.
fn chain_columns() -> [Column; 2] {
    [
        Column::new("hash", "VARCHAR(64)"),
        Column::new("prev_hash", "VARCHAR(64)"),
    ]
}

pub(crate) fn version_table() -> Vec<Column> {
    let mut columns = vec![
        Column::new("doctype", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("docname", "VARCHAR(255)")
            .not_null()
            .primary_key(),
        Column::new("version", "INTEGER").not_null().primary_key(),
        Column::new("data", "TEXT").not_null(),
        Column::new("changed_by", "VARCHAR(255)").not_null(),
        Column::new("changed_at", "VARCHAR(64)").not_null(),
        Column::new("change_type", "VARCHAR(32)").not_null(),
        Column::new("comment", "TEXT"),
    ];
    columns.extend(chain_columns());
    columns
}

/// Create `tab_version` and add the chain columns to an existing table.
///
/// The single DDL entry point for both the engine's lazy
/// [`crate::runtime::ensure_version_table`] and [`VersionManager::init`]:
/// `ensure_table` covers a fresh database, `ensure_column` covers one created
/// before the chain existed (the duplicate-column error is consumed, so running
/// this on every boot is a no-op), and both go through [`version_table`] so a
/// retyped column definition cannot drift.
#[allow(clippy::result_large_err)] // the crate-wide `Error` type is intentionally unchanged
pub(crate) async fn ensure_version_schema(db: &Db) -> Result<()> {
    db.ensure_table(TAB_VERSION, &version_table()).await?;
    for column in chain_columns() {
        db.ensure_column(TAB_VERSION, &column).await?;
    }
    // `get_versions` filters on `(doctype, docname)` and orders by `version`;
    // the table's primary key is exactly `(doctype, docname, version)`, so no
    // second index is needed and every version insert stays a single index
    // update.
    Ok(())
}

fn row_to_version(row: &Row) -> DocumentVersion {
    DocumentVersion {
        // DOC15: a truncating `as u32` would wrap a version past `u32::MAX` to a
        // small number, so a corrupt/overflowed column would silently look like
        // an early revision. Saturate instead of wrapping.
        version: u32::try_from(row.i64("version").unwrap_or(0)).unwrap_or(u32::MAX),
        doctype: row.str("doctype").unwrap_or_default(),
        docname: row.str("docname").unwrap_or_default(),
        data: row.json_object("data").unwrap_or_default(),
        changed_by: row.str("changed_by").unwrap_or_default(),
        changed_at: row
            .opt_str("changed_at")
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
        change_type: row
            .opt_str("change_type")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(ChangeType::Update),
        comment: row.opt_str("comment"),
        // Rows written before the chain existed have no hash; the decoder has to
        // tolerate them, `verify_chain` is what labels them unverified.
        hash: row.opt_str("hash").unwrap_or_default(),
        prev_hash: row.opt_str("prev_hash").unwrap_or_default(),
    }
}

/// Document version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersion {
    pub version: u32,
    pub doctype: String,
    pub docname: String,
    pub data: HashMap<String, Value>,
    pub changed_by: String,
    pub changed_at: DateTime<Utc>,
    pub change_type: ChangeType,
    pub comment: Option<String>,
    /// SHA-256 (hex) over this row's content and [`Self::prev_hash`].
    ///
    /// Empty for rows written before the chain existed; the engine fills it on
    /// every write from [`VersionManager::verify_chain`]'s point of view.
    #[serde(default)]
    pub hash: String,
    /// The previous version's `hash` for the same `(doctype, docname)`.
    ///
    /// Empty for the first version — the anchor of the chain.
    #[serde(default)]
    pub prev_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeType {
    Create,
    Update,
    Delete,
    Submit,
    Cancel,
    Amend,
    /// A tombstone was reinserted from `Deleted Document`.
    Restore,
}

/// A soft-deleted document recovered from the trash.
///
/// Written by [`crate::engine::DocumentEngine::delete`] for a cancelled
/// document and consumed by [`crate::engine::DocumentEngine::restore`]. The
/// table itself is not a regular DocType table: it is a lazily-created side
/// table owned by the document engine (the same contract as `tab_version`), so
/// older databases gain it on the next delete/restore without a migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedDocument {
    /// Primary key and the value `restore` takes.
    pub restore_token: String,
    pub doctype: String,
    pub docname: String,
    /// The complete document snapshot (the same JSON body the live table held).
    pub data: HashMap<String, Value>,
    pub deleted_by: String,
    pub deleted_at: DateTime<Utc>,
    /// Why the document was deleted; `None` unless the caller supplied one.
    pub reason: Option<String>,
    /// Set once the snapshot was reinserted, so a token restores only once.
    pub restored_at: Option<DateTime<Utc>>,
    /// When set, the retention purge skips this row.
    pub legal_hold: bool,
}

/// Version manager for document versioning.
///
/// Backed by `tab_version` when a database is configured (survives restarts and is
/// shared across instances); otherwise falls back to an in-memory map.
pub struct VersionManager {
    versions: parking_lot::RwLock<HashMap<String, Vec<DocumentVersion>>>,
    db: Option<Db>,
}

impl VersionManager {
    pub fn new() -> Self {
        Self {
            versions: parking_lot::RwLock::new(HashMap::new()),
            db: None,
        }
    }

    pub fn with_db(db: Db) -> Self {
        Self {
            versions: parking_lot::RwLock::new(HashMap::new()),
            db: Some(db),
        }
    }

    /// Create the backing table and add the chain columns to an existing one
    /// (no-op without a database).
    pub async fn init(&self) -> Result<()> {
        if let Some(db) = &self.db {
            ensure_version_schema(db).await?;
        }
        Ok(())
    }

    /// Create a version row outside the document engine.
    ///
    /// Crate-internal: the engine is the single production writer of
    /// `tab_version` (inside the document's transaction, see
    /// [`crate::runtime`]), and every row it writes goes through
    /// [`write_version`], the same function used here. This remains for
    /// [`Self::restore_version`], the activity feed and the crate's own tests;
    /// calling it after a normal write would duplicate the row the engine
    /// already recorded.
    pub(crate) async fn create_version(
        &self,
        doctype: &str,
        docname: &str,
        data: HashMap<String, ferris_core::Value>,
        changed_by: String,
        change_type: ChangeType,
        comment: Option<String>,
    ) -> Result<DocumentVersion> {
        let changed_at = Utc::now();

        if let Some(db) = &self.db {
            // The next version number is derived inside the transaction so a
            // rolled-back insert does not leave a gap that a later read reports.
            let mut tx = db.begin().await?;
            let version = crate::runtime::write_version(
                &mut crate::engine::support::WriteSink::Tx(&mut tx),
                doctype,
                docname,
                data,
                crate::runtime::VersionMeta {
                    changed_by: &changed_by,
                    changed_at,
                    change_type,
                    comment,
                },
            )
            .await?;
            tx.commit().await?;
            return Ok(version);
        }

        let key = format!("{doctype}/{docname}");
        let data_json = crate::runtime::canonical_data(&data)?;
        let mut versions = self.versions.write();
        let doc_versions = versions.entry(key).or_default();
        // DOC15: saturate rather than truncate a counter past `u32::MAX`.
        let version_num = u32::try_from(doc_versions.len() + 1).unwrap_or(u32::MAX);
        let mut version = DocumentVersion {
            version: version_num,
            doctype: doctype.to_string(),
            docname: docname.to_string(),
            data,
            changed_by,
            changed_at,
            change_type,
            comment,
            hash: String::new(),
            prev_hash: doc_versions
                .last()
                .map(|previous| previous.hash.clone())
                .unwrap_or_default(),
        };
        version.hash = crate::runtime::version_hash(&version, &data_json);
        doc_versions.push(version.clone());
        // Release the writer lock before returning: the guard only protects the
        // map mutation above, not the value handed back.
        drop(versions);
        Ok(version)
    }

    /// Get all versions for a document
    pub async fn get_versions(&self, doctype: &str, docname: &str) -> Result<Vec<DocumentVersion>> {
        if let Some(db) = &self.db {
            let rows = db
                .fetch_all(&Stmt::new(
                    format!(
                        "SELECT * FROM {TAB_VERSION} WHERE doctype = ? AND docname = ? ORDER BY version ASC"
                    ),
                    vec![Value::from(doctype), Value::from(docname)],
                ))
                .await?;
            return Ok(rows.iter().map(row_to_version).collect());
        }

        let key = format!("{doctype}/{docname}");
        Ok(self.versions.read().get(&key).cloned().unwrap_or_default())
    }

    /// Get a specific version
    pub async fn get_version(
        &self,
        doctype: &str,
        docname: &str,
        version: u32,
    ) -> Result<Option<DocumentVersion>> {
        let versions = self.get_versions(doctype, docname).await?;
        Ok(versions.into_iter().find(|v| v.version == version))
    }

    /// Restore a document to a specific version
    pub async fn restore_version(
        &self,
        doctype: &str,
        docname: &str,
        version: u32,
        restored_by: String,
    ) -> Result<HashMap<String, ferris_core::Value>> {
        let versions = self.get_versions(doctype, docname).await?;

        if let Some(version_data) = versions.into_iter().find(|v| v.version == version) {
            // Create a new version for the restore
            self.create_version(
                doctype,
                docname,
                version_data.data.clone(),
                restored_by,
                ChangeType::Update,
                Some(format!("Restored from version {}", version)),
            )
            .await?;

            Ok(version_data.data)
        } else {
            Err(Error::Validation(ferris_core::ValidationError::Field {
                field: "version".to_string(),
                message: format!("Version {} not found", version),
            }))
        }
    }

    /// Get version count
    pub async fn version_count(&self, doctype: &str, docname: &str) -> Result<u32> {
        // DOC15: `len() as u32` truncates, and in release a wrapped count is a
        // silently wrong answer. Saturate at `u32::MAX` instead.
        let len = self.get_versions(doctype, docname).await?.len();
        Ok(u32::try_from(len).unwrap_or(u32::MAX))
    }

    /// Verify the hash chain of one document's history.
    ///
    /// Walks the versions in ascending order and checks three things for each
    /// row: the numbering is contiguous from `1` (so a removed row leaves a
    /// gap), `prev_hash` names the preceding row's `hash`, and the stored `hash`
    /// hashes the row's own content. Returns how many rows were verified.
    ///
    /// Rows written before the chain existed carry no hash; they are skipped and
    /// not counted. The chain is only tamper-*evident*: an attacker who can
    /// rewrite the last remaining row, or the whole chain, cannot be detected
    /// without an external anchor (WORM storage or a signature), which this
    /// module does not provide yet. Deleting the newest rows is likewise
    /// undetectable from the table alone.
    ///
    /// # Errors
    /// [`VersionChainError`] names whether the read failed, a version is
    /// missing, a link is broken or a row's content changed.
    #[allow(clippy::result_large_err)] // `VersionChainError` wraps the crate-wide error type
    pub async fn verify_chain(
        &self,
        doctype: &str,
        docname: &str,
    ) -> std::result::Result<u64, VersionChainError> {
        let versions = self
            .get_versions(doctype, docname)
            .await
            .map_err(VersionChainError::Storage)?;
        let mut expected_prev = String::new();
        let mut expected_version = 1_u32;
        let mut verified = 0_u64;
        for version in &versions {
            if version.version != expected_version {
                return Err(VersionChainError::VersionGap {
                    expected: expected_version,
                    found: version.version,
                });
            }
            expected_version = expected_version.saturating_add(1);
            if version.hash.is_empty() {
                // Pre-chain row: it occupies its version number but carries no
                // proof. A later hashed row anchors on an empty `prev_hash`.
                expected_prev.clear();
                continue;
            }
            if version.prev_hash != expected_prev {
                return Err(VersionChainError::BrokenLink {
                    version: version.version,
                });
            }
            let data_json = crate::runtime::canonical_data(&version.data)
                .map_err(VersionChainError::Storage)?;
            if crate::runtime::version_hash(version, &data_json) != version.hash {
                return Err(VersionChainError::HashMismatch {
                    version: version.version,
                });
            }
            expected_prev.clone_from(&version.hash);
            verified = verified.saturating_add(1);
        }
        Ok(verified)
    }
}

/// Why [`VersionManager::verify_chain`] rejected a document's history.
#[derive(Debug)]
pub enum VersionChainError {
    /// The rows could not be read (or their data could not be re-canonicalized).
    Storage(Error),
    /// The version numbers are not the contiguous sequence starting at one, so a
    /// row was removed.
    VersionGap { expected: u32, found: u32 },
    /// A row's `prev_hash` does not name the preceding row's `hash`.
    BrokenLink { version: u32 },
    /// A row's stored `hash` does not match the content it holds.
    HashMismatch { version: u32 },
}

impl fmt::Display for VersionChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "version chain unavailable: {error}"),
            Self::VersionGap { expected, found } => write!(
                f,
                "version chain gap: expected version {expected}, found {found}"
            ),
            Self::BrokenLink { version } => {
                write!(f, "version {version} does not link to its predecessor")
            }
            Self::HashMismatch { version } => {
                write!(f, "version {version} does not match its stored hash")
            }
        }
    }
}

impl std::error::Error for VersionChainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::VersionGap { .. } | Self::BrokenLink { .. } | Self::HashMismatch { .. } => None,
        }
    }
}

impl From<VersionChainError> for Error {
    fn from(error: VersionChainError) -> Self {
        // Keeps `verify_chain` callable from a `ferris_core::Result` path while
        // still exposing the typed variant to callers that match on it.
        Self::Validation(ValidationError::Invalid(error.to_string()))
    }
}

/// The enum is stored as its JSON name so the column stays human-readable and
/// `row_to_version` can decode it with the same serde impls.
///
/// `pub(crate)` because [`crate::runtime::record_version_in_tx`] writes the same
/// column inside the engine's transaction (DOC7): the encoder lives here, once,
/// so a version written by the engine and one written by
/// [`VersionManager::create_version`] decode identically.
pub(crate) fn change_type_json(change_type: ChangeType) -> String {
    serde_json::to_string(&change_type).unwrap_or_else(|_| "\"Update\"".to_string())
}

impl Default for VersionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        ferris_db::memory_db().await.unwrap()
    }

    #[tokio::test]
    async fn test_version_manager_backed_by_db() {
        let manager = VersionManager::with_db(db().await);
        manager.init().await.unwrap();

        let mut data = HashMap::new();
        data.insert("field1".to_string(), Value::from("value1"));

        let v1 = manager
            .create_version(
                "Test",
                "doc-1",
                data.clone(),
                "user1".to_string(),
                ChangeType::Create,
                None,
            )
            .await
            .unwrap();
        assert_eq!(v1.version, 1);

        data.insert("field2".to_string(), Value::from("value2"));
        let v2 = manager
            .create_version(
                "Test",
                "doc-1",
                data.clone(),
                "user1".to_string(),
                ChangeType::Update,
                Some("second".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(v2.version, 2);

        // The round trip has to survive the database encoding, including the
        // `ChangeType` stored as JSON and the nullable comment.
        let versions = manager.get_versions("Test", "doc-1").await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].change_type, ChangeType::Create);
        assert_eq!(versions[1].change_type, ChangeType::Update);
        assert_eq!(versions[1].comment.as_deref(), Some("second"));
        assert_eq!(
            versions[0].data.get("field1").unwrap().as_str(),
            Some("value1")
        );
        assert!(versions[0].data.get("field2").is_none());

        let restored = manager
            .restore_version("Test", "doc-1", 1, "user1".to_string())
            .await
            .unwrap();
        assert_eq!(restored.get("field1").unwrap().as_str(), Some("value1"));
        assert!(restored.get("field2").is_none());
        assert_eq!(manager.version_count("Test", "doc-1").await.unwrap(), 3);

        assert!(
            manager
                .get_version("Test", "doc-1", 99)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_version_manager() {
        let manager = VersionManager::new();

        let mut data = std::collections::HashMap::new();
        data.insert("field1".to_string(), ferris_core::Value::from("value1"));

        let v1 = manager
            .create_version(
                "Test",
                "doc-1",
                data.clone(),
                "user1".to_string(),
                ChangeType::Create,
                None,
            )
            .await
            .unwrap();

        assert_eq!(v1.version, 1);
        assert_eq!(v1.change_type, ChangeType::Create);

        data.insert("field2".to_string(), ferris_core::Value::from("value2"));
        let v2 = manager
            .create_version(
                "Test",
                "doc-1",
                data.clone(),
                "user1".to_string(),
                ChangeType::Update,
                None,
            )
            .await
            .unwrap();

        assert_eq!(v2.version, 2);

        let versions = manager.get_versions("Test", "doc-1").await.unwrap();
        assert_eq!(versions.len(), 2);

        let restored = manager
            .restore_version("Test", "doc-1", 1, "user1".to_string())
            .await
            .unwrap();
        assert_eq!(restored.get("field1").unwrap().as_str(), Some("value1"));
        assert!(restored.get("field2").is_none());
    }
}
