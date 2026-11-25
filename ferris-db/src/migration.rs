//! Migrations: the bookkeeping table, the runner and checksum verification.
//!
//! The dialect is no longer a field the caller sets. It comes from the
//! [`Db`] the migrations run against, so a runner cannot be pointed at a MySQL
//! pool while still emitting Postgres advisory locks — which is what the
//! previous `MigrationRunner::new()` defaulting to `Dialect::Postgres` allowed.

use crate::driver::PinnedConnection;
use crate::exec::{Db, Stmt};
use ferris_core::{DatabaseError, Error, Result, ValidationError};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;

/// Advisory-lock id used to serialise concurrent migration runs.
const MIGRATION_LOCK_ID: i64 = 0x4665_7272_6973; // "Ferris"

/// Name of the bookkeeping table.
pub const MIGRATIONS_TABLE: &str = "_ferris_migrations";

/// Lock name used on MySQL, which has advisory locks of its own.
const MYSQL_LOCK_NAME: &str = "ferris_migrations";

/// A single migration.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Ordering key. Applied in ascending order.
    pub version: u32,
    pub name: String,
    /// Statements to apply. May be several, separated by `;`.
    pub up_sql: String,
    /// Statements to undo. Nothing is run unless a caller asks for it.
    pub down_sql: Option<String>,
    /// Fingerprint of `up_sql`, verified on every run.
    pub checksum: String,
}

impl Migration {
    /// A migration with the checksum computed from `up_sql`.
    pub fn new(
        version: u32,
        name: impl Into<String>,
        up_sql: impl Into<String>,
        down_sql: Option<String>,
    ) -> Self {
        let up_sql = up_sql.into();
        Self {
            version,
            name: name.into(),
            checksum: calculate_checksum(&up_sql),
            up_sql,
            down_sql,
        }
    }
}

/// The result of applying one migration.
#[derive(Debug, Clone)]
pub struct MigrationResult {
    pub version: u32,
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
    pub execution_time_ms: u64,
}

/// A migration that was applied with a checksum other than the one it now has.
#[derive(Debug, Clone)]
pub struct ChecksumMismatch {
    pub version: u32,
    pub name: String,
    /// Checksum recorded in the database.
    pub expected: String,
    /// Checksum of the migration as it exists now.
    pub actual: String,
}

/// SHA-256 of `sql`, hex encoded.
pub fn calculate_checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// The `CREATE TABLE` for the bookkeeping table, per dialect.
///
/// The column types differ because there is no portable spelling: MySQL has no
/// `TEXT DEFAULT`, and a `TIMESTAMP` default expression differs per backend.
fn bookkeeping_ddl(db: &Db) -> Result<String> {
    use crate::dialect::Dialect;
    let d = db.dialect();
    // The name is a constant and always valid, but `quote_ident` validates
    // rather than panicking — the crate keeps zero `expect`/`unwrap` on
    // library paths.
    let table = d.quote_ident(MIGRATIONS_TABLE)?;
    let (version_ty, name_ty, checksum_ty, applied_ty, ms_ty) = match d {
        Dialect::Postgres => (
            "INTEGER",
            "VARCHAR(255)",
            "VARCHAR(64)",
            "TIMESTAMP",
            "INTEGER",
        ),
        Dialect::MySQL => ("INT", "VARCHAR(255)", "VARCHAR(64)", "TIMESTAMP", "INT"),
        Dialect::SQLite => (
            "INTEGER",
            "TEXT",
            "TEXT",
            "TEXT NOT NULL DEFAULT (datetime('now'))",
            "INTEGER",
        ),
    };
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {table} (\
         version {version_ty} PRIMARY KEY, \
         name {name_ty} NOT NULL, \
         checksum {checksum_ty} NOT NULL, \
         applied_at {applied_ty}{applied_default}, \
         execution_time_ms {ms_ty} NOT NULL)",
        applied_default = if matches!(d, Dialect::SQLite) {
            ""
        } else {
            " NOT NULL DEFAULT CURRENT_TIMESTAMP"
        }
    ))
}

/// Create the bookkeeping table if it is absent.
///
/// Called by [`crate::pool::create_pool`] so the runner never finds the table
/// missing, which used to surface as an opaque query failure.
pub async fn ensure_bookkeeping(db: &Db) -> Result<()> {
    db.execute_script(&bookkeeping_ddl(db)?).await
}

/// A set of migrations to apply against a database.
#[derive(Debug, Clone, Default)]
pub struct MigrationRunner {
    migrations: Vec<Migration>,
}

impl MigrationRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a runner from a set of migrations.
    pub fn with_migrations(migrations: impl IntoIterator<Item = Migration>) -> Self {
        Self {
            migrations: migrations.into_iter().collect(),
        }
    }

    pub fn add_migration(&mut self, migration: Migration) {
        self.migrations.push(migration);
    }

    pub fn add_migrations(&mut self, migrations: impl IntoIterator<Item = Migration>) {
        self.migrations.extend(migrations);
    }

    /// The migrations this runner holds, in the order they were added.
    pub fn migrations(&self) -> &[Migration] {
        &self.migrations
    }

    /// Reject a runner whose migrations are out of order or duplicated.
    ///
    /// Ordering matters because migrations are applied in sequence and a
    /// repeated version would mean one of them silently never runs.
    pub fn validate(&self) -> Result<()> {
        let mut previous: Option<u32> = None;
        for migration in &self.migrations {
            match previous {
                Some(p) if p >= migration.version => {
                    return Err(Error::Validation(ValidationError::Invalid(format!(
                        "migration {} ({}) is not after version {p}",
                        migration.version, migration.name
                    ))));
                }
                _ => {}
            }
            previous = Some(migration.version);
        }
        Ok(())
    }

    /// Versions applied to the database, with their recorded checksums.
    pub async fn applied_versions(db: &Db) -> Result<HashMap<u32, String>> {
        // A database that has never been migrated has no bookkeeping table;
        // every migration is pending in that case, not an error.
        ensure_bookkeeping(db).await?;
        let rows = db
            .fetch_all(&Stmt::raw(format!(
                "SELECT version, checksum FROM {MIGRATIONS_TABLE} ORDER BY version"
            )))
            .await?;
        Self::applied_from_rows(rows)
    }

    /// The applied set read on a connection the caller already holds.
    ///
    /// The migration batch pins one connection so its advisory lock, its DDL
    /// and this read all happen on the same session — and so a pool with a
    /// single connection (the in-memory test pool) does not deadlock trying to
    /// check out a second one.
    async fn applied_versions_on(
        conn: &mut PinnedConnection,
        dialect: crate::dialect::Dialect,
    ) -> Result<HashMap<u32, String>> {
        let rows = crate::exec::fetch_all_on(
            conn,
            &Stmt::raw(format!(
                "SELECT version, checksum FROM {MIGRATIONS_TABLE} ORDER BY version"
            )),
            dialect,
        )
        .await?;
        Self::applied_from_rows(rows)
    }

    fn applied_from_rows(rows: Vec<crate::row::Row>) -> Result<HashMap<u32, String>> {
        let mut applied = HashMap::with_capacity(rows.len());
        for row in rows {
            let version = row.i64("version")?;
            let checksum = row.str("checksum")?;
            // A version outside `u32` cannot have come from this runner, which
            // numbers migrations as `u32`; keep it rather than losing the row.
            applied.insert(clamp_version(version), checksum);
        }
        Ok(applied)
    }

    /// Migrations this runner holds that the database has not applied.
    pub async fn pending(&self, db: &Db) -> Result<Vec<&Migration>> {
        let applied = Self::applied_versions(db).await?;
        Ok(self
            .migrations
            .iter()
            .filter(|m| !applied.contains_key(&m.version))
            .collect())
    }

    /// Applied migrations whose checksum no longer matches their SQL.
    ///
    /// Editing an already-applied migration means the database and the source
    /// disagree about what the schema is, so this is reported rather than
    /// ignored.
    pub async fn checksum_mismatches(&self, db: &Db) -> Result<Vec<ChecksumMismatch>> {
        let applied = Self::applied_versions(db).await?;
        Ok(self
            .migrations
            .iter()
            .filter_map(|migration| {
                let recorded = applied.get(&migration.version)?;
                (recorded != &migration.checksum).then(|| ChecksumMismatch {
                    version: migration.version,
                    name: migration.name.clone(),
                    expected: recorded.clone(),
                    actual: migration.checksum.clone(),
                })
            })
            .collect())
    }

    /// Apply every pending migration, refusing to run on a checksum mismatch.
    ///
    /// `target_version` stops after the last migration at or below it.
    pub async fn migrate(
        &self,
        db: &Db,
        target_version: Option<u32>,
    ) -> Result<Vec<MigrationResult>> {
        let mismatches = self.checksum_mismatches(db).await?;
        if !mismatches.is_empty() {
            let detail = mismatches
                .iter()
                .map(|m| format!("{} ({})", m.version, m.name))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "Migration checksum mismatch for: {detail}"
            ))));
        }
        self.apply(db, target_version).await
    }

    /// Apply pending migrations, downgrading a checksum mismatch to a warning.
    ///
    /// For operators accepting schema drift — for instance an
    /// already-applied migration that was edited before checksums existed.
    pub async fn migrate_forced(
        &self,
        db: &Db,
        target_version: Option<u32>,
    ) -> Result<Vec<MigrationResult>> {
        ensure_bookkeeping(db).await?;
        for m in self.checksum_mismatches(db).await? {
            tracing::warn!(
                "migration {} ({}) checksum mismatch; forcing past it (recorded {}, current {})",
                m.version,
                m.name,
                m.expected,
                m.actual
            );
        }
        self.apply(db, target_version).await
    }

    async fn apply(&self, db: &Db, target_version: Option<u32>) -> Result<Vec<MigrationResult>> {
        self.validate()?;
        ensure_bookkeeping(db).await?;

        // Acquire-and-release must happen on ONE session: Postgres advisory
        // locks and MySQL `GET_LOCK` are session-scoped, so a release issued on
        // a different pooled connection is a no-op (it errors with "you don't
        // own a lock") and provides no mutual exclusion at all. Pin a
        // connection for the whole batch, and read the applied set on it too —
        // a pool with a single connection would otherwise deadlock trying to
        // check out a second one.
        let mut conn = db.acquire().await?;
        let lock = self.acquire_lock(&mut conn, db.dialect()).await?;

        let result = self
            .apply_locked(&mut conn, target_version, db.dialect())
            .await;
        // Release on **both** paths. An error after the lock was taken used to
        // return through `?` and leave the pooled session holding it, so the
        // next migration (or process) waited on a lock nobody would release.
        let released = self.release_lock(&mut conn, lock).await;
        match (result, released) {
            (Ok(results), Ok(())) => Ok(results),
            (Err(error), _) => Err(error),
            // The migrations ran but the lock could not be released; report it
            // rather than claiming a clean batch.
            (Ok(_), Err(error)) => Err(error),
        }
    }

    /// The lock-holding half of [`Self::apply`].
    async fn apply_locked(
        &self,
        conn: &mut PinnedConnection,
        target_version: Option<u32>,
        dialect: crate::dialect::Dialect,
    ) -> Result<Vec<MigrationResult>> {
        // `pending` is read *under* the lock: two processes booting together
        // must not both compute the same pending set before either takes the
        // lock, or the second one re-applies migrations the first just wrote.
        let applied = Self::applied_versions_on(conn, dialect).await?;
        let pending: Vec<&Migration> = self
            .migrations
            .iter()
            .filter(|m| !applied.contains_key(&m.version))
            .collect();
        let mut results = Vec::new();
        if pending.is_empty() {
            return Ok(results);
        }

        for migration in pending {
            if target_version.is_some_and(|target| migration.version > target) {
                break;
            }
            results.push(run_one(conn, migration, dialect).await);
        }

        Ok(results)
    }

    /// Take the advisory lock for `db`'s dialect, on the connection that will
    /// hold it until [`Self::release_lock`].
    ///
    /// SQLite needs none: it serialises writers with its file lock.
    async fn acquire_lock(
        &self,
        conn: &mut PinnedConnection,
        dialect: crate::dialect::Dialect,
    ) -> Result<Option<Lock>> {
        use crate::dialect::Dialect;
        match dialect {
            Dialect::SQLite => Ok(None),
            Dialect::Postgres => {
                crate::exec::execute_script_on(
                    conn,
                    &format!("SELECT pg_advisory_lock({MIGRATION_LOCK_ID})"),
                )
                .await?;
                Ok(Some(Lock::Postgres))
            }
            Dialect::MySQL => {
                // `GET_LOCK` returns 1 (acquired), 0 (timed out) or NULL (error).
                // The value used to be discarded, so a timeout silently
                // continued and two processes could run the same migration.
                let rows = crate::exec::fetch_all_on(
                    conn,
                    &Stmt::raw(format!(
                        "SELECT GET_LOCK('{MYSQL_LOCK_NAME}', 30) AS locked"
                    )),
                    dialect,
                )
                .await?;
                let locked = rows.first().and_then(|row| row.opt_i64("locked"));
                check_mysql_lock(locked).map(|()| Some(Lock::MySQL))
            }
        }
    }

    async fn release_lock(&self, conn: &mut PinnedConnection, lock: Option<Lock>) -> Result<()> {
        match lock {
            None => Ok(()),
            Some(Lock::Postgres) => {
                crate::exec::execute_script_on(
                    conn,
                    &format!("SELECT pg_advisory_unlock({MIGRATION_LOCK_ID})"),
                )
                .await
            }
            Some(Lock::MySQL) => {
                crate::exec::execute_script_on(
                    conn,
                    &format!("SELECT RELEASE_LOCK('{MYSQL_LOCK_NAME}')"),
                )
                .await
            }
        }
    }
}

/// Interpret the value `GET_LOCK` returned: 1 means acquired, 0 means the
/// 30-second wait expired, `NULL` means the server reported an error.
fn check_mysql_lock(value: Option<i64>) -> Result<()> {
    if value == Some(1) {
        return Ok(());
    }
    Err(Error::Database(DatabaseError::Migration(format!(
        "could not acquire the migration lock: GET_LOCK returned {}",
        value.map_or_else(|| "NULL".to_string(), |v| v.to_string())
    ))))
}

/// Narrow a stored version to the `u32` the runner uses, saturating rather
/// than wrapping so an out-of-range value cannot alias a real migration.
fn clamp_version(version: i64) -> u32 {
    u32::try_from(version.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
}

/// Milliseconds since `started`, saturated so a `u128` millisecond count cannot
/// wrap a `u64` field.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The advisory lock a runner holds, released by name on the same connection.
enum Lock {
    Postgres,
    MySQL,
}

/// Apply one migration inside a transaction on the pinned connection.
///
/// A failure is recorded rather than propagated so a batch reports every
/// migration that failed, which is what makes `ferris migrate` output useful.
async fn run_one(
    conn: &mut PinnedConnection,
    migration: &Migration,
    dialect: crate::dialect::Dialect,
) -> MigrationResult {
    let started = Instant::now();
    let result = apply(conn, migration, started, dialect).await;
    MigrationResult {
        version: migration.version,
        name: migration.name.clone(),
        success: result.is_ok(),
        error: result.err().map(|e| e.to_string()),
        execution_time_ms: elapsed_ms(started),
    }
}

async fn apply(
    conn: &mut PinnedConnection,
    migration: &Migration,
    started: Instant,
    dialect: crate::dialect::Dialect,
) -> Result<()> {
    // The transaction is managed explicitly on the pinned connection — a
    // `Tx` would have to borrow it, and the connection is already held for the
    // whole batch. `BEGIN`/`COMMIT`/`ROLLBACK` are spelled identically by all
    // three backends.
    crate::exec::execute_script_on(conn, "BEGIN").await?;
    let result = apply_inner(conn, migration, started, dialect).await;
    match &result {
        Ok(()) => crate::exec::execute_script_on(conn, "COMMIT").await?,
        Err(_) => crate::exec::execute_script_on(conn, "ROLLBACK").await?,
    }
    result
}

async fn apply_inner(
    conn: &mut PinnedConnection,
    migration: &Migration,
    started: Instant,
    dialect: crate::dialect::Dialect,
) -> Result<()> {
    // A migration is a script: dialect-aware DDL emits a table plus its
    // indexes, which is more than one statement.
    crate::exec::execute_script_on(conn, &migration.up_sql).await?;

    crate::exec::execute_stmt_on(
        conn,
        &Stmt::new(
            format!(
                "INSERT INTO {MIGRATIONS_TABLE} (version, name, checksum, execution_time_ms) \
                 VALUES (?, ?, ?, ?)"
            ),
            vec![
                ferris_core::Value::from(i64::from(migration.version)),
                ferris_core::Value::from(migration.name.clone()),
                ferris_core::Value::from(migration.checksum.clone()),
                ferris_core::Value::from(i64::try_from(elapsed_ms(started)).unwrap_or(i64::MAX)),
            ],
        ),
        dialect,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests;
