use ferris_core::{Result, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, EngineContext};
use ferris_schema::DoctypeMeta;

use super::meta::context;
use super::{DocStore, effective_meta, validate_doctype};

impl DocStore {
    /// Read the one document of a Single `DocType`, materialising it with its
    /// field defaults on first access.
    ///
    /// A Single has no list and no caller-chosen name, so it is reached through
    /// this dedicated entry point rather than `/api/resource/{doctype}`.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an unknown `doctype`, the engine
    /// permission error, or the storage error of the read/insert.
    pub async fn get_single(pool: &Db, doctype: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        DocumentEngine::new(pool.clone())
            .get_single(&context(doctype, "", meta.as_ref()))
            .await
    }

    /// Read (or materialise) the one document of a Single `DocType` **with the
    /// read check bypassed**.
    ///
    /// A Single holds install-wide framework state — `System Settings`, `Global
    /// Search Settings`, the navbar/website settings — that the host reads to
    /// drive behaviour for *every* request, regardless of who is asking. Running
    /// that read as the ambient caller would make the settings invisible to any
    /// principal without `Read` on them, which is the opposite of install-wide.
    ///
    /// The engine is default-deny, so the caller opts out explicitly here: the
    /// ambient user is dropped and anonymous reads are allowed. Only the read
    /// bypasses — [`DocStore::save_single`] stays authorised, so who may *change*
    /// a setting is still governed by its role rules.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an unknown `doctype`, and the storage
    /// error of the read/insert (never a permission error — that is the point).
    #[allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.
    pub async fn get_single_system(pool: &Db, doctype: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        let ctx = system_context(doctype, meta.as_ref());
        DocumentEngine::new(pool.clone()).get_single(&ctx).await
    }

    /// Read a system Single **without materialising it**.
    ///
    /// A GET must not turn into an INSERT (LOGIC-06): `/api/search` only needs
    /// the configured defaults, and a missing row means "use them". The caller
    /// gets an empty object rather than a synthetic row.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an unknown `doctype` and the storage
    /// error of the read; a missing row is `Ok({})`.
    pub async fn read_single_system(pool: &Db, doctype: &str) -> Result<Value> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        let mut ctx = context(doctype, "", meta.as_ref());
        let mut system =
            ferris_core::User::system_user("system@ferris.local".to_string(), "System".to_string());
        system.id = "system@ferris.local".to_string();
        system.roles = vec!["System Manager".to_string()];
        ctx.user = Some(system);
        match DocumentEngine::new(pool.clone()).get(&ctx, doctype).await {
            Ok(value) => Ok(value),
            Err(ferris_core::Error::Document(ferris_core::DocumentError::NotFound { .. })) => {
                Ok(Value::Object(ferris_core::Map::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// Save the one document of a Single `DocType` (upsert).
    ///
    /// # Errors
    ///
    /// Returns a validation error for an unknown `doctype`, the engine
    /// permission/validation error, or the storage error of the write.
    pub async fn save_single(pool: &Db, doctype: &str, data: Value) -> Result<Value> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        DocumentEngine::new(pool.clone())
            .save_single(&context(doctype, "", meta.as_ref()), data)
            .await
    }
}

/// The framework's own system identity for boot-time writes.
///
/// The seed runs before any session exists, and the rows it writes are
/// install-wide reference data (`Module Def`, `Language`), not caller-owned
/// documents. Running as the system principal keeps the engine default-deny
/// while still allowing the write, exactly like `get_single_system`.
fn system_context<'a>(doctype: &str, meta: Option<&'a DoctypeMeta>) -> EngineContext<'a> {
    let mut ctx = context(doctype, "", meta);
    let mut system =
        ferris_core::User::system_user("system@ferris.local".to_string(), "System".to_string());
    system.id = "system@ferris.local".to_string();
    system.roles = vec!["System Manager".to_string()];
    ctx.user = Some(system);
    ctx.allow_anonymous = false;
    ctx
}

impl DocStore {
    /// Insert a boot-time reference row with an explicit name.
    ///
    /// Only for framework seeds (`Module Def`, `Language`): the name is the
    /// identifier other records link to, so it cannot be minted.
    ///
    /// # Errors
    ///
    /// Validation, permission or storage errors from the engine.
    pub async fn create_system(pool: &Db, doctype: &str, value: Value) -> Result<Value> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        let ctx = system_context(doctype, meta.as_ref());
        DocumentEngine::new(pool.clone()).create(&ctx, value).await
    }

    /// Whether a boot-time reference row already exists.
    ///
    /// # Errors
    ///
    /// Validation errors for an unknown `doctype` and storage errors other than
    /// "not found".
    pub async fn exists_system(pool: &Db, doctype: &str, name: &str) -> Result<bool> {
        validate_doctype(doctype)?;
        let meta = effective_meta(pool, doctype).await?;
        let ctx = system_context(doctype, meta.as_ref());
        match DocumentEngine::new(pool.clone()).get(&ctx, name).await {
            Ok(_) => Ok(true),
            Err(ferris_core::Error::Document(ferris_core::DocumentError::NotFound { .. })) => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}
