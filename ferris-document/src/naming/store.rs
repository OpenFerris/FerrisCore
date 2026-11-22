//! The persistent, atomic naming counter (`tab_series`).

use super::pattern::format_series;
use crate::engine::support::WriteSink;
use ferris_core::{Error, Result, Value};
use ferris_db::{
    Db, Stmt,
    sql::{Column, UpsertBuilder},
};

const TAB_SERIES: &str = "tab_series";

/// Persistent, atomic naming series backed by the `tab_series` table.
///
/// Frappe parity: the counter lives in the database and the read-modify-write
/// happens inside a transaction, so names never collide across restarts or
/// multiple instances.
pub struct NamingSeriesStore;

impl NamingSeriesStore {
    /// The backing table.
    pub fn table() -> Vec<Column> {
        vec![
            Column::new("name", "VARCHAR(255)").not_null().primary_key(),
            Column::new("current", "BIGINT").not_null().default("0"),
        ]
    }

    /// Create the backing table if it does not exist.
    pub async fn ensure_table(db: &Db) -> Result<()> {
        db.ensure_table(TAB_SERIES, &Self::table()).await
    }

    /// Atomically increment and format the next value for `series`.
    pub async fn next(db: &Db, series: &str) -> Result<String> {
        let current = Self::bump(db, series).await?;
        Ok(format_series(series, current))
    }

    /// Atomically increment and return the next **plain numeric** value for the
    /// auto-increment sequence `key` (`"1"`, `"2"`, …, no prefix, no padding).
    ///
    /// It shares `tab_series` with the naming series but is addressed under a
    /// distinct key (`autoincrement:<doctype>`), so a doctype that declares
    /// both cannot collide the two counters.
    pub async fn next_auto(db: &Db, key: &str) -> Result<String> {
        Ok(Self::bump(db, key).await?.to_string())
    }

    /// [`Self::next`] on an explicit sink, so the counter can be minted inside a
    /// transaction the caller already holds.
    ///
    /// This is what lets a naming series be minted inside
    /// `DocumentEngine::apply`'s batch transaction: a *fresh* `db.begin()` there
    /// would need a second connection (deadlock on a one-connection pool) and
    /// would also escape the batch's rollback, so a rolled-back batch still
    /// burned a series number. The caller must have ensured [`Self::ensure_table`]
    /// before opening the transaction — DDL must never run inside one.
    pub(crate) async fn next_via(sink: &mut WriteSink<'_>, series: &str) -> Result<String> {
        let current = Self::bump_via(sink, series).await?;
        Ok(format_series(series, current))
    }

    /// [`Self::next_auto`] on an explicit sink. See [`Self::next_via`].
    pub(crate) async fn next_auto_via(sink: &mut WriteSink<'_>, key: &str) -> Result<String> {
        Ok(Self::bump_via(sink, key).await?.to_string())
    }

    /// The atomic read-modify-write both counters share: ensure the row, bump
    /// it, read it back — all inside one transaction — and reject an exhausted
    /// (negative) counter before committing.
    async fn bump(db: &Db, key: &str) -> Result<u64> {
        Self::ensure_table(db).await?;
        let mut tx = db.begin().await?;
        let current = Self::bump_via(&mut WriteSink::Tx(&mut tx), key).await?;
        tx.commit().await?;
        Ok(current)
    }

    /// The body of [`Self::bump`] against an explicit sink.
    ///
    /// A `Tx` sink leaves commit/rollback to the caller, so a naming mint inside
    /// a larger batch participates in that batch's atomicity instead of
    /// committing on its own; a `Db` sink is already its own connection.
    ///
    /// The table is ensured only for the `Db` sink. A caller that passes a
    /// transaction (i.e. `DocumentEngine::apply`) guarantees [`Self::ensure_table`]
    /// ran before it opened the transaction, because DDL inside a transaction is
    /// exactly what this split exists to avoid — `apply` does that.
    async fn bump_via(sink: &mut WriteSink<'_>, key: &str) -> Result<u64> {
        if let WriteSink::Db(db) = &*sink {
            Self::ensure_table(db).await?;
        }
        // Make sure the row exists, without disturbing it if it does: the
        // increment below has to apply to one row, every time.
        sink.execute(
            &UpsertBuilder::new(sink.dialect(), TAB_SERIES)
                .values(vec![
                    ("name", Value::from(key)),
                    ("current", Value::from(0)),
                ])
                .conflict_on(["name"])
                .on_conflict_do_nothing()
                .build()?
                .into(),
        )
        .await?;

        sink.execute(&Stmt::new(
            format!("UPDATE {TAB_SERIES} SET current = current + 1 WHERE name = ?"),
            vec![Value::from(key)],
        ))
        .await?;

        let row = sink
            .fetch_one(&Stmt::new(
                format!("SELECT current FROM {TAB_SERIES} WHERE name = ?"),
                vec![Value::from(key)],
            ))
            .await?;
        let current = row.i64("current")?;

        if current < 0 {
            return Err(Error::Document(
                ferris_core::DocumentError::NamingSeriesExhausted(key.to_string()),
            ));
        }
        Ok(current as u64)
    }

    /// Current counter value for a series (0 if it has never been used).
    pub async fn current(db: &Db, series: &str) -> Result<u64> {
        Self::ensure_table(db).await?;
        let row = db
            .fetch_optional(&Stmt::new(
                format!("SELECT current FROM {TAB_SERIES} WHERE name = ?"),
                vec![Value::from(series)],
            ))
            .await?;
        Ok(row
            .map(|r| r.i64("current"))
            .transpose()?
            .unwrap_or(0)
            .max(0) as u64)
    }
}
