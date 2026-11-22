//! Unit tests for the driver-error classification.
//!
//! Restored from the inline `#[cfg(test)] mod tests` moved into `tests/`
//! by commit 2013f3d.

use super::*;

#[test]
fn a_row_not_found_is_its_own_variant_not_a_query_error() {
    let error = sqlx::Error::RowNotFound;
    // "no rows" is an absent record, not a broken query — a caller that means
    // the former must not have to parse a message to tell them apart.
    assert!(matches!(classify(&error), DatabaseError::NotFound));
    assert!(is_row_not_found(&error));
}

#[test]
fn an_io_failure_is_a_connection_error() {
    let error = sqlx::Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "refused",
    ));
    assert!(matches!(classify(&error), DatabaseError::Connection(_)));
    assert!(is_connection_error(&error));
}

#[test]
fn a_closed_pool_is_reported_as_exhausted() {
    assert!(matches!(
        classify(&sqlx::Error::PoolClosed),
        DatabaseError::PoolExhausted
    ));
}
#[test]
fn a_timed_out_pool_is_reported_as_exhausted() {
    let error = sqlx::Error::PoolTimedOut;
    assert!(matches!(classify(&error), DatabaseError::PoolExhausted));
    assert!(is_connection_error(&error));
}

#[test]
fn connection_failures_and_transaction_conflicts_are_classified() {
    assert!(is_connection_error(&sqlx::Error::PoolClosed));
    assert!(!is_connection_error(&sqlx::Error::RowNotFound));

    for needle in [
        // Postgres
        "deadlock detected",
        "could not serialize access due to concurrent update",
        // MySQL
        "Deadlock found when trying to get lock",
        // SQLite
        "database is locked",
    ] {
        assert!(
            is_retryable_transaction(&sqlx::Error::Protocol(needle.to_string())),
            "should retry: {needle}"
        );
    }

    assert!(!is_retryable_transaction(&sqlx::Error::RowNotFound));

    // A timeout is not a conflict: retrying it would turn a slow query
    // into a hot loop.
    for timeout in [
        "canceling statement due to lock timeout",
        "Lock wait timeout exceeded",
    ] {
        assert!(
            !is_retryable_transaction(&sqlx::Error::Protocol(timeout.to_string())),
            "should not retry: {timeout}"
        );
    }
}

#[test]
fn mapping_always_yields_a_database_error() {
    assert!(matches!(map(sqlx::Error::RowNotFound), Error::Database(_)));
}
