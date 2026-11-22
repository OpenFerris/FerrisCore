//! Typed CRUD for [`Document`] implementations.
//!
//! `#[doctype]`-generated types delegate here. The typed document is serialised
//! and handled by the metadata-driven [`DocumentEngine`], so typed and dynamic
//! documents share the same storage, validation, hooks and naming.

use ferris_core::{Error, Result, SerializationError, ValidationError, Value};
use ferris_db::Db;

use crate::document::{Document, DocumentExt};
use crate::engine::{DocumentEngine, EngineContext};

fn document_value<T: Document>(doc: &T) -> Result<Value> {
    let json =
        serde_json::to_value(doc).map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
    Ok(Value::from_json(json))
}

fn decode<T: Document>(stored: Value) -> Result<T> {
    serde_json::from_value(stored.to_json())
        .map_err(|e| Error::Serialization(SerializationError::Json(e)))
}

fn context<'a, T: Document>(
    meta: &'a Option<std::sync::Arc<ferris_schema::DoctypeMeta>>,
) -> EngineContext<'a> {
    let mut ctx = EngineContext::new(T::DOCTYPE, T::NAMING_SERIES);
    // The trait's declaration wins over the implicit series resolution, but an
    // undeclared doctype (both `None`) keeps exactly what `EngineContext::new`
    // resolved.
    let naming_type = T::NAMING_TYPE.unwrap_or(ctx.naming_type);
    ctx = ctx.with_naming(naming_type, T::NAMING_FIELD.map(str::to_string));
    if let Some(meta) = meta.as_ref() {
        ctx.meta = Some(meta.as_ref());
    }
    // Authorise the typed operation as the ambient task user (set by framework
    // entry points via `crate::with_user`). Anonymous typed CRUD is denied.
    ctx.user = crate::auth::current_user();
    // Trusted Rust controller path: it may name a document explicitly even when
    // the DocType mints a series. The HTTP/plugin path does not go through
    // `crud` and keeps the DOC-16 refusal.
    ctx.allow_explicit_name = true;
    ctx
}

/// Insert a new document, assigning a name when missing.
pub async fn insert<T: Document>(doc: &mut T, db: &Db) -> Result<String> {
    let value = document_value(doc)?;
    let meta = T::meta();
    let stored = DocumentEngine::new(db.clone())
        .create(&context::<T>(&meta), value)
        .await?;
    *doc = decode(stored)?;
    Ok(doc.get_name().to_string())
}

/// Update an existing document by name.
pub async fn save<T: Document>(doc: &mut T, db: &Db) -> Result<()> {
    if doc.get_name().is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "Cannot save a document without a name".to_string(),
        )));
    }
    let name = doc.get_name().to_string();
    let value = document_value(doc)?;
    let meta = T::meta();
    let stored = DocumentEngine::new(db.clone())
        .update(&context::<T>(&meta), &name, value)
        .await?;
    *doc = decode(stored)?;
    Ok(())
}

/// Delete a document by name.
pub async fn delete<T: Document>(doc: &T, db: &Db) -> Result<()> {
    if doc.get_name().is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "Cannot delete a document without a name".to_string(),
        )));
    }
    let name = doc.get_name().to_string();
    let meta = T::meta();
    DocumentEngine::new(db.clone())
        .delete(&context::<T>(&meta), &name)
        .await
}

/// Submit a document (Draft -> Submitted).
pub async fn submit<T: Document>(doc: &mut T, db: &Db) -> Result<()> {
    transition::<T>(doc, db, ferris_core::DocStatus::Submitted).await
}

/// Cancel a document (Submitted -> Cancelled).
pub async fn cancel<T: Document>(doc: &mut T, db: &Db) -> Result<()> {
    transition::<T>(doc, db, ferris_core::DocStatus::Cancelled).await
}

async fn transition<T: Document>(
    doc: &mut T,
    db: &Db,
    target: ferris_core::DocStatus,
) -> Result<()> {
    if doc.get_name().is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "Cannot transition a document without a name".to_string(),
        )));
    }
    let name = doc.get_name().to_string();
    let meta = T::meta();
    let stored = DocumentEngine::new(db.clone())
        .set_docstatus(&context::<T>(&meta), &name, target)
        .await?;
    *doc = decode(stored)?;
    Ok(())
}

/// Amend a cancelled document: clone it as a fresh draft named `NAME-1`, ….
///
/// Frappe appends a counter to the amended copy rather than minting an unrelated
/// name, so the chain of amendments is visible from the name itself.
///
/// The typed path deliberately delegates to [`DocumentEngine::amend`] instead of
/// cloning the struct and calling [`insert`]: the engine applies `Permission::Amend`
/// to the source, requires it to be `Cancelled`, records `amended_from` and runs
/// the ordinary insert lifecycle. A second implementation here is how the typed
/// path used to skip the `Amend` check and the provenance link (DOC-02).
pub async fn amend<T: Document>(doc: &T, db: &Db) -> Result<String> {
    let original = doc.get_name().to_string();
    if original.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "Cannot amend a document without a name".to_string(),
        )));
    }
    let meta = T::meta();
    let stored = DocumentEngine::new(db.clone())
        .amend(&context::<T>(&meta), &original)
        .await?;
    Ok(stored
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

/// Load the typed Single document, materialising it with its field defaults on
/// first access.
///
/// Only meaningful for an `is_single` `DocType`; the engine refuses a
/// non-single, so calling it on one is a bug surfaced as a `400`, not a silent
/// list of one.
///
/// # Errors
///
/// Returns the engine error when the `DocType` is not a Single, the caller may
/// not read it, or the stored row cannot be decoded into `T`.
pub async fn get_single<T: Document>(db: &Db) -> Result<T> {
    let meta = T::meta();
    let stored = DocumentEngine::new(db.clone())
        .get_single(&context::<T>(&meta))
        .await?;
    decode(stored)
}

/// Save the typed Single document in place (upsert, fixed name).
///
/// # Errors
///
/// Returns the engine error when the `DocType` is not a Single, the caller may
/// not write it, validation fails, or the stored row cannot be decoded into
/// `T`.
pub async fn save_single<T: Document>(doc: &mut T, db: &Db) -> Result<()> {
    let value = document_value(doc)?;
    let meta = T::meta();
    let stored = DocumentEngine::new(db.clone())
        .save_single(&context::<T>(&meta), value)
        .await?;
    *doc = decode(stored)?;
    Ok(())
}
