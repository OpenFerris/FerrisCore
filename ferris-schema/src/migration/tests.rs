use super::runner::create_table_statements;
use super::*;
use crate::registry::{DoctypeMeta, FieldMeta, IndexMeta};
use ferris_db::Dialect;

/// The table name must be derived the same way by every caller, or the
/// engine and the migration runner create two physical tables — which is
/// what `tabToDo` vs `"tabToDo"` did on Postgres.
#[test]
fn document_table_strips_whitespace_and_validates() {
    assert_eq!(document_table("ToDo").unwrap(), "tabToDo");
    assert_eq!(document_table("To Do").unwrap(), "tabToDo");
    assert_eq!(document_table("User").unwrap(), "tabUser");

    // Anything that would not survive quoting is rejected rather than
    // emitted and dropped at the database.
    assert!(document_table("").is_err());
    assert!(document_table("   ").is_err());
    assert!(document_table("ToDo; DROP TABLE users").is_err());
    assert!(document_table("tab-User").is_err());

    // The prefix is added, not detected, so a DocType already called `tab`
    // gets `tabtab`. Callers derive the name from metadata, never from a
    // stored identifier, so nothing can end up with two spellings.
    assert_eq!(document_table("tab").unwrap(), "tabtab");
}

#[test]
fn document_columns_are_one_definition_for_every_dialect() {
    let columns = document_columns();
    let sqlite = ferris_db::sql::create_table(Dialect::SQLite, "tabToDo", &columns).unwrap();
    let mysql = ferris_db::sql::create_table(Dialect::MySQL, "tabToDo", &columns).unwrap();

    // Columns are the only source of truth: there is no JSON blob.
    assert!(!sqlite.contains("\"data\""));
    assert!(!mysql.contains("`data`"));

    // The engine-stamped columns every table owns, including the three that
    // used to live only inside the blob.
    for column in ["modified_by", "amended_from", "workflow_state"] {
        assert!(
            sqlite.contains(&format!("\"{column}\" VARCHAR(255)")),
            "{sqlite}"
        );
        assert!(
            mysql.contains(&format!("`{column}` VARCHAR(255)")),
            "{mysql}"
        );
    }

    // `name` is the key, so an upsert on it has a unique constraint.
    assert!(sqlite.contains("PRIMARY KEY (\"name\")"));
}

#[test]
fn document_indexes_are_valid_ddl_on_every_dialect() {
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        for (name, columns, unique) in document_indexes() {
            let sql = ferris_db::sql::ensure_index(dialect, name, "tabToDo", columns, unique)
                .unwrap_or_else(|e| panic!("{dialect:?} index {name}: {e}"));
            // The logical name is qualified with the table before rendering.
            assert!(
                sql.contains(&format!("idx_tabToDo_{}", name.trim_start_matches("idx_"))),
                "{dialect:?}: {sql}"
            );
        }
    }
}

/// SCH-03 / PERF-16: Postgres and SQLite index names are shared by every table
/// in the schema, so the old fixed `idx_owner`/`idx_modified`/`idx_docstatus`
/// gave the index to the first DocType table and silently skipped the rest.
/// Each table must get its own three `CREATE INDEX` statements under distinct
/// names.
#[test]
fn standard_indexes_are_named_per_table() {
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let alpha = create_table_statements(dialect, "tabAlpha", &DoctypeMeta::default()).unwrap();
        let beta = create_table_statements(dialect, "tabBeta", &DoctypeMeta::default()).unwrap();

        for (statements, table) in [(&alpha, "tabAlpha"), (&beta, "tabBeta")] {
            for suffix in ["owner", "modified", "docstatus", "owner_modified"] {
                let name = format!("idx_{table}_{suffix}");
                assert!(
                    statements.iter().any(|s| s.contains(&name)),
                    "{dialect:?}: {table} is missing {name}: {statements:?}"
                );
            }
        }

        let indexes = |statements: &[String]| -> Vec<String> {
            statements
                .iter()
                .filter(|s| s.contains(" INDEX "))
                .cloned()
                .collect()
        };
        let alpha_indexes = indexes(&alpha);
        let beta_indexes = indexes(&beta);
        assert_eq!(alpha_indexes.len(), 4, "{dialect:?}: {alpha:?}");
        assert_eq!(beta_indexes.len(), 4, "{dialect:?}: {beta:?}");
        assert!(
            alpha_indexes.iter().all(|s| !beta_indexes.contains(s)),
            "{dialect:?}: the two tables share an index statement"
        );
    }
}

fn unique_meta(name: &str, field: &str) -> DoctypeMeta {
    DoctypeMeta {
        name: name.to_string(),
        fields: vec![FieldMeta {
            fieldname: field.to_string(),
            fieldtype: "Data".to_string(),
            unique: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// LOGIC-01: `FieldMeta::unique` used to have no reader at all. A field with a
/// real column gets a `UNIQUE` index over that column, and the name carries
/// the table so two DocTypes enforcing the same field do not collide.
#[test]
fn unique_fields_on_real_columns_generate_per_table_unique_indexes() {
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let alpha =
            create_table_statements(dialect, "tabAlpha", &unique_meta("Unique Alpha", "owner"))
                .unwrap();
        let beta =
            create_table_statements(dialect, "tabBeta", &unique_meta("Unique Beta", "owner"))
                .unwrap();

        for (statements, table) in [(&alpha, "tabAlpha"), (&beta, "tabBeta")] {
            let unique = statements
                .iter()
                .find(|s| s.contains("CREATE UNIQUE INDEX") && s.contains(&format!("idx_{table}_")))
                .unwrap_or_else(|| {
                    panic!("{dialect:?}: no unique index for {table}: {statements:?}")
                });
            assert!(
                unique.contains(&format!("idx_{table}_owner_unique")),
                "{dialect:?}: {unique}"
            );
            assert!(unique.contains("(\"owner\")") || unique.contains("(`owner`)"));
        }
    }
}

/// LOGIC-01: a unique field lives in the `data` blob, so each dialect indexes
/// its expression: `json_extract` on SQLite, `data->>` on Postgres, and a
/// generated column on MySQL. The uniqueness flag and the per-table name must
/// survive on all three.
#[test]
fn unique_blob_fields_generate_unique_expression_indexes() {
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let alpha =
            create_table_statements(dialect, "tabAlpha", &unique_meta("Unique Alpha", "code"))
                .unwrap();
        let beta = create_table_statements(dialect, "tabBeta", &unique_meta("Unique Beta", "code"))
            .unwrap();

        for (statements, table) in [(&alpha, "tabAlpha"), (&beta, "tabBeta")] {
            let name = format!("idx_{table}_code_unique");
            let joined = statements.join("\n");
            assert!(joined.contains(&name), "{dialect:?}: {joined}");
            assert!(
                joined.contains("CREATE UNIQUE INDEX"),
                "{dialect:?}: {joined}"
            );
            match dialect {
                Dialect::SQLite => assert!(
                    joined.contains(&format!(
                        "CREATE UNIQUE INDEX IF NOT EXISTS \"{name}\" ON \"{table}\" \
                         (json_extract(\"data\", '$.code'))"
                    )),
                    "{joined}"
                ),
                Dialect::Postgres => assert!(
                    joined.contains(&format!(
                        "CREATE UNIQUE INDEX IF NOT EXISTS \"{name}\" ON \"{table}\" \
                         ((\"data\"::jsonb ->> 'code'))"
                    )),
                    "{joined}"
                ),
                Dialect::MySQL => {
                    assert!(joined.contains("ADD COLUMN `g_code`"), "{joined}");
                    assert!(
                        joined.contains(&format!(
                            "CREATE UNIQUE INDEX `{name}` ON `{table}` (`g_code`)"
                        )),
                        "{joined}"
                    );
                }
            }
        }
    }
}

/// A declared `IndexMeta` keeps its `UNIQUE` and is namespaced per table, so
/// the same declaration on two DocTypes does not collapse into one index.
#[test]
fn declared_unique_indexes_keep_uniqueness_per_table() {
    let meta = |name: &str| DoctypeMeta {
        name: name.to_string(),
        indexes: vec![IndexMeta {
            name: "idx_owner".to_string(),
            fields: vec!["owner".to_string()],
            unique: true,
            index_type: 0,
        }],
        ..Default::default()
    };
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let alpha = create_table_statements(dialect, "tabAlpha", &meta("Alpha")).unwrap();
        let beta = create_table_statements(dialect, "tabBeta", &meta("Beta")).unwrap();
        let alpha_sql = alpha.join("\n");
        let beta_sql = beta.join("\n");
        assert!(
            alpha_sql.contains("CREATE UNIQUE INDEX") && alpha_sql.contains("idx_tabAlpha_owner"),
            "{dialect:?}: {alpha_sql}"
        );
        assert!(
            beta_sql.contains("CREATE UNIQUE INDEX") && beta_sql.contains("idx_tabBeta_owner"),
            "{dialect:?}: {beta_sql}"
        );
    }
}

/// D4 / S7: a DocType's migration version must depend only on its own name,
/// so inserting or removing *another* DocType leaves it untouched.
///
/// The previous positional counter failed this: adding a name that sorted
/// earlier shifted every later version, which made `MigrationRunner::migrate`
/// reject an already-migrated database on a checksum mismatch. This test
/// registers a DocType, snapshots its version, then registers a second name
/// that sorts *before* it and asserts the first version did not move.
#[tokio::test]
async fn a_doctypes_version_does_not_depend_on_the_registry_set() {
    crate::registry::register_doctype(DoctypeMeta {
        name: "Mosaic".to_string(),
        module: "d4test".to_string(),
        ..Default::default()
    });

    let db = ferris_db::memory_db().await.unwrap();
    let mut before = MigrationRunner::for_db(&db);
    before.add_schema_migrations().unwrap();
    let mosaic = |runner: &MigrationRunner| {
        runner
            .db_runner
            .migrations()
            .iter()
            .find(|m| m.name == "create_table_mosaic")
            .map(|m| m.version)
    };
    let version_before = mosaic(&before).expect("Mosaic migration present");

    // A name sorting before "Mosaic" — the case that used to shift it.
    crate::registry::register_doctype(DoctypeMeta {
        name: "Mango".to_string(),
        module: "d4test".to_string(),
        ..Default::default()
    });

    let mut after = MigrationRunner::for_db(&db);
    after.add_schema_migrations().unwrap();
    assert_eq!(
        mosaic(&after),
        Some(version_before),
        "adding an earlier-sorting DocType shifted Mosaic's version — \
         the checksum guard would now reject every migrated database"
    );

    // And the version is exactly the name-derived one, so it is stable
    // across processes and Rust versions (`DefaultHasher` would not be).
    assert_eq!(version_before, schema_version("Mosaic"));
    assert_eq!(schema_version("Mosaic"), schema_version("Mosaic"));

    // The runner's own ordering invariant still holds for the whole set.
    after.db_runner.validate().unwrap();

    // The original M2 bug: no two distinct names share a version here.
    let names = ["Mosaic", "Mango", "Apple", "Zebra", "ToDo", "Custom Field"];
    let mut versions: Vec<u32> = names.iter().map(|n| schema_version(n)).collect();
    versions.sort_unstable();
    versions.dedup();
    assert_eq!(
        versions.len(),
        names.len(),
        "version collision: {versions:?}"
    );
}

/// The whole point of [`document_columns`]: the lazily-created table and the
/// migrated table are byte-identical.
#[test]
fn a_migration_creates_exactly_the_engines_table() {
    let meta = DoctypeMeta {
        name: "Test Doc".to_string(),
        fields: vec![FieldMeta {
            fieldname: "title".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        indexes: vec![IndexMeta {
            name: "idx_title".to_string(),
            fields: vec!["title".to_string()],
            unique: true,
            index_type: 0,
        }],
        ..Default::default()
    };
    let table = document_table(&meta.name).unwrap();

    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let statements = create_table_statements(dialect, &table, &meta).unwrap();
        let engine = ferris_db::sql::create_table(dialect, &table, &document_columns()).unwrap();
        assert_eq!(
            statements.first().map(String::as_str),
            Some(engine.as_str()),
            "{dialect:?}: the migration's table differs from the engine's"
        );
    }
}

// ---------------------------------------------------------------------
// R1 — declared indexes against a blob-backed table
// ---------------------------------------------------------------------

#[test]
fn every_system_column_is_indexable_and_other_names_are_not() {
    for field in [
        "name",
        "owner",
        "docstatus",
        "creation",
        "modified",
        "modified_by",
        "amended_from",
        "workflow_state",
    ] {
        assert!(is_indexable_column(field), "{field} is a column");
    }
    // A declared field has a column only when the DocType declares it, which
    // this fixed list cannot know; an unknown name is not a system column.
    for field in ["data", "device_id", "title", ""] {
        assert!(
            !is_indexable_column(field),
            "{field} is not a system column"
        );
    }
}

/// `classify_index` decides how a declaration materializes. A declaration
/// naming only real fields is a plain column index; a single blob field is
/// the JSON expression index; anything else — a composite with a blob
/// member, or an empty list — is skipped rather than emitted as DDL that
/// references a phantom column.
#[test]
fn classify_index_splits_columns_from_unsupported_shapes() {
    let index = |fields: &[&str]| IndexMeta {
        name: "idx".to_string(),
        fields: fields.iter().map(|s| s.to_string()).collect(),
        unique: false,
        index_type: 0,
    };

    assert!(matches!(
        classify_index(&index(&["owner"])),
        DeclaredIndex::Columns
    ));
    assert!(matches!(
        classify_index(&index(&["owner", "modified"])),
        DeclaredIndex::Columns
    ));
    // `device_id` is not a system column: the runner cannot classify a
    // declared field here, so it stays unsupported rather than inventing a
    // JSON expression over a column that does not exist.
    assert!(matches!(
        classify_index(&index(&["device_id"])),
        DeclaredIndex::Unsupported
    ));
    assert!(matches!(
        classify_index(&index(&["owner", "device_id"])),
        DeclaredIndex::Unsupported
    ));
    assert!(matches!(
        classify_index(&index(&["device_id", "status"])),
        DeclaredIndex::Unsupported
    ));
    assert!(matches!(
        classify_index(&index(&[])),
        DeclaredIndex::Unsupported
    ));
}

/// A declaration that names a field with no column is skipped now that there
/// is no `data` blob: it must not emit a JSON expression index over a column
/// the table does not have.
#[test]
fn a_declared_index_over_a_missing_column_is_skipped() {
    let meta = DoctypeMeta {
        name: "SolmecRead".to_string(),
        indexes: vec![IndexMeta {
            name: "idx_reads_device".to_string(),
            fields: vec!["device_id".to_string()],
            unique: false,
            index_type: 0,
        }],
        ..Default::default()
    };
    let table = document_table(&meta.name).unwrap();

    for dialect in [Dialect::SQLite, Dialect::Postgres, Dialect::MySQL] {
        let statements = create_table_statements(dialect, &table, &meta).unwrap();
        let joined = statements.join("\n");
        assert!(!joined.contains("json_extract"), "{dialect:?}: {joined}");
        assert!(
            !joined.contains("idx_reads_device"),
            "{dialect:?}: {joined}"
        );
        assert!(
            !joined.contains("(\"device_id\")") && !joined.contains("(`device_id`)"),
            "{dialect:?}: {joined}"
        );
    }
}
