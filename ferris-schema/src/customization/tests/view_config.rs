//! Validation rules of the collection-view settings.
//!
//! The store refuses a setting the renderer could not honour; these tests pin
//! the refusals (unknown field, wrong fieldtype, a one-option board) and the
//! one path that must stay permissive (clearing a setting).

use ferris_core::{Error, SchemaError, ValidationError};

use super::super::{CustomizationStore, PropertySetter};
use crate::registry::{DoctypeMeta, FieldMeta};

// ────────────────────────────────────────────────────────────────────────────
// Collection-view configuration
// ────────────────────────────────────────────────────────────────────────────

fn view_field(fieldname: &str, fieldtype: &str, options: Option<&str>) -> FieldMeta {
    FieldMeta {
        fieldname: fieldname.to_string(),
        label: Some(fieldname.to_string()),
        fieldtype: fieldtype.to_string(),
        options: options.map(str::to_string),
        ..Default::default()
    }
}

/// A doctype that declares every collection view and the fields they need.
fn view_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "ViewConfigToDo".to_string(),
        module: "test".to_string(),
        in_kanban: true,
        in_calendar: true,
        in_tree_view: true,
        in_gantt: true,
        in_report_view: true,
        fields: vec![
            view_field("title", "Data", None),
            view_field("status", "Select", Some("Open\nDoing\nDone")),
            view_field("category", "Link", Some("Category")),
            view_field("due_date", "Date", None),
            view_field("start_date", "Date", None),
            view_field("parent_task", "Link", Some("ViewConfigToDo")),
            view_field("depends_on", "Link", Some("ViewConfigToDo")),
            view_field("progress", "Percent", None),
        ],
        ..Default::default()
    }
}

fn view_setter(property: &str, value: &str) -> PropertySetter {
    PropertySetter {
        doctype: "ViewConfigToDo".to_string(),
        fieldname: None,
        property: property.to_string(),
        value: value.to_string(),
    }
}

/// Every view setting the Desk reads is applied by one name, and the values
/// land in the metadata the renderers receive.
#[tokio::test]
async fn collection_view_settings_are_applied() {
    crate::registry::register_doctype(view_meta());
    let store = CustomizationStore::new();
    for (property, value) in [
        ("default_view", "kanban"),
        ("kanban_group_field", "status"),
        ("kanban_card_fields", "title, due_date"),
        ("date_field", "due_date"),
        ("calendar_end_field", "due_date"),
        ("calendar_title_field", "title"),
        ("tree_parent_field", "parent_task"),
        ("tree_label_field", "title"),
        ("tree_expanded_levels", "2"),
        ("gantt_start_field", "start_date"),
        ("gantt_end_field", "due_date"),
        ("gantt_dependency_field", "depends_on"),
        ("gantt_progress_field", "progress"),
    ] {
        store
            .set_property(view_setter(property, value))
            .await
            .unwrap();
    }

    let effective = store.apply(&view_meta()).await.unwrap();
    assert_eq!(
        effective.default_view,
        Some(crate::contract::meta::CollectionView::Kanban)
    );
    assert_eq!(effective.kanban_group_field.as_deref(), Some("status"));
    assert_eq!(effective.kanban_card_fields, ["title", "due_date"]);
    assert_eq!(effective.date_field.as_deref(), Some("due_date"));
    assert_eq!(effective.calendar_end_field.as_deref(), Some("due_date"));
    assert_eq!(effective.calendar_title_field.as_deref(), Some("title"));
    assert_eq!(effective.tree_parent_field.as_deref(), Some("parent_task"));
    assert_eq!(effective.tree_label_field.as_deref(), Some("title"));
    assert_eq!(effective.tree_expanded_levels, Some(2));
    assert_eq!(effective.gantt_start_field.as_deref(), Some("start_date"));
    assert_eq!(effective.gantt_end_field.as_deref(), Some("due_date"));
    assert_eq!(
        effective.gantt_dependency_field.as_deref(),
        Some("depends_on")
    );
    assert_eq!(effective.gantt_progress_field.as_deref(), Some("progress"));
}

/// A view setting that names a field the doctype does not declare is refused
/// **before** it is stored: the Desk would otherwise render an empty view with
/// no explanation.
#[tokio::test]
async fn a_view_setting_naming_an_unknown_field_is_rejected() {
    crate::registry::register_doctype(view_meta());
    let store = CustomizationStore::new();
    let err = store
        .set_property(view_setter("tree_parent_field", "nope"))
        .await
        .expect_err("an unknown parent field must be rejected");
    assert!(
        matches!(
            err,
            Error::Schema(SchemaError::FieldNotFound { ref doctype, ref field })
                if doctype == "ViewConfigToDo" && field == "nope"
        ),
        "expected Schema(FieldNotFound), got {err:?}"
    );
    assert!(
        store
            .property_setters("ViewConfigToDo")
            .await
            .unwrap()
            .is_empty(),
        "a rejected setter is not persisted"
    );
}

/// The field exists but cannot carry the property: a `Data` title is not a
/// date, and a link to another doctype is not a parent of this one.
#[tokio::test]
async fn a_view_setting_with_the_wrong_fieldtype_is_rejected() {
    crate::registry::register_doctype(view_meta());
    let store = CustomizationStore::new();
    for (property, field) in [
        ("gantt_start_field", "title"),
        ("tree_parent_field", "category"),
        ("gantt_progress_field", "title"),
        ("kanban_group_field", "title"),
    ] {
        let err = store
            .set_property(view_setter(property, field))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Validation(ValidationError::InvalidValue { ref field, .. })
                    if field == property
            ),
            "{property}={field} should be refused, got {err:?}"
        );
    }
}

/// A `Select` with a single option is a label, not a board: it would render one
/// column and no move.
#[tokio::test]
async fn a_kanban_group_select_needs_at_least_two_options() {
    crate::registry::register_doctype(view_meta());
    let store = CustomizationStore::new();
    let mut meta = view_meta();
    meta.name = "ViewConfigSingleOption".to_string();
    meta.fields.push(view_field("kind", "Select", Some("Only")));
    crate::registry::register_doctype(meta);

    let err = store
        .set_property(PropertySetter {
            doctype: "ViewConfigSingleOption".to_string(),
            fieldname: None,
            property: "kanban_group_field".to_string(),
            value: "kind".to_string(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Validation(ValidationError::InvalidValue { .. })),
        "a one-option Select is not a grouping field, got {err:?}"
    );
}

/// `default_view` is a URL keyword: a typo, or a view the doctype did not
/// enable, must not silently land on the list.
#[tokio::test]
async fn the_default_view_must_be_a_declared_view() {
    crate::registry::register_doctype(view_meta());
    let store = CustomizationStore::new();
    let err = store
        .set_property(view_setter("default_view", "timeline"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Validation(ValidationError::InvalidValue { .. })
    ));

    let mut closed = view_meta();
    closed.name = "ViewConfigListOnly".to_string();
    closed.in_kanban = false;
    crate::registry::register_doctype(closed);
    let err = store
        .set_property(PropertySetter {
            doctype: "ViewConfigListOnly".to_string(),
            fieldname: None,
            property: "default_view".to_string(),
            value: "kanban".to_string(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Validation(ValidationError::InvalidValue { .. })),
        "a view the doctype did not enable cannot be the default, got {err:?}"
    );
}

/// Clearing an input clears the property (the declared value comes back), and
/// a cleared value is never validated: "unset" is always a legal answer.
#[tokio::test]
async fn a_view_setting_can_be_cleared() {
    crate::registry::register_doctype(view_meta());
    let store = CustomizationStore::new();
    store
        .set_property(view_setter("tree_parent_field", "parent_task"))
        .await
        .unwrap();
    store
        .set_property(view_setter("tree_parent_field", ""))
        .await
        .unwrap();
    let effective = store.apply(&view_meta()).await.unwrap();
    assert_eq!(effective.tree_parent_field, None);
}
