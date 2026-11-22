//! Helpers for ferris-db's own unit tests.
//!
//! The decoding tests need real rows and `AnyRow` cannot be built by hand, so
//! they run against an in-memory SQLite database — the one backend available
//! without a server.
//!
//! This is a unit-test helper, not an integration-test one: it reaches
//! `crate::memory_db` and `crate::error::map` directly, so it has to be
//! compiled into the crate under `#[cfg(test)]` rather than as a test target of
//! its own. (A file in `tests/` would be its own crate and would only see the
//! public API.)

use ferris_core::Result;
use sqlx::any::AnyRow;

/// A fresh in-memory SQLite pool.
pub async fn memory_pool() -> Result<sqlx::AnyPool> {
    Ok(crate::memory_db().await?.into_pool())
}

/// A one-row result set built from SQL *expressions*.
///
/// `expr` is interpolated verbatim into the select list, so callers pass
/// literals such as `"7"` or `"'text'"`. This is test-only code and never sees
/// user input.
pub async fn row_of(expressions: &[(&str, &str)]) -> Result<AnyRow> {
    let pool = memory_pool().await?;
    let select = expressions
        .iter()
        .map(|(name, expr)| format!("{expr} AS \"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!("SELECT {select}"))
        .fetch_one(&pool)
        .await
        .map_err(crate::error::map)
}
