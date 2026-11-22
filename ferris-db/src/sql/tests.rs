//! Unit tests for the SQL builders.
//!
//! They were inline `#[cfg(test)] mod tests` before commit 2013f3d moved
//! them into `tests/`, where `crate::sql::…` no longer names this crate.
//! Restored here so the internal paths they use resolve again.

use super::*;
use crate::dialect::Dialect;
use ferris_core::Value;

fn col(name: &str, ty: &str) -> Column {
    Column::new(name, ty)
}

fn key(name: &str, ty: &str) -> Column {
    col(name, ty).primary_key()
}

#[test]
fn ident_validates_and_quotes() {
    let id = Ident::new("tabToDo").unwrap();
    assert_eq!(id.quote(Dialect::Postgres), "\"tabToDo\"");
    assert_eq!(id.quote(Dialect::MySQL), "`tabToDo`");
    assert!(Ident::new("bad name").is_err());
    assert!(Ident::new("a;DROP TABLE t").is_err());
    assert!(Ident::new("").is_err());
}

#[test]
fn qualified_ident_quotes_each_segment() {
    let id = Ident::qualified("tabToDo.owner").unwrap();
    assert_eq!(id.quote(Dialect::Postgres), "\"tabToDo\".\"owner\"");
    assert_eq!(id.quote(Dialect::MySQL), "`tabToDo`.`owner`");
    assert_eq!(id.column(), "owner");
    assert!(Ident::qualified("tabToDo.").is_err());
}

#[test]
fn ident_pair_builds_a_qualified_name() {
    let id = Ident::pair("tabToDo", "owner").unwrap();
    assert_eq!(id.as_str(), "tabToDo.owner");
    assert_eq!(id.quote(Dialect::SQLite), "\"tabToDo\".\"owner\"");
}

#[test]
fn upsert_uses_on_conflict_off_postgres_and_sqlite() {
    for dialect in [Dialect::Postgres, Dialect::SQLite] {
        let built = UpsertBuilder::new(dialect, "sessions")
            .values(vec![
                ("id", Value::string("s1")),
                ("data", Value::string("{}")),
            ])
            .conflict_on(["id"])
            .on_update("data", UpsertSet::Value(Value::string("{}")))
            .build()
            .unwrap();
        assert_eq!(
            built.sql(),
            "INSERT INTO \"sessions\" (\"id\", \"data\") VALUES (?, ?) \
                 ON CONFLICT (\"id\") DO UPDATE SET \"data\" = ?"
        );
        assert_eq!(built.params().len(), 3);
    }
}

#[test]
fn upsert_uses_on_duplicate_key_update_off_mysql() {
    let built = UpsertBuilder::new(Dialect::MySQL, "sessions")
        .values(vec![
            ("id", Value::string("s1")),
            ("data", Value::string("{}")),
        ])
        .conflict_on(["id"])
        .on_update("data", UpsertSet::Value(Value::string("{}")))
        .build()
        .unwrap();
    assert_eq!(
        built.sql(),
        "INSERT INTO `sessions` (`id`, `data`) VALUES (?, ?) \
             ON DUPLICATE KEY UPDATE `data` = ?"
    );
    assert_eq!(built.params().len(), 3);
}

#[test]
fn upsert_supports_counters_and_excluded_references() {
    let built = UpsertBuilder::new(Dialect::Postgres, "file_refs")
        .values(vec![
            ("id", Value::string("f1")),
            ("version", Value::from(1)),
        ])
        .conflict_on(["id"])
        .on_update("version", UpsertSet::Increment(Value::from(1)))
        .on_update("id", UpsertSet::FromExcluded("id".into()))
        .build()
        .unwrap();
    assert!(built.sql().contains("\"version\" = \"version\" + ?"));
    assert!(built.sql().contains("\"id\" = excluded.id"));
}

#[test]
fn upsert_rejects_an_empty_insert_or_missing_conflict_target() {
    assert!(UpsertBuilder::new(Dialect::Postgres, "t").build().is_err());
    // Postgres/SQLite name the conflict target, so omitting it is an error.
    assert!(
        UpsertBuilder::new(Dialect::Postgres, "t")
            .values(vec![("id", Value::from(1))])
            .build()
            .is_err()
    );
    // MySQL needs no target, so the same statement is valid.
    assert!(
        UpsertBuilder::new(Dialect::MySQL, "t")
            .values(vec![("id", Value::from(1))])
            .build()
            .is_ok()
    );
}

/// Names are `&str` so a statement reads as data, but an invalid one is
/// still rejected before anything is sent.
#[test]
fn a_do_nothing_upsert_is_spelled_per_dialect() {
    for dialect in [Dialect::Postgres, Dialect::SQLite] {
        let built = UpsertBuilder::new(dialect, "tab_series")
            .values(vec![
                ("name", Value::from("TODO-.#####")),
                ("current", Value::from(0)),
            ])
            .conflict_on(["name"])
            .on_conflict_do_nothing()
            .build()
            .unwrap();
        assert_eq!(
            built.sql(),
            "INSERT INTO \"tab_series\" (\"name\", \"current\") VALUES (?, ?) \
                 ON CONFLICT (\"name\") DO NOTHING"
        );
    }

    // MySQL spells the same intent as `INSERT IGNORE`, and needs no target.
    let mysql = UpsertBuilder::new(Dialect::MySQL, "tab_series")
        .values(vec![
            ("name", Value::from("TODO-.#####")),
            ("current", Value::from(0)),
        ])
        .on_conflict_do_nothing()
        .build()
        .unwrap();
    assert_eq!(
        mysql.sql(),
        "INSERT IGNORE INTO `tab_series` (`name`, `current`) VALUES (?, ?)"
    );
    assert!(mysql.sql().to_ascii_uppercase().contains("ON DUPLICATE") == false);

    // Ignoring a conflict and assigning one are contradictory.
    assert!(
        UpsertBuilder::new(Dialect::Postgres, "t")
            .values(vec![("id", Value::from(1))])
            .conflict_on(["id"])
            .on_conflict_do_nothing()
            .on_update("id", UpsertSet::Value(Value::from(1)))
            .build()
            .is_err()
    );
}

#[test]
fn upsert_and_ddl_reject_an_unusable_identifier() {
    assert!(
        UpsertBuilder::new(Dialect::Postgres, "sessions; DROP TABLE users")
            .values(vec![("id", Value::from(1))])
            .conflict_on(["id"])
            .build()
            .is_err()
    );
    assert!(
        UpsertBuilder::new(Dialect::Postgres, "sessions")
            .values(vec![("id); DROP TABLE users", Value::from(1))])
            .conflict_on(["id"])
            .build()
            .is_err()
    );
    assert!(create_table(Dialect::Postgres, "bad name", &[col("id", "INT")]).is_err());
    assert!(create_table(Dialect::Postgres, "t", &[col("bad name", "INT")]).is_err());
    assert!(ensure_index(Dialect::Postgres, "idx", "tabUser", &["bad name"], false).is_err());
}

#[test]
fn create_table_quotes_reserved_column_names() {
    let columns = vec![
        Column::new("id", "VARCHAR(255)").not_null(),
        Column::new("user", "VARCHAR(255)").not_null(),
    ];
    let sql = create_table(Dialect::MySQL, "tab_api_key", &columns).unwrap();
    assert_eq!(
        sql,
        "CREATE TABLE IF NOT EXISTS `tab_api_key` (`id` VARCHAR(255) NOT NULL, \
             `user` VARCHAR(255) NOT NULL)"
    );
}

#[test]
fn create_table_widens_text_defaults_for_mysql_only() {
    let columns = vec![
        col("id", "VARCHAR(255)"),
        Column::new("data", "TEXT").default("'{}'"),
    ];

    let mysql = create_table(Dialect::MySQL, "t", &columns).unwrap();
    assert!(mysql.contains("`data` VARCHAR(255) DEFAULT '{}'"));

    // Postgres and SQLite accept a default on TEXT.
    let pg = create_table(Dialect::Postgres, "t", &columns).unwrap();
    assert!(pg.contains("\"data\" TEXT DEFAULT '{}'"));

    // A TEXT column with no default stays TEXT everywhere.
    let no_default = vec![col("id", "VARCHAR(255)"), col("data", "TEXT")];
    assert!(
        create_table(Dialect::MySQL, "t", &no_default)
            .unwrap()
            .contains("`data` TEXT")
    );
}

#[test]
fn ensure_index_is_idempotent_where_supported_and_plain_on_mysql() {
    assert_eq!(
        ensure_index(Dialect::Postgres, "idx_owner", "tabUser", &["owner"], false).unwrap(),
        "CREATE INDEX IF NOT EXISTS \"idx_tabUser_owner\" ON \"tabUser\" (\"owner\")"
    );
    assert_eq!(
        ensure_index(Dialect::MySQL, "idx_owner", "tabUser", &["owner"], false).unwrap(),
        "CREATE INDEX `idx_tabUser_owner` ON `tabUser` (`owner`)"
    );
    assert_eq!(
        ensure_index(Dialect::SQLite, "idx_owner", "tabUser", &["owner"], true).unwrap(),
        "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_tabUser_owner\" ON \"tabUser\" (\"owner\")"
    );
    assert!(ensure_index(Dialect::Postgres, "idx", "tabUser", &[], false).is_err());
}

/// SCH-03: Postgres and SQLite keep index names in the schema namespace, so
/// two tables asking for `idx_owner` would get one index between them. The
/// rendered name must carry its table, and an already-qualified name must not
/// be prefixed twice.
#[test]
fn index_names_are_qualified_with_their_table() {
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let alpha = ensure_index(dialect, "idx_owner", "tabAlpha", &["owner"], false).unwrap();
        let beta = ensure_index(dialect, "idx_owner", "tabBeta", &["owner"], false).unwrap();
        assert!(alpha.contains("idx_tabAlpha_owner"), "{dialect:?}: {alpha}");
        assert!(beta.contains("idx_tabBeta_owner"), "{dialect:?}: {beta}");
        assert_ne!(alpha, beta);
        assert_eq!(
            ensure_index(dialect, "idx_tabAlpha_owner", "tabAlpha", &["owner"], false).unwrap(),
            alpha,
            "{dialect:?}: a pre-qualified name must pass through unchanged"
        );
    }
}

#[test]
fn add_column_renders_per_dialect_and_widens_text_defaults() {
    let simple = Column::new("mfa_secret", "TEXT").default("''");
    assert_eq!(
        add_column(Dialect::SQLite, "users", &simple).unwrap(),
        "ALTER TABLE \"users\" ADD COLUMN \"mfa_secret\" TEXT DEFAULT ''"
    );
    // MySQL rejects a DEFAULT on TEXT, so the type is widened.
    assert_eq!(
        add_column(Dialect::MySQL, "users", &simple).unwrap(),
        "ALTER TABLE `users` ADD COLUMN `mfa_secret` VARCHAR(255) DEFAULT ''"
    );
    let int = Column::new("mfa_enabled", "INTEGER")
        .not_null()
        .default("0");
    assert_eq!(
        add_column(Dialect::Postgres, "users", &int).unwrap(),
        "ALTER TABLE \"users\" ADD COLUMN \"mfa_enabled\" INTEGER NOT NULL DEFAULT 0"
    );
    assert!(add_column(Dialect::Postgres, "bad name", &col("x", "INT")).is_err());
}

#[test]
fn create_table_rejects_an_empty_column_list() {
    assert!(create_table(Dialect::Postgres, "t", &[]).is_err());
}

/// An upsert conflict target must name a unique constraint, so the key has
/// to be expressible through the same helper that creates the table.
#[test]
fn create_table_emits_single_and_composite_primary_keys() {
    let single = create_table(
        Dialect::Postgres,
        "sessions",
        &[key("id", "VARCHAR(255)"), col("data", "TEXT")],
    )
    .unwrap();
    assert_eq!(
        single,
        "CREATE TABLE IF NOT EXISTS \"sessions\" (\"id\" VARCHAR(255), \"data\" TEXT, \
             PRIMARY KEY (\"id\"))"
    );

    // A composite key cannot be written inline, which is why the key is
    // always emitted as a table constraint.
    let composite = create_table(
        Dialect::SQLite,
        "_ferris_search",
        &[
            key("doctype", "VARCHAR(255)").not_null(),
            key("name", "VARCHAR(255)").not_null(),
            col("doc", "TEXT"),
        ],
    )
    .unwrap();
    assert_eq!(
        composite,
        "CREATE TABLE IF NOT EXISTS \"_ferris_search\" (\"doctype\" VARCHAR(255) NOT NULL, \
             \"name\" VARCHAR(255) NOT NULL, \"doc\" TEXT, PRIMARY KEY (\"doctype\", \"name\"))"
    );
}

#[test]
fn a_surrogate_key_is_rendered_per_dialect() {
    let columns = || {
        vec![
            Column::new("id", "INTEGER").auto_increment(),
            col("value", "TEXT").not_null(),
        ]
    };

    // SQLite only auto-fills an inline `INTEGER PRIMARY KEY AUTOINCREMENT`.
    assert_eq!(
        create_table(Dialect::SQLite, "t", &columns()).unwrap(),
        "CREATE TABLE IF NOT EXISTS \"t\" (\"id\" INTEGER PRIMARY KEY AUTOINCREMENT, \
             \"value\" TEXT NOT NULL)"
    );
    assert_eq!(
        create_table(Dialect::Postgres, "t", &columns()).unwrap(),
        "CREATE TABLE IF NOT EXISTS \"t\" (\"id\" SERIAL PRIMARY KEY, \
             \"value\" TEXT NOT NULL)"
    );
    assert_eq!(
        create_table(Dialect::MySQL, "t", &columns()).unwrap(),
        "CREATE TABLE IF NOT EXISTS `t` (`id` INTEGER NOT NULL AUTO_INCREMENT PRIMARY KEY, \
             `value` TEXT NOT NULL)"
    );
}

#[test]
fn a_surrogate_key_must_be_a_single_integer_column() {
    let non_integer = vec![Column::new("id", "VARCHAR(255)").auto_increment()];
    assert!(create_table(Dialect::Postgres, "t", &non_integer).is_err());

    let two = vec![
        Column::new("id", "INTEGER").auto_increment(),
        Column::new("seq", "INTEGER").auto_increment(),
    ];
    assert!(create_table(Dialect::Postgres, "t", &two).is_err());
}

#[test]
fn create_table_rejects_a_duplicated_column() {
    let columns = vec![col("id", "VARCHAR(255)"), col("id", "VARCHAR(255)")];
    assert!(create_table(Dialect::Postgres, "t", &columns).is_err());
}

// ---------------------------------------------------------------------
// R1 — JSON expression indexes for blob-backed document fields
// ---------------------------------------------------------------------

/// `json_field_target` is the *one* spelling of `data.field` shared by the
/// index DDL and the queries that must use it, per dialect: `json_extract`
/// on SQLite, `jsonb ->>` on Postgres, and the generated mirror column on
/// MySQL.
#[test]
fn json_field_target_is_dialect_specific_and_validated() {
    assert_eq!(
        json_field_target(Dialect::SQLite, "device_id").unwrap(),
        "json_extract(\"data\", '$.device_id')"
    );
    assert_eq!(
        json_field_target(Dialect::Postgres, "device_id").unwrap(),
        "(\"data\"::jsonb ->> 'device_id')"
    );
    assert_eq!(
        json_field_target(Dialect::MySQL, "device_id").unwrap(),
        "`g_device_id`"
    );

    // A field that could never exist in the blob is rejected, not emitted:
    // the same name would also fail the column generation below, so no
    // query and no DDL can disagree about it.
    assert!(json_field_target(Dialect::SQLite, "bad name").is_err());
    assert!(json_field_target(Dialect::SQLite, "").is_err());
    assert!(json_field_column("bad name").is_err());
}

/// The MySQL mirror column derives deterministically from the field, so the
/// DDL and the query can never drift apart.
#[test]
fn json_field_column_is_deterministic_and_prefixed() {
    assert_eq!(json_field_column("device_id").unwrap(), "g_device_id");
    assert_eq!(json_field_column("ts").unwrap(), "g_ts");
    assert_ne!(
        json_field_column("a").unwrap(),
        json_field_column("b").unwrap()
    );
}

/// `ensure_json_index` renders one expression index on SQLite/Postgres and
/// a generated column plus an index on MySQL — always against the exact
/// target [`json_field_target`] produces.
#[test]
fn ensure_json_index_renders_per_dialect() {
    let sqlite = ensure_json_index(
        Dialect::SQLite,
        "idx_reads_device",
        "tabSolmecRead",
        "device_id",
        false,
    )
    .unwrap();
    assert_eq!(
        sqlite,
        vec![
            "CREATE INDEX IF NOT EXISTS \"idx_tabSolmecRead_reads_device\" ON \"tabSolmecRead\" \
                  (json_extract(\"data\", '$.device_id'))"
        ]
    );

    let pg = ensure_json_index(
        Dialect::Postgres,
        "idx_reads_device",
        "tabSolmecRead",
        "device_id",
        true,
    )
    .unwrap();
    assert_eq!(
        pg,
        vec![
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_tabSolmecRead_reads_device\" ON \"tabSolmecRead\" \
                  ((\"data\"::jsonb ->> 'device_id'))"
        ]
    );

    let mysql = ensure_json_index(
        Dialect::MySQL,
        "idx_reads_device",
        "tabSolmecRead",
        "device_id",
        false,
    )
    .unwrap();
    assert_eq!(mysql.len(), 2, "{mysql:?}");
    assert!(
        mysql[0].contains("ADD COLUMN `g_device_id` VARBINARY(255) GENERATED ALWAYS AS"),
        "{}",
        mysql[0]
    );
    assert!(mysql[0].contains("BINARY json_unquote(json_extract(`data`, '$.device_id'))"));
    assert_eq!(
        mysql[1],
        "CREATE INDEX `idx_tabSolmecRead_reads_device` ON `tabSolmecRead` (`g_device_id`)"
    );

    // The query side names the same column the DDL created.
    assert_eq!(
        json_field_target(Dialect::MySQL, "device_id").unwrap(),
        "`g_device_id`"
    );
}

/// Both DDL shapes validate their identifiers; a field that would not
/// survive quoting fails before any statement is produced.
#[test]
fn ensure_json_index_validates_its_names() {
    assert!(ensure_json_index(Dialect::SQLite, "idx", "tabX", "bad name", false).is_err());
    assert!(ensure_json_index(Dialect::SQLite, "bad name", "tabX", "device_id", false).is_err());
    assert!(ensure_json_index(Dialect::SQLite, "idx", "bad table", "device_id", false).is_err());
    assert!(ensure_json_index(Dialect::MySQL, "idx", "tabX", "", false).is_err());
}
