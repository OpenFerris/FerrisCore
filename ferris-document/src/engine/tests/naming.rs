use super::*;

#[tokio::test]
async fn auto_increment_naming_mints_a_plain_counter() {
    let engine = engine().await;
    let ctx = EngineContext::new("Ticket", "")
        .with_naming(ferris_schema::contract::NamingType::AutoIncrement, None)
        .allow_anonymous(true);

    let first = engine
        .create(&ctx, ferris_core::value!({ "title": "a" }))
        .await
        .unwrap();
    let second = engine
        .create(&ctx, ferris_core::value!({ "title": "b" }))
        .await
        .unwrap();
    assert_eq!(first.get("name").and_then(|v| v.as_str()), Some("1"));
    assert_eq!(second.get("name").and_then(|v| v.as_str()), Some("2"));
}

/// The implicit resolution: no declaration and no series is an opaque UUID, so
/// a doctype that says nothing keeps the pre-existing behaviour.
#[tokio::test]
async fn an_undeclared_doctype_without_a_series_gets_a_uuid() {
    let engine = engine().await;
    let ctx = EngineContext::new("Blob", "").allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "x" }))
        .await
        .unwrap();
    let name = created.get("name").and_then(|v| v.as_str()).unwrap();
    assert!(
        uuid::Uuid::parse_str(name).is_ok(),
        "expected a UUID, got {name}"
    );
}

/// A declared `Prompt` refuses to mint: the caller must supply the id, and the
/// typed error says which doctype demanded it.
#[tokio::test]
async fn prompt_naming_requires_the_caller_to_supply_a_name() {
    let engine = engine().await;
    let ctx = EngineContext::new("Case", "")
        .with_naming(ferris_schema::contract::NamingType::Prompt, None)
        .allow_anonymous(true);

    let err = engine
        .create(&ctx, ferris_core::value!({ "title": "a" }))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Document(DocumentError::NameRequired { ref doctype }) if doctype == "Case"
        ),
        "expected NameRequired, got {err:?}"
    );

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({ "name": "CASE-1", "title": "a" }),
        )
        .await
        .unwrap();
    assert_eq!(created.get("name").and_then(|v| v.as_str()), Some("CASE-1"));
}

/// A declared `Field` names the document after that field: the value wins, a
/// missing value is a typed error, and a missing `naming_field` is a
/// declaration error rather than a silent UUID.
#[tokio::test]
async fn field_naming_reads_the_declared_field() {
    let engine = engine().await;
    let ctx = EngineContext::new("Item", "")
        .with_naming(
            ferris_schema::contract::NamingType::Field,
            Some("code".to_string()),
        )
        .allow_anonymous(true);

    let created = engine
        .create(&ctx, ferris_core::value!({ "code": "SKU-7", "title": "a" }))
        .await
        .unwrap();
    assert_eq!(created.get("name").and_then(|v| v.as_str()), Some("SKU-7"));

    let err = engine
        .create(&ctx, ferris_core::value!({ "title": "b" }))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Document(DocumentError::NamingFieldEmpty { ref field, .. }) if field == "code"
        ),
        "expected NamingFieldEmpty, got {err:?}"
    );

    let undeclared = EngineContext::new("Item", "")
        .with_naming(ferris_schema::contract::NamingType::Field, None)
        .allow_anonymous(true);
    let err = engine
        .create(&undeclared, ferris_core::value!({ "code": "X" }))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Document(DocumentError::NamingFieldUndeclared { .. })
    ));
}

/// "By Naming Series field": the pattern comes from the document itself, so
/// two documents of the same DocType may follow different series.
#[tokio::test]
async fn naming_series_field_reads_the_document_pattern() {
    let engine = engine().await;
    let ctx = EngineContext::new("Invoice", "")
        .with_naming(ferris_schema::contract::NamingType::NamingSeries, None)
        .allow_anonymous(true);
    let first = engine
        .create(
            &ctx,
            ferris_core::value!({ "naming_series": "PRE.#####", "title": "a" }),
        )
        .await
        .unwrap();
    let second = engine
        .create(
            &ctx,
            ferris_core::value!({ "naming_series": "OTH.#####", "title": "b" }),
        )
        .await
        .unwrap();
    assert_eq!(first.get("name").and_then(|v| v.as_str()), Some("PRE00001"));
    assert_eq!(
        second.get("name").and_then(|v| v.as_str()),
        Some("OTH00001")
    );
}

/// A Naming Series field that is empty and no declared fallback is a typed
/// error, not a silent UUID.
#[tokio::test]
async fn an_empty_naming_series_is_a_typed_error() {
    let engine = engine().await;
    let ctx = EngineContext::new("Invoice", "")
        .with_naming(ferris_schema::contract::NamingType::NamingSeries, None)
        .allow_anonymous(true);
    let error = engine
        .create(&ctx, ferris_core::value!({ "title": "a" }))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Document(DocumentError::NamingSeriesMissing { ref doctype }) if doctype == "Invoice"
    ));
}

/// The Expression method concatenates its dot-separated parts and pads the
/// counter to the `#` run, so `EX-{code}-.#####` mints `EX-A-00001`.
#[tokio::test]
async fn expression_naming_uses_the_declared_pattern() {
    let engine = engine().await;
    let ctx = EngineContext::new("Example", "EX-{code}-.#####")
        .with_naming(ferris_schema::contract::NamingType::Expression, None)
        .allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "code": "A" }))
        .await
        .unwrap();
    assert_eq!(
        created.get("name").and_then(|v| v.as_str()),
        Some("EX-A-00001")
    );
}

/// The `Script` method is a registered Rust provider; an undeclared provider
/// fails loudly instead of falling back to a UUID.
#[tokio::test]
async fn script_naming_uses_a_registered_provider() {
    fn provider(_document: &Value, ctx: &EngineContext<'_>) -> ferris_core::Result<String> {
        Ok(format!("{}-SCRIPT", ctx.doctype))
    }
    crate::naming::register_naming_provider("Scripted", provider);
    let engine = engine().await;
    let ctx = EngineContext::new("Scripted", "")
        .with_naming(ferris_schema::contract::NamingType::Script, None)
        .allow_anonymous(true);
    let created = engine
        .create(&ctx, ferris_core::value!({ "title": "a" }))
        .await
        .unwrap();
    assert_eq!(
        created.get("name").and_then(|v| v.as_str()),
        Some("Scripted-SCRIPT")
    );

    let missing = EngineContext::new("Unscripted", "")
        .with_naming(ferris_schema::contract::NamingType::Script, None)
        .allow_anonymous(true);
    let error = engine
        .create(&missing, ferris_core::value!({ "title": "a" }))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Document(DocumentError::NamingScriptUndeclared { .. })
    ));
}

/// A matching Document Naming Rule wins over the declared naming type and
/// advances its own persisted counter; a document that does not match falls
/// back to the declaration.
#[tokio::test]
async fn a_document_naming_rule_beats_the_declared_type() {
    let db = ferris_db::memory_db().await.unwrap();
    crate::naming::NamingSeriesStore::ensure_table(&db)
        .await
        .unwrap();
    db.ensure_table(
        "tabDocumentNamingRule",
        &ferris_schema::migration::document_columns(),
    )
    .await
    .unwrap();
    let rule = serde_json::json!({
        "name": "High priority tickets",
        "document_type": "Ticket",
        "prefix": "hi-",
        "counter": 0,
        "prefix_digits": 3,
        "priority": 10,
        "disabled": 0,
        "apply_if": "Fieldname",
        "fieldname": "urgent",
    })
    .to_string();
    db.execute(&Stmt::new(
        "INSERT INTO tabDocumentNamingRule (name, owner, docstatus, creation, modified, data) \
         VALUES (?, ?, ?, ?, ?, ?)",
        vec![
            Value::from("High priority tickets"),
            Value::from("admin@local"),
            Value::from(0i64),
            Value::from(0i64),
            Value::from(0i64),
            Value::from(rule),
        ],
    ))
    .await
    .unwrap();

    let engine = DocumentEngine::new(db);
    let ctx = EngineContext::new("Ticket", "")
        .with_naming(ferris_schema::contract::NamingType::AutoIncrement, None)
        .allow_anonymous(true);
    let high = engine
        .create(&ctx, ferris_core::value!({ "urgent": 1, "title": "a" }))
        .await
        .unwrap();
    let ordinary = engine
        .create(&ctx, ferris_core::value!({ "urgent": 0, "title": "b" }))
        .await
        .unwrap();
    assert_eq!(high.get("name").and_then(|v| v.as_str()), Some("hi-001"));
    // The rule did not apply, so the declared AutoIncrement counter is used.
    assert_eq!(ordinary.get("name").and_then(|v| v.as_str()), Some("1"));
}

/// An explicit name always wins over the declared strategy, so importers and
/// the amend copy keep working whatever the doctype declares.
#[tokio::test]
async fn an_explicit_name_beats_the_declared_strategy() {
    let engine = engine().await;
    let ctx = EngineContext::new("Ticket", "")
        .with_naming(ferris_schema::contract::NamingType::AutoIncrement, None)
        .allow_anonymous(true);
    let created = engine
        .create(
            &ctx,
            ferris_core::value!({ "name": "CHOSEN", "title": "a" }),
        )
        .await
        .unwrap();
    assert_eq!(created.get("name").and_then(|v| v.as_str()), Some("CHOSEN"));
}
