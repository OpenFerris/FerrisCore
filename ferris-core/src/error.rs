use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("tenant error: {0}")]
    Tenant(#[from] TenantError),

    #[error("authentication error: {0}")]
    Auth(#[from] AuthError),

    #[error("authorization error: {0}")]
    Authz(#[from] AuthzError),

    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("document error: {0}")]
    Document(#[from] DocumentError),

    #[error("schema error: {0}")]
    Schema(#[from] SchemaError),

    #[error("migration error: {0}")]
    Migration(#[from] MigrationError),

    #[error("job error: {0}")]
    Job(#[from] JobError),

    #[error("realtime error: {0}")]
    Realtime(#[from] RealtimeError),

    #[error("email error: {0}")]
    Email(#[from] EmailError),

    #[error("file error: {0}")]
    File(#[from] FileError),

    #[error("search error: {0}")]
    Search(#[from] SearchError),

    #[error("workflow error: {0}")]
    Workflow(#[from] WorkflowError),

    #[error("serialization error: {0}")]
    Serialization(#[from] SerializationError),

    #[error("secret error: {0}")]
    Secret(#[from] SecretError),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serialization(SerializationError::Json(e))
    }
}

/// `?` on an I/O call keeps the failure *class* instead of flattening every
/// error into an "internal" string: a missing file stays [`FileError::NotFound`]
/// (callers branch on it) and a permission problem stays a storage failure with
/// the OS message attached. Every remaining I/O site in this crate goes through
/// this one conversion, so the classification happens exactly once (§4).
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound {
            Error::File(FileError::NotFound(e.to_string()))
        } else {
            Error::File(FileError::Storage(format!("io error: {e}")))
        }
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("missing required config: {0}")]
    Missing(String),
    #[error("invalid config value for {key}: {msg}")]
    Invalid { key: String, msg: String },
    #[error("config file not found: {0}")]
    NotFound(String),
    #[error("parse error: {0}")]
    Parse(#[from] figment::Error),
}

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("query failed: {0}")]
    Query(String),
    #[error("migration failed: {0}")]
    Migration(String),
    #[error("transaction failed: {0}")]
    Transaction(String),
    #[error("pool exhausted")]
    PoolExhausted,
    #[error("constraint violation: {0}")]
    Constraint(String),
    /// A query that returned no rows where one was required. Callers that mean
    /// "an absent record" distinguish this from a fault instead of parsing the
    /// driver's message.
    #[error("no rows returned")]
    NotFound,
}

#[derive(Error, Debug)]
pub enum TenantError {
    #[error("tenant not found: {0}")]
    NotFound(String),
    #[error("tenant already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid tenant config: {0}")]
    InvalidConfig(String),
    #[error("database setup failed: {0}")]
    DatabaseSetup(String),
    #[error("migration failed: {0}")]
    Migration(String),
}

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("token expired")]
    TokenExpired,
    #[error("token invalid: {0}")]
    TokenInvalid(String),
    #[error("account disabled")]
    AccountDisabled,
    #[error("mfa required")]
    MfaRequired,
    #[error("rate limited")]
    RateLimited,
    /// The user already holds the configured number of concurrent sessions.
    /// Login is refused instead of evicting one: an older tab may be mid-edit,
    /// and silently taking its session away loses work.
    #[error("maximum of {0} concurrent sessions reached; sign out on another device")]
    TooManySessions(u32),
    #[error("oauth error: {0}")]
    OAuthError(String),
    #[error("oauth state mismatch")]
    OAuthStateMismatch,
    #[error("oauth code exchange failed: {0}")]
    OAuthCodeExchange(String),
    #[error("oauth token refresh failed: {0}")]
    OAuthTokenRefresh(String),
    #[error("not found: {0}")]
    NotFound(String),
}

#[derive(Error, Debug)]
pub enum AuthzError {
    #[error("permission denied: {action} on {resource}")]
    PermissionDenied { action: String, resource: String },
    #[error("insufficient role: required {required}, have {actual}")]
    InsufficientRole { required: String, actual: String },
    #[error("tenant access denied")]
    TenantAccessDenied,
    /// A policy document that will not parse. The *shape* of a policy is an
    /// authorisation concern, so it gets an authorisation variant rather than a
    /// generic bad-request error.
    #[error("invalid policy: {0}")]
    InvalidPolicy(String),
}

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("field {field}: {message}")]
    Field { field: String, message: String },
    #[error("required field missing: {0}")]
    Required(String),
    #[error("invalid value for {field}: {message}")]
    InvalidValue { field: String, message: String },
    #[error("constraint violated: {0}")]
    Constraint(String),
    /// A malformed input that is not tied to a named document field (a raw
    /// value, a query parameter, a payload envelope). Prefer [`ValidationError::InvalidValue`]
    /// or a field-specific variant when the field is known; this is the typed
    /// landing spot for malformed input that has no field-specific variant.
    #[error("invalid value: {0}")]
    Invalid(String),
}

#[derive(Error, Debug)]
pub enum DocumentError {
    #[error("document not found: {doctype}/{name}")]
    NotFound { doctype: String, name: String },
    #[error("document already exists: {doctype}/{name}")]
    AlreadyExists { doctype: String, name: String },
    #[error("document is submitted, cannot modify")]
    Submitted,
    #[error("document is cancelled")]
    Cancelled,
    #[error("naming series exhausted: {0}")]
    NamingSeriesExhausted(String),
    #[error("`{doctype}` requires an explicit `name`: its naming type is `prompt`")]
    NameRequired { doctype: String },
    #[error("`{doctype}` is named after `{field}`, but that field is empty or missing")]
    NamingFieldEmpty { doctype: String, field: String },
    #[error("`{doctype}` declares `naming_type = field` but no `naming_field`")]
    NamingFieldUndeclared { doctype: String },
    #[error("`{doctype}` declares `naming_type = script` but no provider is registered")]
    NamingScriptUndeclared { doctype: String },
    #[error("`{doctype}` uses a Naming Series field but `naming_series` is empty")]
    NamingSeriesMissing { doctype: String },
    #[error("`{doctype}` does not allow renaming")]
    RenameNotAllowed { doctype: String },
    #[error("`{doctype}` is virtual but no data-source provider is registered for it")]
    VirtualDoctypeProviderMissing { doctype: String },
    #[error("`{doctype}.{field}` is a virtual field and cannot be written")]
    VirtualFieldReadOnly { doctype: String, field: String },
    #[error("`{doctype}.{field}` is virtual but no provider is registered for it")]
    VirtualFieldProviderMissing { doctype: String, field: String },
    #[error("link validation failed: {field} -> {target}")]
    LinkValidation { field: String, target: String },
    #[error("child table error: {0}")]
    ChildTable(String),
    #[error(
        "`{doctype}` is a Single DocType and has no separate documents; use the single endpoints"
    )]
    SingleNotCreatable { doctype: String },
    #[error("`{doctype}` is a Single DocType and cannot be deleted")]
    SingleNotDeletable { doctype: String },
    #[error("`{doctype}` is not a Single DocType")]
    NotSingle { doctype: String },
}

#[derive(Error, Debug)]
pub enum SchemaError {
    #[error("doctype not found: {0}")]
    DocTypeNotFound(String),
    #[error("field not found: {doctype}.{field}")]
    FieldNotFound { doctype: String, field: String },
    #[error("invalid field type: {0}")]
    InvalidFieldType(String),
    #[error("circular reference detected: {0}")]
    CircularReference(String),
    #[error("duplicate field: {doctype}.{field}")]
    DuplicateField { doctype: String, field: String },
    /// A plugin tried to declare (or replace) a DocType the framework owns.
    #[error("doctype name is reserved by the framework: {0}")]
    ReservedDocType(String),
    /// A plugin tried to declare a DocType another app already owns.
    #[error("doctype {name} is owned by app {owner}")]
    DocTypeOwnedBy { name: String, owner: String },
    /// Two DocTypes would share one physical table (`ToDo` / `To Do`).
    #[error("doctype {name} collides with {other} on table {table}")]
    DocTypeTableCollision {
        name: String,
        other: String,
        table: String,
    },
}

#[derive(Error, Debug)]
pub enum MigrationError {
    #[error("migration {version} failed: {msg}")]
    Failed { version: u32, msg: String },
    #[error("migration {version} already applied")]
    AlreadyApplied { version: u32 },
    #[error("migration {version} not found")]
    NotFound { version: u32 },
    #[error("checksum mismatch for migration {version}")]
    ChecksumMismatch { version: u32 },
    #[error("downgrade not supported for migration {version}")]
    DowngradeNotSupported { version: u32 },
}

#[derive(Error, Debug)]
pub enum JobError {
    #[error("job not found: {0}")]
    NotFound(String),
    #[error("job failed: {0}")]
    Failed(String),
    #[error("job timeout")]
    Timeout,
    #[error("max retries exceeded")]
    MaxRetriesExceeded,
    #[error("dependency failed: {0}")]
    DependencyFailed(String),
    #[error("queue not found: {0}")]
    QueueNotFound(String),
    #[error("queue backend unavailable: {0}")]
    Connection(String),
}

#[derive(Error, Debug)]
pub enum RealtimeError {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("room not found: {0}")]
    RoomNotFound(String),
    #[error("subscription failed: {0}")]
    Subscription(String),
    /// An event could not be encoded. Reported instead of silently emitting a
    /// `null` payload, so a caller can tell "empty event" from "failed to
    /// encode" (§4 no silent success).
    #[error("serialization failed: {0}")]
    Serialization(String),
    /// The transport accepted the call but refused the packet.
    #[error("delivery failed: {0}")]
    Delivery(String),
}

#[derive(Error, Debug)]
pub enum EmailError {
    #[error("smtp error: {0}")]
    Smtp(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("template error: {0}")]
    Template(String),
    #[error("attachment error: {0}")]
    Attachment(String),
}

#[derive(Error, Debug)]
pub enum FileError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("upload failed: {0}")]
    Upload(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("invalid file type: {0}")]
    InvalidType(String),
    #[error("file too large: {size} > {limit}")]
    TooLarge { size: u64, limit: u64 },
}

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("index not found: {0}")]
    IndexNotFound(String),
    #[error("search failed: {0}")]
    SearchFailed(String),
    #[error("indexing failed: {0}")]
    IndexingFailed(String),
    /// A backend or option the crate does not implement. Reported instead of
    /// silently falling back, so a deployment that selects it finds out.
    #[error("search feature not implemented: {0}")]
    Unsupported(String),
}

#[derive(Error, Debug)]
pub enum WorkflowError {
    #[error("workflow not found: {0}")]
    NotFound(String),
    #[error("invalid transition: {from} -> {to} via {action}")]
    InvalidTransition {
        from: String,
        to: String,
        action: String,
    },
    #[error("condition not met: {0}")]
    ConditionNotMet(String),
    #[error("required field missing: {0}")]
    RequiredField(String),
    #[error("approval required")]
    ApprovalRequired,
    #[error("workflow already exists: {0}")]
    AlreadyExists(String),
}

/// Failures of *encoding/decoding* our own data.
///
/// Deliberately free of transport errors: a `redis::RedisError` is not a
/// serialization failure, and keeping the conversion here dragged the whole
/// `redis` stack into the bottom crate of the graph (every crate — including
/// the wasm consumers — paid for it). The crates that actually talk to redis
/// (`ferris-jobs`, `ferris-scheduler`, `ferris-realtime`) classify that error
/// at their own boundary into their domain variants (e.g. `JobError::Connection`).
#[derive(Error, Debug)]
pub enum SerializationError {
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Failures of a recoverable `Secret` field (AES-256-GCM at rest).
///
/// One variant per genuinely different recovery action: configure a key, install
/// the old key, re-enter the value, or treat the ciphertext as tampered. A
/// single "secret error" string would force a caller to parse the message to
/// know which.
#[derive(Error, Debug)]
pub enum SecretError {
    #[error("secret key is not configured (set FERRIS_SECRETS__KEY or [secrets] key)")]
    KeyMissing,
    #[error("invalid secret key: {0}")]
    KeyInvalid(String),
    #[error("this engine context is not allowed to reveal secrets")]
    RevealNotPermitted,
    #[error("field `{field}` of `{doctype}` is not a Secret field")]
    NotSecretField { doctype: String, field: String },
    #[error(
        "secret `{field}` of `{doctype}` was stored before encryption was available (or with \
         another format) and cannot be recovered; re-enter it in the Desk"
    )]
    NotRecoverable { doctype: String, field: String },
    #[error(
        "secret `{field}` of `{doctype}` was encrypted with key `{key_id}`, which is not in \
         `secrets.previous_keys`"
    )]
    UnknownKeyId {
        doctype: String,
        field: String,
        key_id: String,
    },
    #[error(
        "secret `{field}` of `{doctype}` failed authentication (wrong key, value moved to \
         another field, or tampering)"
    )]
    AuthenticationFailed { doctype: String, field: String },
}
