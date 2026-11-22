//! Virtual DocFields and Virtual DocTypes.
//!
//! Both are Frappe extension points (Virtual DocField: a computed, read-only
//! field; Virtual DocType: a whole data source that lives outside the document
//! tables):
//!
//! * <https://docs.frappe.io/framework/user/en/basics/virtual_docfield>
//! * <https://docs.frappe.io/framework/user/en/basics/doctypes/virtual-doctype>
//!
//! # Why registries instead of traits on the metadata
//!
//! The registry lives in `ferris-document`, not in `ferris-schema`: metadata is
//! serialized and shipped to the browser, and a function/`Arc` cannot cross that
//! boundary. Keeping the provider registration here means the engine stays the
//! single place that resolves a virtual field on read and rejects a write to one,
//! while the metadata only carries the `virtual` flag.
//!
//! # Synchronous providers on purpose
//!
//! The provider signatures are synchronous. An async trait method needs
//! `async_trait` (a dependency this change must not add) or hand-written
//! `Pin<Box<dyn Future …>>` signatures that make every implementation noisier
//! than the data it computes. A provider that needs I/O owns its own bridge: it
//! can block on its own runtime, or pre-compute into memory and return that. The
//! engine calls providers on the read path only, and a virtual field is expected
//! to be cheap and pure — see [`virtual_read`] for the cost note.
//!
//! # Virtual child tables
//!
//! A virtual field whose `fieldtype` is `Table` is the same registry: the
//! provider returns an array of child rows and the engine inserts it like any
//! other computed value. No separate registration or special case exists.

use super::super::*;
// `redact_read`/`field_read_allowed` are `pub(in crate::engine)` helpers that
// `support` re-exports, so importing the support namespace brings them into
// scope without widening their visibility.
use super::*;
use ferris_core::{DocumentError, Error, Result, Value};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Compute one virtual field from the document as read so far.
///
/// The argument is the redacted document, so a provider never sees a `Password`
/// hash or a field the caller may not read. The returned value is inserted under
/// the field's `fieldname`.
pub type VirtualFieldProvider = fn(&Value) -> Result<Value>;

/// A data source for a Virtual DocType.
///
/// The engine authorizes the action **before** calling the provider; the
/// provider is responsible for fetching/storing the real data and for applying
/// its own row-level access rules if its backend has finer-grained ownership
/// than the DocType metadata can express.
///
/// `list` receives the same [`ListQuery`] the SQL list path receives. It owns
/// filtering, ordering and pagination: the engine reports the number of rows the
/// provider returned as the total. `create`/`update` own naming — a provider that
/// returns a document without a `name` gets a typed error instead of an invented
/// one.
pub trait VirtualDoctypeProvider: Send + Sync {
    fn list(&self, ctx: &EngineContext<'_>, query: &ListQuery) -> Result<Vec<Value>>;
    fn get(&self, ctx: &EngineContext<'_>, name: &str) -> Result<Value>;
    fn create(&self, ctx: &EngineContext<'_>, data: Value) -> Result<Value>;
    fn update(&self, ctx: &EngineContext<'_>, name: &str, data: Value) -> Result<Value>;
    fn delete(&self, ctx: &EngineContext<'_>, name: &str) -> Result<()>;
}

type FieldKey = (String, String);

static VIRTUAL_FIELDS: OnceLock<RwLock<HashMap<FieldKey, VirtualFieldProvider>>> = OnceLock::new();
static VIRTUAL_DOCTYPES: OnceLock<RwLock<HashMap<String, Arc<dyn VirtualDoctypeProvider>>>> =
    OnceLock::new();

fn fields_registry() -> &'static RwLock<HashMap<FieldKey, VirtualFieldProvider>> {
    VIRTUAL_FIELDS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn doctypes_registry() -> &'static RwLock<HashMap<String, Arc<dyn VirtualDoctypeProvider>>> {
    VIRTUAL_DOCTYPES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register the provider for `doctype.field`.
///
/// Re-registering the same pair replaces the previous provider, which makes this
/// idempotent for a dev server that reloads a plugin: repeated registration must
/// not stack duplicates or leak the old function.
pub fn register_virtual_field(doctype: &str, field: &str, provider: VirtualFieldProvider) {
    fields_registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert((doctype.to_string(), field.to_string()), provider);
}

/// Register the data-source provider for a Virtual DocType.
///
/// Assigning the same DocType again replaces the previous provider for the same
/// idempotency reason as [`register_virtual_field`].
pub fn register_virtual_doctype(doctype: &str, provider: Arc<dyn VirtualDoctypeProvider>) {
    doctypes_registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(doctype.to_string(), provider);
}

fn virtual_field_provider(doctype: &str, field: &str) -> Option<VirtualFieldProvider> {
    fields_registry()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(doctype.to_string(), field.to_string()))
        .copied()
}

pub(in crate::engine) fn virtual_doctype_provider(
    doctype: &str,
) -> Option<Arc<dyn VirtualDoctypeProvider>> {
    doctypes_registry()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(doctype)
        .cloned()
}

/// Whether the context's DocType is a Virtual DocType.
pub(in crate::engine) fn is_virtual(ctx: &EngineContext<'_>) -> bool {
    ctx.meta.is_some_and(|m| m.is_virtual)
}

/// Apply [`redact_read`] and then every virtual field to a document about to be
/// returned.
///
/// Redaction runs **first** so a provider cannot observe a `Password` hash or a
/// field above the caller's permlevel, and once more after the providers run so
/// a virtual `Password` field (or one at a restricted permlevel) cannot leak
/// either.
///
/// This runs per row in `list`. A virtual field is expected to be a cheap, pure
/// computation; a provider that performs a remote call per row will turn a page
/// into an N+1. Such a provider should batch/cache on its side — the engine
/// deliberately does not hide that cost.
pub(in crate::engine) fn virtual_read(ctx: &EngineContext<'_>, value: Value) -> Result<Value> {
    let mut document = redact_read(ctx, value);
    let Some(meta) = ctx.meta else {
        return Ok(document);
    };
    if !matches!(document, Value::Object(_)) {
        return Ok(document);
    }
    for field in &meta.fields {
        if !field.r#virtual {
            continue;
        }
        if !field_read_allowed(ctx, field) {
            document.remove(&field.fieldname);
            continue;
        }
        let Some(provider) = virtual_field_provider(&ctx.doctype, &field.fieldname) else {
            return Err(Error::Document(
                DocumentError::VirtualFieldProviderMissing {
                    doctype: ctx.doctype.clone(),
                    field: field.fieldname.clone(),
                },
            ));
        };
        let computed = provider(&document)?;
        document.set(field.fieldname.clone(), computed);
    }
    Ok(redact_read(ctx, document))
}

/// Reject a body that tries to write a field the DocType declares `virtual`.
///
/// Virtual fields are computed on read; accepting a value would persist a value
/// the next read silently overwrites, which is the "silent success" defect
/// `AGENTS.md` §4 forbids. Returns the first offending field so the caller learns
/// exactly which one is read-only.
pub(in crate::engine) fn reject_virtual_field_writes(
    ctx: &EngineContext<'_>,
    fields: &HashMap<String, Value>,
) -> Result<()> {
    let Some(meta) = ctx.meta else {
        return Ok(());
    };
    for field in &meta.fields {
        if field.r#virtual && fields.contains_key(&field.fieldname) {
            return Err(Error::Document(DocumentError::VirtualFieldReadOnly {
                doctype: ctx.doctype.clone(),
                field: field.fieldname.clone(),
            }));
        }
    }
    Ok(())
}

/// Finish a provider write: redact it and require the provider to have named it.
///
/// The engine does not mint a name for a Virtual DocType: the provider owns its
/// identifiers, and inventing one here would create a name the provider cannot
/// resolve on the next read.
pub(in crate::engine) fn finish_virtual_write(
    ctx: &EngineContext<'_>,
    value: Value,
) -> Result<Value> {
    let named = value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| !name.is_empty());
    if !named {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "the provider for virtual DocType `{}` returned a document without a `name`",
            ctx.doctype
        ))));
    }
    virtual_read(ctx, value)
}
