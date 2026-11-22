use super::*;
use crate::registry::FieldMeta;

fn meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "ToDo".to_string(),
        module: "core".to_string(),
        fields: vec![
            FieldMeta {
                fieldname: "name".to_string(),
                fieldtype: "Data".to_string(),
                ..Default::default()
            },
            FieldMeta {
                fieldname: "title".to_string(),
                label: Some("Title".to_string()),
                fieldtype: "Data".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[tokio::test]
async fn test_custom_fields_and_property_setters() {
    let store = CustomizationStore::new();
    store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "priority".to_string(),
            label: Some("Priority".to_string()),
            fieldtype: "Select".to_string(),
            options: Some("Low\nHigh".to_string()),
            insert_after: Some("title".to_string()),
            reqd: true,
            default: Some("Low".to_string()),
            hidden: false,
            read_only: false,
        })
        .await
        .unwrap();
    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: Some("title".to_string()),
            property: "reqd".to_string(),
            value: "true".to_string(),
        })
        .await
        .unwrap();
    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: None,
            property: "title_field".to_string(),
            value: "title".to_string(),
        })
        .await
        .unwrap();

    let effective = store.apply(&meta()).await.unwrap();
    let fieldnames: Vec<&str> = effective
        .fields
        .iter()
        .map(|f| f.fieldname.as_str())
        .collect();
    assert_eq!(fieldnames, vec!["name", "title", "priority"]);
    let priority = effective
        .fields
        .iter()
        .find(|f| f.fieldname == "priority")
        .unwrap();
    assert_eq!(priority.fieldtype, "Select");
    assert!(priority.reqd);
    let title = effective
        .fields
        .iter()
        .find(|f| f.fieldname == "title")
        .unwrap();
    assert!(title.reqd);
    assert_eq!(effective.title_field.as_deref(), Some("title"));
}

/// a fieldtype outside the canonical allowlist is refused at the
/// write boundary with the typed error, instead of being persisted and
/// silently rendered as a text box.
#[tokio::test]
async fn custom_field_with_an_unknown_fieldtype_is_rejected() {
    let store = CustomizationStore::new();
    let err = store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "weird".to_string(),
            label: None,
            fieldtype: "Icon".to_string(),
            options: None,
            insert_after: None,
            reqd: false,
            default: None,
            hidden: false,
            read_only: false,
        })
        .await
        .expect_err("an unknown fieldtype must be rejected");
    assert!(
        matches!(err, Error::Schema(SchemaError::InvalidFieldType(ref name)) if name == "Icon"),
        "expected Schema(InvalidFieldType(\"Icon\")), got {err:?}"
    );
    assert!(store.custom_fields("ToDo").await.unwrap().is_empty());
}

/// The canonical fieldtype is stored verbatim, so what comes back out of the
/// store is what the Desk renderer matches.
#[tokio::test]
async fn custom_field_stores_the_canonical_fieldtype() {
    let store = CustomizationStore::new();
    store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "notes".to_string(),
            label: None,
            fieldtype: "Small Text".to_string(),
            options: None,
            insert_after: None,
            reqd: false,
            default: None,
            hidden: false,
            read_only: false,
        })
        .await
        .expect("a known fieldtype is accepted");

    let stored = store.custom_fields("ToDo").await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].fieldtype, "Small Text");
}

#[tokio::test]
async fn test_persistence_roundtrip() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = CustomizationStore::with_db(db);
    store.init().await.unwrap();
    store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "due_date".to_string(),
            label: None,
            fieldtype: "Date".to_string(),
            options: None,
            insert_after: None,
            reqd: false,
            default: Some("2026-01-01".to_string()),
            hidden: false,
            read_only: false,
        })
        .await
        .unwrap();

    let fields = store.custom_fields("ToDo").await.unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].fieldtype, "Date");
    // `default` is a reserved word: it only round-trips if it is quoted.
    assert_eq!(fields[0].default.as_deref(), Some("2026-01-01"));

    // Adding the same field again replaces it rather than duplicating.
    store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "due_date".to_string(),
            label: Some("Due".to_string()),
            fieldtype: "Date".to_string(),
            options: None,
            insert_after: None,
            reqd: true,
            default: Some("2026-01-01".to_string()),
            hidden: false,
            read_only: false,
        })
        .await
        .unwrap();
    let fields = store.custom_fields("ToDo").await.unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].label.as_deref(), Some("Due"));
    assert!(fields[0].reqd);

    // A property setter gets its id from the database, so two of them for
    // the same doctype must not collide the way `MAX(id) + 1` would.
    for property in ["reqd", "label"] {
        store
            .set_property(PropertySetter {
                doctype: "ToDo".to_string(),
                fieldname: Some("due_date".to_string()),
                property: property.to_string(),
                value: "1".to_string(),
            })
            .await
            .unwrap();
    }
    let setters = store.property_setters("ToDo").await.unwrap();
    assert_eq!(setters.len(), 2);
    assert_eq!(setters[0].property, "reqd");
    assert_eq!(setters[1].property, "label");
}

/// Removing customizations is what makes "customize at runtime" reversible:
/// a deleted custom field stops appearing in the effective metadata, and a
/// deleted property setter stops overriding the registry default.
#[tokio::test]
async fn delete_custom_field_and_property_setter_undo_a_customization() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = CustomizationStore::with_db(db);
    store.init().await.unwrap();
    store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "priority".to_string(),
            label: None,
            fieldtype: "Select".to_string(),
            options: None,
            insert_after: Some("title".to_string()),
            reqd: false,
            default: None,
            hidden: false,
            read_only: false,
        })
        .await
        .unwrap();
    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: Some("title".to_string()),
            property: "reqd".to_string(),
            value: "true".to_string(),
        })
        .await
        .unwrap();
    // A DocType-level setter has a NULL fieldname; deleting by `fieldname =
    // NULL` would match nothing, so this is the case that proves `IS NULL`.
    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: None,
            property: "title_field".to_string(),
            value: "title".to_string(),
        })
        .await
        .unwrap();

    store.delete_custom_field("ToDo", "priority").await.unwrap();
    store
        .delete_property_setter("ToDo", Some("title"), "reqd")
        .await
        .unwrap();
    store
        .delete_property_setter("ToDo", None, "title_field")
        .await
        .unwrap();

    assert!(store.custom_fields("ToDo").await.unwrap().is_empty());
    assert!(store.property_setters("ToDo").await.unwrap().is_empty());

    let effective = store.apply(&meta()).await.unwrap();
    assert!(
        effective.fields.iter().all(|f| f.fieldname != "priority"),
        "a deleted custom field must not survive in the effective metadata"
    );
    let title = effective
        .fields
        .iter()
        .find(|f| f.fieldname == "title")
        .unwrap();
    assert!(!title.reqd, "a deleted property setter must stop applying");
    assert_eq!(effective.title_field, None);
}

/// Deleting a customization that was never made deletes nothing and
/// succeeds, so a double undo is safe.
#[tokio::test]
async fn deleting_an_absent_customization_is_a_no_op() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = CustomizationStore::with_db(db);
    store.init().await.unwrap();
    store.delete_custom_field("ToDo", "never").await.unwrap();
    store
        .delete_property_setter("ToDo", Some("never"), "reqd")
        .await
        .unwrap();
}

/// H10: a `fieldtype` property setter goes through the same allowlist as a
/// custom field. An unknown name is rejected before it reaches the effective
/// metadata, where it would silently degrade to a plain text input.
#[tokio::test]
async fn property_setter_rejects_an_unknown_fieldtype() {
    let store = CustomizationStore::new();
    let err = store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: Some("title".to_string()),
            property: "fieldtype".to_string(),
            value: "NotAFieldtype".to_string(),
        })
        .await
        .expect_err("an unknown fieldtype must be rejected");
    assert!(
        matches!(err, Error::Schema(SchemaError::InvalidFieldType(ref name)) if name == "NotAFieldtype"),
        "unexpected error: {err}"
    );
    assert!(
        store.property_setters("ToDo").await.unwrap().is_empty(),
        "a rejected setter must not be stored"
    );
}

/// A property setter stores the exact canonical fieldtype the renderer knows.
#[tokio::test]
async fn property_setter_stores_the_canonical_fieldtype() {
    let store = CustomizationStore::new();
    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: Some("title".to_string()),
            property: "FieldType".to_string(),
            value: "Small Text".to_string(),
        })
        .await
        .unwrap();

    let stored = store.property_setters("ToDo").await.unwrap();
    assert_eq!(stored[0].property, "fieldtype");
    assert_eq!(stored[0].value, "Small Text");

    let effective = store.apply(&meta()).await.unwrap();
    let title = effective
        .fields
        .iter()
        .find(|f| f.fieldname == "title")
        .unwrap();
    assert_eq!(title.fieldtype, "Small Text");
}

/// The calendar groups by `DoctypeMeta::date_field`; a doctype-level setter is
/// what makes that field editable from the Desk instead of a code change in the
/// doctype builder. Clearing the input removes the declared field again.
#[tokio::test]
async fn a_date_field_setter_is_applied_and_can_be_cleared() {
    let store = CustomizationStore::new();
    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: None,
            property: "date_field".to_string(),
            value: "due_date".to_string(),
        })
        .await
        .unwrap();
    let effective = store.apply(&meta()).await.unwrap();
    assert_eq!(effective.date_field.as_deref(), Some("due_date"));

    store
        .set_property(PropertySetter {
            doctype: "ToDo".to_string(),
            fieldname: None,
            property: "date_field".to_string(),
            value: String::new(),
        })
        .await
        .unwrap();
    let effective = store.apply(&meta()).await.unwrap();
    assert_eq!(
        effective.date_field, None,
        "an empty value means 'declared none'"
    );
}

fn field(fieldname: &str) -> FieldMeta {
    FieldMeta {
        fieldname: fieldname.to_string(),
        fieldtype: "Data".to_string(),
        ..Default::default()
    }
}

fn move_after(fieldname: &str, anchor: &str) -> PropertySetter {
    PropertySetter {
        doctype: "ToDo".to_string(),
        fieldname: Some(fieldname.to_string()),
        property: "insert_after".to_string(),
        value: anchor.to_string(),
    }
}

#[test]
fn insert_after_moves_a_field_next_to_its_anchor() {
    let mut fields = vec![field("a"), field("b"), field("c")];
    apply_field_order(&mut fields, &[move_after("a", "c")]);
    let names: Vec<&str> = fields.iter().map(|f| f.fieldname.as_str()).collect();
    assert_eq!(names, ["b", "c", "a"]);
}

#[test]
fn insert_after_first_puts_a_field_at_the_top() {
    let mut fields = vec![field("a"), field("b"), field("c")];
    apply_field_order(&mut fields, &[move_after("c", "__first__")]);
    let names: Vec<&str> = fields.iter().map(|f| f.fieldname.as_str()).collect();
    assert_eq!(names, ["c", "a", "b"]);
}

#[test]
fn a_move_with_an_unknown_anchor_leaves_the_order_untouched() {
    let mut fields = vec![field("a"), field("b"), field("c")];
    apply_field_order(&mut fields, &[move_after("b", "missing")]);
    let names: Vec<&str> = fields.iter().map(|f| f.fieldname.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"], "an unknown anchor is a no-op");
    // A self-anchor is a no-op instead of a silent reordering.
    let mut fields = vec![field("a"), field("b")];
    apply_field_order(&mut fields, &[move_after("a", "a")]);
    let names: Vec<&str> = fields.iter().map(|f| f.fieldname.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
}

#[tokio::test]
async fn apply_moves_a_field_across_a_section() {
    let store = CustomizationStore::new();
    store
        .add_custom_field(CustomField {
            doctype: "ToDo".to_string(),
            fieldname: "section".to_string(),
            label: Some("More".to_string()),
            fieldtype: "Section Break".to_string(),
            options: None,
            insert_after: Some("title".to_string()),
            reqd: false,
            default: None,
            hidden: false,
            read_only: false,
        })
        .await
        .unwrap();
    // `name` starts above the break; anchoring it to the break itself is how the
    // Desk moves a field into the section the break opens.
    store
        .set_property(move_after("name", "section"))
        .await
        .unwrap();
    let applied = store.apply(&meta()).await.unwrap();
    let names: Vec<&str> = applied
        .fields
        .iter()
        .map(|f| f.fieldname.as_str())
        .collect();
    assert_eq!(names, ["title", "section", "name"]);
}

mod view_config;
