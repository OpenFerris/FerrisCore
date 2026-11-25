use super::*;

fn field(name: &str, fieldtype: &str) -> ferris_schema::FieldMeta {
    ferris_schema::FieldMeta {
        fieldname: name.to_string(),
        fieldtype: fieldtype.to_string(),
        ..Default::default()
    }
}

/// Declared fields are real columns and `Table` children are real rows.
///
/// There is no blob: the assertions read the columns directly and then read the
/// document back through the engine, which rebuilds it from those columns and
/// the child table.
#[tokio::test]
async fn declared_fields_are_columns_and_children_are_rows() {
    let parent = "TypedProbe";
    let child = "TypedProbeLine";
    ferris_schema::register_doctype(DoctypeMeta {
        name: child.to_string(),
        is_child_table: true,
        fields: vec![field("item", "Data"), field("amount", "Currency")],
        ..Default::default()
    });
    let mut lines = field("lines", "Table");
    lines.options = Some(child.to_string());
    ferris_schema::register_doctype(DoctypeMeta {
        name: parent.to_string(),
        fields: vec![
            field("title", "Data"),
            field("qty", "Int"),
            field("price", "Currency"),
            field("active", "Check"),
            field("due", "Date"),
            field("seen_at", "Datetime"),
            lines,
        ],
        ..Default::default()
    });

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new(parent, "TYP-.#####").allow_anonymous(true);

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "probe",
                "qty": 3,
                "price": 12.5,
                "active": true,
                "due": "2026-01-02",
                "seen_at": "2026-01-02T03:04:05Z",
                "lines": [
                    {"item": "a", "amount": 1.5},
                    {"item": "b", "amount": 2.5}
                ]
            }),
        )
        .await
        .unwrap();
    let name = created
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();

    let row = db
        .fetch_one(&Stmt::new(
            "SELECT title, qty, price, active, due, seen_at FROM tabTypedProbe WHERE name = ?",
            vec![Value::from(name.clone())],
        ))
        .await
        .unwrap();
    assert_eq!(row.opt_str("title").as_deref(), Some("probe"));
    assert_eq!(row.opt_i64("qty"), Some(3));
    assert_eq!(row.opt_decimal_f64("price"), Some(12.5));
    assert_eq!(row.opt_bool("active"), Some(true));
    assert_eq!(row.opt_date("due").as_deref(), Some("2026-01-02"));
    assert_eq!(
        row.opt_datetime("seen_at").as_deref(),
        Some("2026-01-02T03:04:05+00:00")
    );

    let children = db
        .fetch_all(&Stmt::new(
            "SELECT item, amount FROM tabTypedProbeLine WHERE parent = ? ORDER BY idx",
            vec![Value::from(name.clone())],
        ))
        .await
        .unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].opt_str("item").as_deref(), Some("a"));
    assert_eq!(children[1].opt_str("item").as_deref(), Some("b"));

    // Update replaces the child rows and the changed scalar column.
    engine
        .update(
            &ctx,
            &name,
            ferris_core::value!({
                "qty": 9,
                "lines": [{"item": "c", "amount": 7.0}]
            }),
        )
        .await
        .unwrap();
    let row = db
        .fetch_one(&Stmt::new(
            "SELECT qty FROM tabTypedProbe WHERE name = ?",
            vec![Value::from(name.clone())],
        ))
        .await
        .unwrap();
    assert_eq!(row.opt_i64("qty"), Some(9));
    let children = db
        .fetch_all(&Stmt::new(
            "SELECT item FROM tabTypedProbeLine WHERE parent = ? ORDER BY idx",
            vec![Value::from(name.clone())],
        ))
        .await
        .unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].opt_str("item").as_deref(), Some("c"));

    // Read back through the engine: the child row comes from the child table.
    let fetched = engine.get(&ctx, &name).await.unwrap();
    let lines = fetched.get("lines").and_then(Value::as_array).unwrap();
    assert_eq!(lines.len(), 1, "the child row comes from the child table");
    assert_eq!(lines[0].get("item").and_then(Value::as_str), Some("c"));
}

/// The four link columns are engine-owned only on a child table.
///
/// A normal `DocType` may declare `parent` as an ordinary field (Frappe's
/// `Custom DocPerm` does): it owns a real column and must round-trip, not be
/// filtered out as if it were a child's bookkeeping.
#[tokio::test]
async fn a_normal_doctype_owns_a_declared_parent_column() {
    let doctype = "TypedDeclaredParent";
    ferris_schema::register_doctype(DoctypeMeta {
        name: doctype.to_string(),
        fields: vec![field("title", "Data"), field("parent", "Data")],
        ..Default::default()
    });

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new(doctype, "TDP-.#####").allow_anonymous(true);
    let created = engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "override", "parent": "TypedProbe" }),
        )
        .await
        .unwrap();
    let name = created
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();

    let row = db
        .fetch_one(&Stmt::new(
            "SELECT parent FROM tabTypedDeclaredParent WHERE name = ?",
            vec![Value::from(name.clone())],
        ))
        .await
        .unwrap();
    assert_eq!(row.opt_str("parent").as_deref(), Some("TypedProbe"));

    let fetched = engine.get(&ctx, &name).await.unwrap();
    assert_eq!(
        fetched.get("parent").and_then(Value::as_str),
        Some("TypedProbe")
    );
}

/// A `NULL` column reads as a field that was never set, not as `0` or `""`.
#[tokio::test]
async fn null_columns_read_as_missing_fields() {
    let doctype = "TypedLegacy";
    ferris_schema::register_doctype(DoctypeMeta {
        name: doctype.to_string(),
        fields: vec![field("title", "Data"), field("qty", "Int")],
        ..Default::default()
    });

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new(doctype, "LEG-.#####").allow_anonymous(true);
    engine.ensure_table(doctype).await.unwrap();

    // Only the system columns: every declared column is NULL.
    db.execute(&Stmt::new(
        "INSERT INTO tabTypedLegacy (name, owner, docstatus, creation, modified) \
         VALUES (?, ?, ?, ?, ?)",
        vec![
            Value::from("LEG-1"),
            Value::from("tester"),
            Value::from(0_i64),
            Value::from(1_i64),
            Value::from(1_i64),
        ],
    ))
    .await
    .unwrap();

    let fetched = engine.get(&ctx, "LEG-1").await.unwrap();
    assert_eq!(
        fetched.get("name").and_then(Value::as_str),
        Some("LEG-1"),
        "the system columns always reconstruct"
    );
    assert!(fetched.get("title").is_none(), "{fetched:?}");
    assert!(fetched.get("qty").is_none(), "{fetched:?}");
}

/// The ensured-table cache belongs to the database, not to one engine.
///
/// The observability path builds a fresh engine for every request; if each one
/// re-ran `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX`, logging would issue DDL
/// per request. `Db::id` lets all clones of one pool share the set.
#[tokio::test]
async fn engines_over_one_database_share_the_ensured_cache() {
    let db = ferris_db::memory_db().await.unwrap();
    let first = DocumentEngine::new(db.clone());
    let second = DocumentEngine::new(db);
    assert!(std::sync::Arc::ptr_eq(&first.ensured, &second.ensured));
}

/// A save with many child rows uses the batched `INSERT`, including the chunk
/// split that keeps the bind count under SQLite's variable ceiling.
#[tokio::test]
async fn many_child_rows_are_written_in_batches() {
    let parent = "TypedBatchParent";
    let child = "TypedBatchLine";
    ferris_schema::register_doctype(DoctypeMeta {
        name: child.to_string(),
        is_child_table: true,
        fields: vec![field("item", "Data"), field("amount", "Currency")],
        ..Default::default()
    });
    let mut lines = field("lines", "Table");
    lines.options = Some(child.to_string());
    ferris_schema::register_doctype(DoctypeMeta {
        name: parent.to_string(),
        fields: vec![field("title", "Data"), lines],
        ..Default::default()
    });

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db);
    let ctx = EngineContext::new(parent, "BAT-.#####").allow_anonymous(true);

    let rows: Vec<Value> = (0..120)
        .map(|index| {
            let mut row = ferris_core::Map::new();
            row.insert("item".to_string(), Value::from(format!("item-{index}")));
            row.insert("amount".to_string(), Value::from(f64::from(index)));
            Value::Object(row)
        })
        .collect();
    let mut body = ferris_core::Map::new();
    body.insert("title".to_string(), Value::from("batch"));
    body.insert("lines".to_string(), Value::Array(rows));
    let created = engine.create(&ctx, Value::Object(body)).await.unwrap();
    let name = created.get("name").and_then(Value::as_str).unwrap();

    let fetched = engine.get(&ctx, name).await.unwrap();
    let children = fetched.get("lines").and_then(Value::as_array).unwrap();
    assert_eq!(children.len(), 120);
    assert_eq!(
        children[119].get("item").and_then(Value::as_str),
        Some("item-119")
    );
}

/// Register a parent with one child table, returning both names.
fn cascade_fixture(parent: &str, child: &str) {
    ferris_schema::register_doctype(DoctypeMeta {
        name: child.to_string(),
        is_child_table: true,
        fields: vec![field("item", "Data")],
        ..Default::default()
    });
    let mut lines = field("lines", "Table");
    lines.options = Some(child.to_string());
    ferris_schema::register_doctype(DoctypeMeta {
        name: parent.to_string(),
        fields: vec![field("title", "Data"), lines],
        ..Default::default()
    });
}

/// The number of child rows a parent owns, read from the child table directly.
async fn child_rows(db: &ferris_db::Db, child: &str, parent: &str) -> i64 {
    let table = q(db.dialect(), &crate::engine::table_name(child).unwrap()).unwrap();
    db.fetch_one(&Stmt::new(
        format!("SELECT COUNT(*) AS n FROM {table} WHERE parent = ?"),
        vec![Value::from(parent)],
    ))
    .await
    .unwrap()
    .i64("n")
    .unwrap()
}

/// Deleting a parent removes its child rows in the same transaction.
///
/// A child table is keyed by `parent` and nothing else; the database enforces
/// no foreign key, so the engine is the only one that can cascade. A leftover
/// row would resurface the moment the name is reused.
#[tokio::test]
async fn deleting_a_parent_removes_its_child_rows() {
    let parent = "TypedCascadeParent";
    let child = "TypedCascadeLine";
    cascade_fixture(parent, child);

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new(parent, "CAS-.#####").with_user(&sysadmin());
    let name = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "cascade",
                "lines": [{"item": "a"}, {"item": "b"}, {"item": "c"}]
            }),
        )
        .await
        .unwrap()
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    assert_eq!(child_rows(&db, child, &name).await, 3);

    engine.delete(&ctx, &name).await.unwrap();
    assert_eq!(
        child_rows(&db, child, &name).await,
        0,
        "delete must cascade the child table, not orphan its rows"
    );
}

/// A purge is a hard delete: the child rows go with the document too.
#[tokio::test]
async fn purging_a_parent_removes_its_child_rows() {
    let parent = "TypedPurgeParent";
    let child = "TypedPurgeLine";
    cascade_fixture(parent, child);

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new(parent, "PRG-.#####").with_user(&sysadmin());
    let name = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "purge",
                "lines": [{"item": "a"}, {"item": "b"}]
            }),
        )
        .await
        .unwrap()
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Submitted)
        .await
        .unwrap();
    engine
        .set_docstatus(&ctx, &name, DocStatus::Cancelled)
        .await
        .unwrap();
    assert_eq!(child_rows(&db, child, &name).await, 2);

    engine.purge(&ctx, &name, "court order").await.unwrap();
    assert_eq!(
        child_rows(&db, child, &name).await,
        0,
        "purge must cascade the child table"
    );
}

/// Two parent DocTypes may share a child table and a field name; their rows
/// must not collide.
///
/// `parent` alone is not unique across DocTypes (names are scoped per table),
/// which is exactly what `parenttype` is for. A write or a read that ignored it
/// would delete or return the other DocType's lines.
#[tokio::test]
async fn two_parents_sharing_a_child_table_do_not_leak_rows() {
    let child = "TypedSharedLine";
    ferris_schema::register_doctype(DoctypeMeta {
        name: child.to_string(),
        is_child_table: true,
        fields: vec![field("item", "Data")],
        ..Default::default()
    });
    let parents = ["TypedSharedAlpha", "TypedSharedBeta"];
    for parent in parents {
        let mut lines = field("lines", "Table");
        lines.options = Some(child.to_string());
        ferris_schema::register_doctype(DoctypeMeta {
            name: parent.to_string(),
            fields: vec![field("title", "Data"), lines],
            ..Default::default()
        });
    }

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db);
    let alpha = EngineContext::new(parents[0], "")
        .allow_anonymous(true)
        .with_explicit_name();
    let beta = EngineContext::new(parents[1], "")
        .allow_anonymous(true)
        .with_explicit_name();
    let body = |title: &str, item: &str| {
        ferris_core::value!({
            "name": "SHARED-1",
            "title": title,
            "lines": [{"item": item}]
        })
    };

    engine
        .create(&alpha, body("alpha", "alpha-1"))
        .await
        .unwrap();
    engine.create(&beta, body("beta", "beta-1")).await.unwrap();

    // Rewriting alpha replaces *its* children only.
    engine
        .update(
            &alpha,
            "SHARED-1",
            ferris_core::value!({
                "lines": [{"item": "alpha-2"}]
            }),
        )
        .await
        .unwrap();

    for (doctype, expected) in [(parents[0], "alpha-2"), (parents[1], "beta-1")] {
        let ctx = EngineContext::new(doctype, "").allow_anonymous(true);
        let document = engine.get(&ctx, "SHARED-1").await.unwrap();
        let item = document
            .get("lines")
            .and_then(Value::as_array)
            .and_then(|lines| lines.first())
            .and_then(|line| line.get("item"))
            .and_then(Value::as_str);
        assert_eq!(
            item,
            Some(expected),
            "`{doctype}` must see its own children and only its own"
        );
    }
}

/// `create_bulk` resolves the physical shape once and shares it across every
/// body: the batched children must still be written for each document.
#[tokio::test]
async fn bulk_create_shares_the_shape_and_writes_every_child_row() {
    let parent = "TypedBulkParent";
    let child = "TypedBulkLine";
    cascade_fixture(parent, child);

    let db = ferris_db::memory_db().await.unwrap();
    let engine = DocumentEngine::new(db.clone());
    let ctx = EngineContext::new(parent, "BULK-.#####").allow_anonymous(true);
    let bodies = (0..40)
        .map(|index| {
            let mut line = ferris_core::Map::new();
            line.insert("item".to_string(), Value::from(format!("line-{index}")));
            let mut body = ferris_core::Map::new();
            body.insert("title".to_string(), Value::from(format!("row-{index}")));
            body.insert("lines".to_string(), Value::Array(vec![Value::Object(line)]));
            Value::Object(body)
        })
        .collect();

    let created = engine.create_bulk(&ctx, bodies).await.unwrap();
    assert_eq!(created.len(), 40);
    for document in &created {
        let name = document.get("name").and_then(Value::as_str).unwrap();
        assert_eq!(
            child_rows(&db, child, name).await,
            1,
            "{name} lost its child row"
        );
    }
}
