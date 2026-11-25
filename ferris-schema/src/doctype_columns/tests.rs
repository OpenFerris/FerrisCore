use super::*;

fn field(fieldname: &str, fieldtype: &str) -> FieldMeta {
    FieldMeta::new(fieldname, fieldtype)
}

fn registered(name: &str, fields: Vec<FieldMeta>) -> DoctypeMeta {
    let meta = DoctypeMeta {
        name: name.to_string(),
        module: "phase1columns".to_string(),
        fields,
        ..Default::default()
    };
    crate::registry::register_doctype(meta.clone());
    meta
}

/// Every mapping, on every dialect. A type is wrong if it changes without
/// this table changing.
#[test]
fn every_fieldtype_maps_on_every_dialect() {
    let cases: &[(&str, &str, &str, &str)] = &[
        ("Data", "VARCHAR(255)", "VARCHAR(255)", "VARCHAR(255)"),
        ("Select", "VARCHAR(255)", "VARCHAR(255)", "VARCHAR(255)"),
        ("Link", "VARCHAR(255)", "VARCHAR(255)", "VARCHAR(255)"),
        (
            "Dynamic Link",
            "VARCHAR(255)",
            "VARCHAR(255)",
            "VARCHAR(255)",
        ),
        ("Attach", "VARCHAR(255)", "VARCHAR(255)", "VARCHAR(255)"),
        (
            "Table MultiSelect",
            "VARCHAR(255)",
            "VARCHAR(255)",
            "VARCHAR(255)",
        ),
        ("Small Text", "TEXT", "TEXT", "TEXT"),
        ("Text", "TEXT", "TEXT", "TEXT"),
        ("Long Text", "TEXT", "TEXT", "TEXT"),
        ("Text Editor", "TEXT", "TEXT", "TEXT"),
        ("Code", "TEXT", "TEXT", "TEXT"),
        ("JSON", "TEXT", "TEXT", "TEXT"),
        ("Int", "BIGINT", "BIGINT", "BIGINT"),
        ("Float", "DOUBLE PRECISION", "DOUBLE", "REAL"),
        ("Currency", "NUMERIC(21,9)", "DECIMAL(21,9)", "NUMERIC"),
        ("Percent", "NUMERIC(21,9)", "DECIMAL(21,9)", "NUMERIC"),
        ("Check", "BOOLEAN", "BOOLEAN", "BOOLEAN"),
        ("Date", "DATE", "DATE", "TEXT"),
        ("Datetime", "TIMESTAMP", "DATETIME", "TEXT"),
        ("Password", "TEXT", "TEXT", "TEXT"),
        ("Secret", "TEXT", "TEXT", "TEXT"),
    ];
    for (fieldtype, postgres, mysql, sqlite) in cases {
        for (dialect, expected) in [
            (Dialect::Postgres, postgres),
            (Dialect::MySQL, mysql),
            (Dialect::SQLite, sqlite),
        ] {
            let column = field_column(&field("value", fieldtype), dialect)
                .unwrap_or_else(|error| panic!("{fieldtype} on {dialect:?}: {error}"))
                .unwrap_or_else(|| panic!("{fieldtype} produced no column"));
            assert_eq!(
                column.ty, *expected,
                "{fieldtype} on {dialect:?} should be {expected}"
            );
        }
    }
}

/// Layout breaks and valueless fieldtypes never reach the database, and a
/// virtual field is not stored even when its type is `Data`.
#[test]
fn non_stored_fieldtypes_are_skipped() {
    for fieldtype in NON_STORED_FIELDTYPES {
        for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
            assert!(
                field_column(&field("layout", fieldtype), dialect)
                    .unwrap_or_else(|error| panic!("{fieldtype}: {error}"))
                    .is_none(),
                "{fieldtype} on {dialect:?} must not get a column"
            );
        }
    }
    let virtual_field = FieldMeta::data("computed").virtual_field();
    assert!(
        field_column(&virtual_field, Dialect::Postgres)
            .unwrap()
            .is_none()
    );
}

/// A `reqd` field is `NOT NULL` in a fresh table; a nullable unique column
/// is represented by the flag alone and enforced by an index, not here.
#[test]
fn required_fields_are_not_null() {
    let column = field_column(&FieldMeta::data("title").required(), Dialect::Postgres)
        .unwrap()
        .expect("a Data field has a column");
    assert!(column.not_null);
    let column = field_column(&field("code", "Data"), Dialect::Postgres)
        .unwrap()
        .expect("a Data field has a column");
    assert!(!column.not_null);
}

/// The child table gets Frappe's four link columns plus the child's own
/// fields, with no primary key.
#[test]
fn child_table_columns_include_link_columns_and_child_fields() {
    let child = DoctypeMeta {
        name: "Phase One Child".to_string(),
        module: "phase1columns".to_string(),
        is_child_table: true,
        fields: vec![
            FieldMeta::data("label").required(),
            FieldMeta::int("qty"),
            field("notes", "Text"),
            FieldMeta::table("nested", "Phase One Child"),
        ],
        ..Default::default()
    };
    let columns = child_table_columns(&child, Dialect::Postgres).expect("child columns render");
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "name",
            "owner",
            "docstatus",
            "creation",
            "modified",
            "modified_by",
            "amended_from",
            "workflow_state",
            "parent",
            "parenttype",
            "parentfield",
            "idx",
            "label",
            "qty",
            "notes"
        ],
        "the eight system columns, the four link columns, then the stored fields; \
         Table and layout are skipped"
    );
    assert!(columns[0].primary_key, "the child row has its own name");
    assert!(columns[0].not_null);
    assert!(columns[2].not_null && columns[3].not_null && columns[4].not_null);
    assert!(
        columns[5..8].iter().all(|column| !column.not_null),
        "the three optional system columns are nullable"
    );
    assert!(!columns[8].not_null, "a standalone child row has no parent");
    assert_eq!(columns[8].ty, "VARCHAR(255)");
    assert!(columns[11].not_null, "`idx` is NOT NULL");
    assert_eq!(columns[11].ty, "BIGINT");
    assert_eq!(columns[12].ty, "VARCHAR(255)");
    assert_eq!(columns[13].ty, "BIGINT");
    assert_eq!(columns[14].ty, "TEXT");
}

/// A child field that redeclares a link column is the link column itself, not
/// a duplicate. Child DocTypes declare `parent` as a `Link` routinely.
#[test]
fn child_fields_may_shadow_link_columns() {
    let child = DoctypeMeta {
        name: "Phase One Bad Child".to_string(),
        fields: vec![FieldMeta::data("parent"), FieldMeta::data("title")],
        ..Default::default()
    };
    let columns = child_table_columns(&child, Dialect::Postgres).unwrap();
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names.iter().filter(|name| **name == "parent").count(),
        1,
        "{names:?}"
    );
    assert!(names.contains(&"title"), "{names:?}");
}

/// The full parent + child DDL: system columns, the child's table, and the
/// `(parent, parentfield, idx)` index.
#[test]
fn create_statements_build_parent_and_child_tables() {
    registered(
        "Phase One Child Table",
        vec![
            FieldMeta::data("label").required(),
            FieldMeta::int("qty"),
            FieldMeta::check("enabled"),
        ],
    );
    let parent = DoctypeMeta {
        name: "Phase One Parent".to_string(),
        module: "phase1columns".to_string(),
        fields: vec![
            FieldMeta::data("title").required(),
            FieldMeta::table("items", "Phase One Child Table"),
            FieldMeta::section_break("details"),
        ],
        ..Default::default()
    };

    let statements = create_doc_type_statements(&parent, Dialect::Postgres).expect("DDL renders");
    let joined = statements.join("\n");
    assert!(
        joined.contains("CREATE TABLE IF NOT EXISTS \"tabPhaseOneParent\""),
        "{joined}"
    );
    assert!(
        joined.contains("\"title\" VARCHAR(255) NOT NULL"),
        "{joined}"
    );
    assert!(joined.contains("PRIMARY KEY (\"name\")"), "{joined}");
    assert!(!joined.contains("\"data\""), "there is no data blob column");
    assert!(!joined.contains("details"), "layout fields have no column");

    assert!(
        joined.contains("CREATE TABLE IF NOT EXISTS \"tabPhaseOneChildTable\""),
        "{joined}"
    );
    for fragment in [
        "\"parent\" VARCHAR(255)",
        "\"parenttype\" VARCHAR(255)",
        "\"parentfield\" VARCHAR(255)",
        "\"idx\" BIGINT NOT NULL",
        "\"label\" VARCHAR(255) NOT NULL",
        "\"qty\" BIGINT",
        "\"enabled\" BOOLEAN",
    ] {
        assert!(joined.contains(fragment), "missing {fragment}: {joined}");
    }
    // The child row keeps the document primary key and is also located by its
    // link columns.
    let child_create = statements
        .iter()
        .find(|s| s.contains("tabPhaseOneChildTable") && s.contains("CREATE TABLE"))
        .expect("child CREATE TABLE");
    assert!(child_create.contains("PRIMARY KEY"), "{child_create}");
    assert!(
        joined.contains(
            "CREATE INDEX IF NOT EXISTS \"idx_tabPhaseOneChildTable_parent\" \
             ON \"tabPhaseOneChildTable\" (\"parent\", \"parentfield\", \"idx\")"
        ),
        "{joined}"
    );
}

/// SQLite spells the types differently and MySQL uses backticks; the child
/// link columns still render.
#[test]
fn create_statements_follow_the_dialect() {
    registered("Phase One Dialect Child", vec![FieldMeta::datetime("when")]);
    let parent = DoctypeMeta {
        name: "Phase One Dialect Parent".to_string(),
        fields: vec![FieldMeta::table("rows", "Phase One Dialect Child")],
        ..Default::default()
    };

    let sqlite = create_doc_type_statements(&parent, Dialect::SQLite)
        .expect("SQLite DDL")
        .join("\n");
    assert!(sqlite.contains("\"when\" TEXT"), "{sqlite}");

    let mysql = create_doc_type_statements(&parent, Dialect::MySQL)
        .expect("MySQL DDL")
        .join("\n");
    assert!(mysql.contains("`when` DATETIME"), "{mysql}");
    assert!(mysql.contains("`parent` VARCHAR(255)"), "{mysql}");
}

/// An unregistered child `DocType` leaves its rows in the parent's blob
/// instead of making the whole parent table fail to create.
#[test]
fn a_missing_child_doctype_keeps_its_rows_in_the_blob() {
    let parent = DoctypeMeta {
        name: "Phase One Orphan Parent".to_string(),
        fields: vec![FieldMeta::table("items", "Phase One Missing Child")],
        ..Default::default()
    };
    let statements = create_doc_type_statements(&parent, Dialect::Postgres)
        .expect("the parent table is still created");
    let joined = statements.join("\n");
    assert!(joined.contains("tabPhaseOneOrphanParent"), "{joined}");
    assert!(
        !joined.contains("tabPhaseOneMissingChild"),
        "no child table is invented: {joined}"
    );
}

/// Identifiers are validated before they reach SQL, both the field name and
/// a declared index name.
#[test]
fn invalid_identifiers_are_rejected() {
    let error = field_column(&field("bad-name", "Data"), Dialect::Postgres).unwrap_err();
    assert!(error.to_string().contains("Invalid identifier"), "{error}");

    let parent = DoctypeMeta {
        name: "Bad; Parent".to_string(),
        ..Default::default()
    };
    assert!(create_doc_type_statements(&parent, Dialect::Postgres).is_err());

    let parent = DoctypeMeta {
        name: "Phase One Bad Index".to_string(),
        indexes: vec![crate::registry::IndexMeta {
            name: "idx bad".to_string(),
            fields: vec!["title".to_string()],
            ..Default::default()
        }],
        fields: vec![FieldMeta::data("title")],
        ..Default::default()
    };
    assert!(create_doc_type_statements(&parent, Dialect::Postgres).is_err());
}

/// A field marked `unique` and also present in `meta.indexes` is one
/// constraint, emitted once.
#[test]
fn unique_is_emitted_once() {
    let parent = DoctypeMeta {
        name: "Phase One Unique".to_string(),
        fields: vec![FieldMeta::data("code").unique()],
        indexes: vec![crate::registry::IndexMeta {
            name: "idx_code".to_string(),
            fields: vec!["code".to_string()],
            unique: true,
            index_type: 0,
        }],
        ..Default::default()
    };
    for dialect in [Dialect::Postgres, Dialect::MySQL, Dialect::SQLite] {
        let statements = create_doc_type_statements(&parent, dialect).expect("DDL renders");
        let unique = statements
            .iter()
            .filter(|s| s.contains("CREATE UNIQUE INDEX"))
            .count();
        assert_eq!(unique, 1, "{dialect:?}: {statements:?}");
        assert!(
            statements
                .join("\n")
                .contains("idx_tabPhaseOneUnique_code_unique"),
            "{dialect:?}: {statements:?}"
        );
    }
}

/// An index over a name that is not a real column is skipped, not emitted
/// against a phantom column. `data` is a column but holds the whole
/// document, so it is not a legal index target either.
#[test]
fn declared_indexes_must_name_real_columns() {
    let parent = DoctypeMeta {
        name: "Phase One Phantom Index".to_string(),
        fields: vec![FieldMeta::data("code")],
        indexes: vec![
            crate::registry::IndexMeta {
                name: "idx_ghost".to_string(),
                fields: vec!["ghost".to_string()],
                unique: false,
                index_type: 0,
            },
            crate::registry::IndexMeta {
                name: "idx_data".to_string(),
                fields: vec!["data".to_string()],
                unique: false,
                index_type: 0,
            },
            crate::registry::IndexMeta {
                name: "idx_code".to_string(),
                fields: vec!["code".to_string()],
                unique: false,
                index_type: 0,
            },
        ],
        ..Default::default()
    };
    let joined = create_doc_type_statements(&parent, Dialect::Postgres)
        .expect("DDL renders")
        .join("\n");
    assert!(
        joined.contains("idx_tabPhaseOnePhantomIndex_code"),
        "{joined}"
    );
    assert!(
        !joined.contains("idx_tabPhaseOnePhantomIndex_ghost"),
        "{joined}"
    );
    assert!(
        !joined.contains("idx_tabPhaseOnePhantomIndex_data"),
        "{joined}"
    );
}

/// `alter_add_columns` only adds what is missing and keeps every new column
/// nullable so the backfill can populate it.
#[test]
fn alter_adds_only_missing_columns_and_keeps_them_nullable() {
    let parent = DoctypeMeta {
        name: "Phase One Alter".to_string(),
        fields: vec![
            FieldMeta::data("title").required(),
            FieldMeta::data("note").required(),
            FieldMeta::table("items", "Phase One Alter Child"),
            FieldMeta::section_break("break"),
        ],
        ..Default::default()
    };
    // `Phase One Alter Child` is intentionally unregistered: alter only
    // looks at scalar columns, never at child tables.
    let existing = vec!["name".to_string(), "title".to_string(), "data".to_string()];
    let statements =
        alter_add_columns(&parent, &existing, Dialect::Postgres).expect("ALTER renders");
    assert_eq!(statements.len(), 1, "{statements:?}");
    assert!(
        statements[0].contains("\"note\" VARCHAR(255)"),
        "{statements:?}"
    );
    assert!(!statements[0].contains("NOT NULL"), "{statements:?}");
}

/// An unknown fieldtype is rejected instead of silently becoming a column.
#[test]
fn unknown_fieldtype_is_rejected() {
    let error = field_column(&field("weird", "Dat"), Dialect::Postgres).unwrap_err();
    assert!(error.to_string().contains("unknown fieldtype"), "{error}");
}
