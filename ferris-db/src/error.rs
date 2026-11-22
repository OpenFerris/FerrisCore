//! The one place a `sqlx::Error` becomes a `ferris_core::Error`.
//!
//! The repositories used to each carry a private `fn db_error(e: sqlx::Error)`
//! — eight byte-identical copies — plus a blanket `#[from]` on
//! `Error::Sqlx`, so the same failure surfaced as three different error
//! variants depending on which file raised it. Mapping here keeps the shape
//! stable and, because the classification happens once, lets callers match on
//! `DatabaseError::Constraint` (which nothing could previously reach, as the
//! message was flattened to a `String`).

use ferris_core::{Error, error::DatabaseError};

/// Classify a driver error so callers can react to it rather than parse text.
///
/// The classification is precise because callers depend on the distinction:
/// the document engine treats a *unique* violation as a conflict
/// (`DocumentError::AlreadyExists`) while an FK or check violation is a data
/// fault, and a `RowNotFound` means "absent record" rather than "query broke".
/// A caller that had to `match` on a flattened `Query(String)` could not tell
/// those apart.
///
/// # Mapping
/// | driver error | variant |
/// |---|---|
/// | `PoolTimedOut`, `PoolClosed` | [`DatabaseError::PoolExhausted`] |
/// | `Io`, `Tls` | [`DatabaseError::Connection`] |
/// | `RowNotFound` | [`DatabaseError::NotFound`] |
/// | unique / FK / check violation | [`DatabaseError::Constraint`] |
/// | anything else | [`DatabaseError::Query`] |
pub fn classify(error: &sqlx::Error) -> DatabaseError {
    match error {
        // A closed pool is not "exhausted" in the queueing sense, but it is a
        // pool-level failure no query can recover from and the caller's
        // correct reaction is the same: reconnect.
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => DatabaseError::PoolExhausted,
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) => DatabaseError::Connection(error.to_string()),
        sqlx::Error::RowNotFound => DatabaseError::NotFound,
        sqlx::Error::Database(db)
            if db.is_unique_violation()
                || db.is_foreign_key_violation()
                || db.is_check_violation() =>
        {
            DatabaseError::Constraint(error.to_string())
        }
        _ => DatabaseError::Query(error.to_string()),
    }
}

/// Map a `sqlx::Error` into the framework error type.
///
/// Takes it by value because that is what `Result::map_err` hands over, and
/// the caller is discarding the driver error either way.
#[allow(clippy::needless_pass_by_value)]
pub fn map(error: sqlx::Error) -> Error {
    Error::Database(classify(&error))
}

/// Whether the error is a unique-constraint violation.
pub fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Whether the error is a connection failure, i.e. worth retrying.
pub fn is_connection_error(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
    )
}

/// Whether the error is a transaction conflict the database resolves by
/// letting a competitor go first, so an immediate retry can succeed.
///
/// That covers a deadlock, a serialization failure, a contended row lock, or a
/// busy SQLite file.
///
/// A statement or lock *timeout* is deliberately not retryable — the work is
/// simply too slow, and retrying it turns a slow query into a hot loop.
///
/// The `Any` driver erases the server's SQLSTATE down to a message, so this
/// matches on the text the three backends emit.
pub fn is_retryable_transaction(error: &sqlx::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("deadlock")
        || message.contains("serialization failure")
        || message.contains("could not serialize access")
        || message.contains("deadlock found when trying to get lock")
        || message.contains("database is locked")
}

/// Whether the error is a "no rows returned" condition, which callers treat as
/// an absent record rather than a fault.
pub fn is_row_not_found(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::RowNotFound)
}

#[cfg(test)]
mod tests;
