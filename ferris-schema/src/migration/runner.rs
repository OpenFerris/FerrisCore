//! Turn the shared DocType table definition into numbered migrations.

#[cfg(test)]
use super::schema::{
    DeclaredIndex, classify_index, document_columns, document_indexes, is_indexable_column,
};
use super::schema::{document_table, schema_version};
use crate::registry::{get_doctype, list_doctypes};
use ferris_core::Result;
use ferris_db::sql::Ident;
use ferris_db::{Db, Dialect, Migration, MigrationRunner as DbMigrationRunner};

pub struct MigrationRunner {
    pub(super) db_runner: DbMigrationRunner,
    dialect: Dialect,
}

impl MigrationRunner {
    /// Build a runner that generates DDL for `db`'s dialect.
    ///
    /// The dialect cannot be defaulted: guessing wrong emits Postgres SQL for a
    /// MySQL server, which is exactly what happened when this took a
    /// `with_dialect` that defaulted to `Dialect::Postgres`.
    pub fn for_db(db: &Db) -> Self {
        Self {
            db_runner: DbMigrationRunner::new(),
            dialect: db.dialect(),
        }
    }

    /// Register one migration per registered DocType, each under a version
    /// derived from its name.
    ///
    /// Every DocType's table migration gets a version derived **only from the
    /// DocType name**, never from its position in the registry.
    ///
    /// The previous shape (M2) hashed the name into a 1M bucket and could
    /// collide; the version after it was a monotonic counter over a
    /// name-sorted list. The counter fixed the collision but coupled a
    /// DocType's version to the *set* of other DocTypes: registering or
    /// removing one shifted every later name by one. That is fatal, not
    /// cosmetic — `MigrationRunner::migrate` refuses to run when any recorded
    /// version's checksum no longer matches, so inserting one DocType earlier in
    /// sort order turned `ferris migrate` into a checksum error on every
    /// already-migrated database (D4 / S7). The registry is rebuilt from
    /// compiled code *and* plugin `describe()` output, so that set is not stable
    /// across deployments.
    ///
    /// A content-derived version decouples them: the number a DocType gets does
    /// not depend on which other DocTypes happen to be registered, so adding or
    /// removing one leaves every other version (and the checksum guard)
    /// untouched. The space is wide (see [`schema_version`]) and a collision is
    /// reported rather than silently dropping a table.
    ///
    /// # Errors
    ///
    /// Returns [`ferris_core::ValidationError::Invalid`] when two DocTypes
    /// derive the same version (a hash collision) or a DocType name is not a
    /// usable identifier.
    pub fn add_schema_migrations(&mut self) -> Result<()> {
        let mut doctypes: Vec<String> = list_doctypes();
        doctypes.sort();

        // Assign first, then sort by the derived version: `validate` requires
        // strictly increasing versions, and hashed values are unordered.
        let mut planned: Vec<(u32, String, Migration)> = Vec::with_capacity(doctypes.len());
        for doctype_name in doctypes {
            let Some(meta) = get_doctype(&doctype_name) else {
                continue;
            };
            let version = schema_version(&meta.name);
            let table = document_table(&meta.name)?;
            // The native generator creates the system columns, the declared
            // columns, the child tables and every index in one migration; the
            // blob-only `create_table_statements` is gone from this path.
            let sql = format!(
                "{};\n",
                crate::doctype_columns::create_doc_type_statements(&meta, self.dialect)?
                    .join(";\n")
            );
            let migration = Migration {
                version,
                name: format!(
                    "create_table_{}",
                    doctype_name.to_lowercase().replace(' ', "_")
                ),
                down_sql: Some(format!("DROP TABLE IF EXISTS {}", self.quote(&table)?)),
                checksum: ferris_db::migration::calculate_checksum(&sql),
                up_sql: sql,
            };
            planned.push((version, meta.name.clone(), migration));
        }
        planned.sort_by_key(|(version, _, _)| *version);

        // Two DocTypes under one version would silently lose a table: the runner
        // records the first and skips the second as already applied. Name both
        // and refuse instead.
        for pair in planned.windows(2) {
            if let [(version, first, _), (_, second, _)] = pair
                && version == &schema_version(second)
            {
                return Err(ferris_core::Error::Validation(
                    ferris_core::ValidationError::Invalid(format!(
                        "schema migration version {version} collides between \
                         DocTypes `{first}` and `{second}`; rename one or widen \
                         the version space"
                    )),
                ));
            }
        }

        for (_, _, migration) in planned {
            self.db_runner.add_migration(migration);
        }
        Ok(())
    }

    fn quote(&self, name: &str) -> Result<String> {
        Ok(Ident::new(name)?.quote(self.dialect))
    }

    pub async fn run(
        &self,
        db: &Db,
        target_version: Option<u32>,
    ) -> Result<Vec<ferris_db::migration::MigrationResult>> {
        self.db_runner.migrate(db, target_version).await
    }

    /// Run pending migrations, downgrading checksum mismatches to warnings
    /// (`ferris migrate --force`).
    pub async fn run_forced(
        &self,
        db: &Db,
        target_version: Option<u32>,
    ) -> Result<Vec<ferris_db::migration::MigrationResult>> {
        self.db_runner.migrate_forced(db, target_version).await
    }

    /// List migrations that have not been applied yet (no side effects).
    pub async fn pending(&self, db: &Db) -> Result<Vec<PendingMigration>> {
        Ok(self
            .db_runner
            .pending(db)
            .await?
            .into_iter()
            .map(|m| PendingMigration {
                version: m.version,
                name: m.name.clone(),
            })
            .collect())
    }
}

/// A migration that has not been applied yet.
#[derive(Debug, Clone)]
pub struct PendingMigration {
    pub version: u32,
    pub name: String,
}

/// Every statement that brings a DocType's table into existence.
///
/// The table comes first because the indexes reference it, and the standard
/// indexes come before the DocType's own so a duplicate name in `meta.indexes`
/// fails loudly on its own index rather than silently shadowing a standard one.
/// Every index name is qualified with its table by
/// [`ferris_db::sql::ensure_index`], so two DocTypes cannot share one physical
/// index (SCH-03) and a `UNIQUE` declaration keeps its uniqueness per table.
#[cfg(test)]
pub(super) fn create_table_statements(
    dialect: Dialect,
    table: &str,
    meta: &crate::registry::DoctypeMeta,
) -> Result<Vec<String>> {
    let mut statements = vec![ferris_db::sql::create_table(
        dialect,
        table,
        &document_columns(),
    )?];
    for (name, columns, unique) in document_indexes() {
        statements.push(ferris_db::sql::ensure_index(
            dialect, name, table, columns, unique,
        )?);
    }
    for index in &meta.indexes {
        let unique = index.unique;
        match classify_index(index) {
            DeclaredIndex::Columns => {
                let columns: Vec<&str> = index.fields.iter().map(String::as_str).collect();
                statements.push(ferris_db::sql::ensure_index(
                    dialect,
                    &index.name,
                    table,
                    &columns,
                    unique,
                )?);
            }
            DeclaredIndex::JsonField(field) => {
                // A field inside the `data` blob is indexed by its expression,
                // never by a column that does not exist. This used to emit a
                // `CREATE INDEX` over a phantom column, which only failed when
                // the migration actually ran.
                statements.extend(ferris_db::sql::ensure_json_index(
                    dialect,
                    &index.name,
                    table,
                    field,
                    unique,
                )?);
            }
            DeclaredIndex::Unsupported => {}
        }
    }
    // `FieldMeta::unique` is a uniqueness **constraint**, so it materializes
    // as a unique index even though the document itself lives in `data`: a
    // real column is indexed directly, any other persisted field through the
    // dialect's JSON expression index (`json_extract` / `->>` / a MySQL
    // generated column). That is what finally gives the flag a reader
    // (LOGIC-01); a field name the dialect cannot quote fails table creation
    // instead of being ignored.
    for field in &meta.fields {
        if !field.unique || field.r#virtual || crate::fieldtype::is_layout(&field.fieldtype) {
            continue;
        }
        let name = format!("{}_unique", field.fieldname);
        if is_indexable_column(&field.fieldname) {
            statements.push(ferris_db::sql::ensure_index(
                dialect,
                &name,
                table,
                &[&field.fieldname],
                true,
            )?);
        } else {
            statements.extend(ferris_db::sql::ensure_json_index(
                dialect,
                &name,
                table,
                &field.fieldname,
                true,
            )?);
        }
    }
    Ok(statements)
}
