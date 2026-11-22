//! Metadata-driven document engine.
//!
//! A single storage layout underpins **both** the typed `#[doctype]` path and
//! the dynamic REST/Desk path, so every write goes through the same lifecycle:
//! validation, lifecycle hooks and (when a user is provided) permissions.
//!
//! Tables are `tab<DocType>` with fixed standard columns plus a `data` JSON
//! blob holding the document fields. The column list is **not** written here:
//! it lives in [`ferris_schema::migration::document_columns`], which the
//! migration runner uses too. The engine used to carry its own copy of the
//! `CREATE TABLE`, and the two definitions were incompatible — the migration
//! declared `modified_by NOT NULL` and no `data`, so whichever ran first
//! decided whether the other could work at all.
//!
//! Storing fields as JSON keeps the layout identical for compiled DocTypes and
//! runtime-defined ones (custom fields), at the cost of not indexing fields.

use crate::filters::{self, ListFilter};
use ferris_core::User;
use ferris_core::hooks::{DocStatus, DocumentHook};
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::{Db, Dialect};
use ferris_schema::DoctypeMeta;
use ferris_schema::contract::NamingType;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

/// Portable list parameters for the engine (filtering happens in Rust).
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    /// Legacy equality filters (raw `HashMap` equality on parsed fields).
    pub filters: HashMap<String, Value>,
    /// Structural operator filters (`[[field, op, value, or?], …]`).
    pub filter_ops: Vec<ListFilter>,
    pub fields: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Column ordering: `"field"`, `"field asc"` or `"field desc"`.
    pub order_by: Option<String>,
    /// Case-insensitive substring match over the document's string values.
    pub search: Option<String>,
    /// Restrict the result to these document names.
    ///
    /// This is how a predicate the document itself cannot answer is fed to the
    /// engine: the caller resolves it against its own side tables (see
    /// `only_liked` in the API layer) and the engine keeps doing the filtering.
    pub only_names: Option<HashSet<String>>,
}

impl ListQuery {
    pub fn matches(&self, value: &Value) -> bool {
        if !filters::matches_filters(value, &self.filters, &self.filter_ops) {
            return false;
        }
        if let Some(only) = &self.only_names {
            match value.get("name").and_then(Value::as_str) {
                Some(name) if only.contains(name) => {}
                _ => return false,
            }
        }
        if let Some(needle) = &self.search {
            let needle = needle.to_lowercase();
            if needle.is_empty() {
                return true;
            }
            if !contains_string(value, &needle) {
                return false;
            }
        }
        true
    }
}

/// Recursively search all string leaves of `value` for `needle` (case-insensitive).
fn contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.to_lowercase().contains(needle),
        Value::Object(map) => map.values().any(|v| contains_string(v, needle)),
        Value::Array(items) => items.iter().any(|v| contains_string(v, needle)),
        _ => false,
    }
}

/// `tab<DocType>` table name, validated as an identifier.
///
/// Re-exported from [`ferris_schema::migration`]: the engine and the migration
/// runner must derive it identically, and two copies of this rule meant
/// `tabToDo` and `"tabToDo"` — two physical tables on Postgres.
pub use ferris_schema::migration::document_table as table_name;

/// Quote a runtime-derived identifier for the dialect in play.
fn q(dialect: Dialect, name: &str) -> Result<String> {
    Ok(ferris_db::sql::Ident::new(name)?.quote(dialect))
}

fn docstatus_str(status: DocStatus) -> &'static str {
    match status {
        DocStatus::Draft => "draft",
        DocStatus::Submitted => "submitted",
        DocStatus::Cancelled => "cancelled",
    }
}

fn docstatus_from_str(value: &str) -> DocStatus {
    match value {
        "submitted" => DocStatus::Submitted,
        "cancelled" => DocStatus::Cancelled,
        _ => DocStatus::Draft,
    }
}

/// Serde-backed [`DocumentHook`] so hooks can run over an untyped document.
struct MapDocument {
    doctype: String,
    name: String,
    fields: HashMap<String, Value>,
    is_new: bool,
}

impl DocumentHook for MapDocument {
    fn doctype(&self) -> &str {
        &self.doctype
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn get_field(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }
    fn set_field(&mut self, field: String, value: Value) {
        self.fields.insert(field, value);
    }
    fn get_fields(&self) -> &HashMap<String, Value> {
        &self.fields
    }
    fn is_new(&self) -> bool {
        self.is_new
    }
    fn docstatus(&self) -> DocStatus {
        self.fields
            .get("docstatus")
            .and_then(|v| v.as_str())
            .map(docstatus_from_str)
            .unwrap_or(DocStatus::Draft)
    }
    fn set_docstatus(&mut self, status: DocStatus) {
        self.fields
            .insert("docstatus".to_string(), Value::from(docstatus_str(status)));
    }
    fn owner(&self) -> &str {
        self.fields
            .get("owner")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }
    fn set_owner(&mut self, owner: String) {
        self.fields.insert("owner".to_string(), Value::from(owner));
    }
    fn modified_by(&self) -> &str {
        self.fields
            .get("modified_by")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }
    fn set_modified_by(&mut self, user: String) {
        self.fields
            .insert("modified_by".to_string(), Value::from(user));
    }
}

fn to_fields(data: &Value) -> Result<HashMap<String, Value>> {
    match data {
        Value::Object(map) => Ok(map.clone()),
        _ => Err(Error::Validation(ValidationError::Invalid(
            "document must be an object".to_string(),
        ))),
    }
}

/// Seed a Single's field defaults into `fields` for keys the caller left unset.
///
/// Frappe materialises a Single's one document during `migrate`, applying each
/// field's `default`, so the row exists with usable values before anyone opens
/// it. This engine materialises it lazily instead, and this is the other half of
/// that: without the defaults, a Single with a required field could never be
/// auto-created. Only absent keys are filled — a value the caller supplied is
/// never overwritten.
fn apply_single_defaults(meta: Option<&DoctypeMeta>, fields: &mut HashMap<String, Value>) {
    let Some(meta) = meta else {
        return;
    };
    for field in &meta.fields {
        let Some(default) = field.default.as_deref() else {
            continue;
        };
        if default.is_empty() || fields.contains_key(&field.fieldname) {
            continue;
        }
        fields.insert(
            field.fieldname.clone(),
            coerce_default(&field.fieldtype, default),
        );
    }
}

/// Parse a `FieldMeta::default` string into the value its `fieldtype` implies.
///
/// Defaults are declared as strings in the registry, so a `Check` default of
/// `"1"` or an `Int` default of `"10"` has to become the typed value validation
/// expects. An unparseable number falls back to the string, which validation
/// then reports as a bad value.
fn coerce_default(fieldtype: &str, default: &str) -> Value {
    match fieldtype {
        "Check" => Value::from(matches!(default, "1" | "true" | "True")),
        // `"Long Int"` used to sit next to `"Int"`: a fieldtype no producer in
        // the repo emits (verified by grep across all crates, ), so
        // it only existed to launder a typo. The registry's allowlist now
        // rejects it outright; `"Int"` is the only integer fieldtype.
        "Int" => default
            .parse::<i64>()
            .map_or_else(|_| Value::from(default), Value::from),
        "Float" | "Currency" | "Percent" => default
            .parse::<f64>()
            .map_or_else(|_| Value::from(default), Value::from),
        _ => Value::from(default),
    }
}

fn fields_to_value(fields: HashMap<String, Value>) -> Value {
    Value::Object(fields.into_iter().collect())
}

/// Settings used by every engine operation.
///
/// **Security:** the engine is default-deny. When `user` is `None`, write
/// operations are rejected with an authorization error unless `allow_anonymous`
/// was explicitly opted in. Prefer resolving the real caller (e.g. via
/// [`crate::with_user`]) over relaxing this flag.
pub struct EngineContext<'a> {
    pub meta: Option<&'a DoctypeMeta>,
    pub doctype: String,
    pub naming_series: String,
    /// How a new document's `name` is minted. Resolved from the doctype's
    /// declaration at construction, so a context that only knows the series
    /// keeps the historical behaviour (series if any, else UUID).
    pub naming_type: NamingType,
    /// Field the name is read from when `naming_type` is
    /// [`NamingType::Field`].
    pub naming_field: Option<String>,
    pub user: Option<User>,
    pub allow_anonymous: bool,
    /// Accept a caller-supplied `name` even when the DocType mints it from a
    /// series/hash/random strategy.
    ///
    /// Only trusted Rust controller paths (`crud` typed inserts) set this; an
    /// HTTP/plugin context leaves it false so a client cannot mint an official
    /// invoice number before the counter does (DOC-16).
    pub allow_explicit_name: bool,
    /// Suppress the plugin `doctype-event` forwarder for this write
    /// (`docstore::write-no-events`): the row is persisted and stamped exactly
    /// as usual, but no guest handler re-enters on it. Used to break the
    /// reentrancy loop a plugin creates when it reacts to its own write.
    pub suppress_events: bool,
    /// Leave `Password` fields unmasked on read.
    ///
    /// This is an internal authentication-store privilege, not a role: the
    /// login flow has to verify a password against the stored Argon2 hash, and
    /// no HTTP caller may ever see it — including a System Manager. Request
    /// contexts must leave this false; only
    /// `ferris_auth::store_support::system_context` opts in.
    pub reveal_password_hash: bool,
    /// Allow [`DocumentEngine::read_secret`] to decrypt `Secret` fields.
    ///
    /// A separate privilege from [`Self::reveal_password_hash`] because the two
    /// hand out different things: that one returns a hash to compare against,
    /// this one returns the plaintext a connector has to sign or authenticate
    /// with. Neither is a role, and neither is set on a request-derived context.
    /// Reads still mask `Secret` values either way; only this flag makes the
    /// plaintext reachable at all.
    pub reveal_secrets: bool,
}

impl<'a> EngineContext<'a> {
    pub fn new(doctype: impl Into<String>, naming_series: impl Into<String>) -> Self {
        let naming_series = naming_series.into();
        let naming_type = NamingType::resolve(None, &naming_series);
        Self {
            meta: None,
            doctype: doctype.into(),
            naming_series,
            naming_type,
            naming_field: None,
            user: None,
            allow_anonymous: false,
            allow_explicit_name: false,
            suppress_events: false,
            reveal_password_hash: false,
            reveal_secrets: false,
        }
    }

    /// Declare the naming strategy explicitly, overriding the implicit
    /// resolution from the series. `field` is only meaningful for
    /// [`NamingType::Field`].
    pub fn with_naming(mut self, naming_type: NamingType, field: Option<String>) -> Self {
        self.naming_type = naming_type;
        self.naming_field = field;
        self
    }

    /// Trust a Rust caller's explicit `name` for a series-backed DocType.
    ///
    /// The typed `crud` path sets this; request contexts must not.
    #[must_use]
    pub fn with_explicit_name(mut self) -> Self {
        self.allow_explicit_name = true;
        self
    }

    /// Suppress the plugin `doctype-event` forwarder for this operation.
    pub fn suppress_events(mut self) -> Self {
        self.suppress_events = true;
        self
    }

    /// Let `Password` fields through `redact_read` unmasked.
    ///
    /// Only the authentication store needs this (to verify a login against the
    /// stored hash). Never call it on a context built from an HTTP request:
    /// the API response redaction is what keeps hashes off every wire.
    #[must_use]
    pub const fn reveal_password_hash(mut self) -> Self {
        self.reveal_password_hash = true;
        self
    }

    /// Let [`DocumentEngine::read_secret`] decrypt `Secret` fields.
    ///
    /// Only a trusted server-side runtime opts in (the integrations/email
    /// connectors loading their own credentials). Never call it on a context
    /// built from an HTTP request: the plaintext never travels on a wire, and a
    /// read still returns [`ferris_core::secret::SECRET_MASK`] whatever this
    /// says.
    #[must_use]
    pub const fn reveal_secrets(mut self) -> Self {
        self.reveal_secrets = true;
        self
    }

    pub fn with_meta(mut self, meta: &'a DoctypeMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn with_user(mut self, user: &User) -> Self {
        self.user = Some(user.clone());
        self
    }

    /// Explicitly allow anonymous (unauthenticated) access to this operation.
    ///
    /// Only use for genuinely public write paths; keep the default otherwise.
    pub fn allow_anonymous(mut self, ok: bool) -> Self {
        self.allow_anonymous = ok;
        self
    }
}

/// One write of a [`DocumentEngine::apply`] batch: the operation together with
/// the DocType-specific context it runs under (doctype, naming series, user,
/// metadata).
pub struct ApplyOp<'a> {
    /// The context of the op's DocType.
    pub ctx: EngineContext<'a>,
    /// The write to perform.
    pub op: ApplyKind,
}

/// The body of an [`ApplyOp`].
pub enum ApplyKind {
    /// Create-or-update by the body's `name`, like `docstore::upsert`.
    Upsert(Value),
    /// Update the named row with the body.
    Update {
        /// Row name.
        name: String,
        /// Partial JSON body.
        data: Value,
    },
    /// Delete the named row.
    Delete {
        /// Row name.
        name: String,
    },
}

/// Metadata-driven document engine.
#[derive(Debug, Clone)]
pub struct DocumentEngine {
    db: Db,
    ensured: Arc<OnceLock<RwLock<HashSet<String>>>>,
}

mod read;
mod secret;
mod shares;
pub(crate) mod support;
mod write;

pub use support::SYSTEM_FIELDS;
pub(crate) use support::is_engine_field;
pub use support::{
    VirtualDoctypeProvider, VirtualFieldProvider, register_virtual_doctype, register_virtual_field,
};

#[cfg(test)]
mod tests;
