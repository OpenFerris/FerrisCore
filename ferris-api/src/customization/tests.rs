//! Tests for the customization surface: the permission gate, the store writes
//! that become effective metadata, and the world export.

use ferris_core::ValidationError;
use ferris_schema::contract::customization::{ExportScope, WorldCustomization};

use super::*;

fn user(id: &str, roles: &[&str]) -> ferris_auth::User {
    let mut user = ferris_auth::User::system_user(id.to_string(), id.to_string());
    user.id = id.to_string();
    user.roles = roles.iter().map(|r| (*r).to_string()).collect();
    user
}

fn admin() -> ferris_auth::User {
    user("admin@local", &["System Manager"])
}

/// A plugins root that only this test owns.
///
/// The suite runs its tests in parallel threads of one process, so the path is
/// keyed by the test's own tag: two exports into the same directory would
/// otherwise overwrite each other's file and make the assertions racy.
fn temp_plugins(tag: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("ferris-export-test-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// Register an empty probe DocType so the customization store has something to
/// attach fields to.
fn register_probe(name: &str) {
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
        name: name.to_string(),
        module: "test".to_string(),
        fields: vec![ferris_schema::FieldMeta {
            fieldname: "title".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });
}

/// Add a custom field to `doctype` with the defaults the export tests do not
/// care about.
async fn add_field(pool: &ferris_db::Db, doctype: &str, fieldname: &str) {
    save_custom_field(
        pool,
        &admin(),
        doctype,
        &CustomFieldSpec {
            fieldname: fieldname.to_string(),
            label: Some(fieldname.to_string()),
            fieldtype: "Data".to_string(),
            options: None,
            insert_after: Some("title".to_string()),
            reqd: false,
            default: None,
            hidden: false,
            read_only: false,
        },
    )
    .await
    .unwrap();
}

/// Add a property setter to `doctype`.
async fn add_setter(pool: &ferris_db::Db, doctype: &str, fieldname: Option<&str>, property: &str) {
    save_property_setter(
        pool,
        &admin(),
        doctype,
        &PropertySetterSpec {
            fieldname: fieldname.map(str::to_string),
            property: property.to_string(),
            value: "set".to_string(),
        },
    )
    .await
    .unwrap();
}

/// Only a manager may customize: a plain user is refused, the System Manager
/// is allowed, and the decision comes from the policy engine.
#[test]
fn only_managers_may_customize() {
    assert!(ensure_can_customize(&admin(), "ToDo").is_ok());
    let err = ensure_can_customize(&user("mallory@local", &[]), "ToDo").unwrap_err();
    assert!(matches!(
        err,
        Error::Authz(AuthzError::PermissionDenied { .. })
    ));
}

/// The whole point of the write side: a custom field saved here, persisted
/// in the store, is what `effective_meta` applies on the next request — no
/// recompilation, and no in-memory-only state.
#[tokio::test]
async fn a_saved_custom_field_becomes_part_of_the_effective_metadata() {
    let pool = ferris_db::memory_db().await.unwrap();
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
        name: "Customizable Probe".to_string(),
        module: "test".to_string(),
        fields: vec![ferris_schema::FieldMeta {
            fieldname: "title".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });

    save_custom_field(
        &pool,
        &admin(),
        "Customizable Probe",
        &CustomFieldSpec {
            fieldname: "priority".to_string(),
            label: Some("Priority".to_string()),
            fieldtype: "Select".to_string(),
            options: Some("Low\nHigh".to_string()),
            insert_after: Some("title".to_string()),
            reqd: true,
            default: None,
            hidden: false,
            read_only: false,
        },
    )
    .await
    .unwrap();

    let meta = crate::store::effective_meta(&pool, "Customizable Probe")
        .await
        .unwrap()
        .expect("the doctype is registered");
    let priority = meta
        .fields
        .iter()
        .find(|f| f.fieldname == "priority")
        .expect("the custom field is in the effective metadata");
    assert_eq!(priority.fieldtype, "Select");
    assert!(priority.reqd);

    // Reading it back through the same API returns what was written.
    let customization = read_customization(&pool, &admin(), "Customizable Probe")
        .await
        .unwrap();
    assert_eq!(customization.fields.len(), 1);
    assert_eq!(customization.fields[0].fieldname, "priority");

    // And removing it takes the field out of the effective metadata again.
    remove_custom_field(&pool, &admin(), "Customizable Probe", "priority")
        .await
        .unwrap();
    let meta = crate::store::effective_meta(&pool, "Customizable Probe")
        .await
        .unwrap()
        .unwrap();
    assert!(meta.fields.iter().all(|f| f.fieldname != "priority"));
}

/// A caller without `Manage` cannot read *or* write, so the customization
/// surface is not a read of another team's schema.
#[tokio::test]
async fn a_non_manager_is_refused_both_ways() {
    let pool = ferris_db::memory_db().await.unwrap();
    let mallory = user("mallory@local", &[]);
    assert!(read_customization(&pool, &mallory, "ToDo").await.is_err());
    assert!(
        save_custom_field(
            &pool,
            &mallory,
            "ToDo",
            &CustomFieldSpec {
                fieldname: "x".to_string(),
                label: None,
                fieldtype: "Data".to_string(),
                options: None,
                insert_after: None,
                reqd: false,
                default: None,
                hidden: false,
                read_only: false,
            },
        )
        .await
        .is_err()
    );
}

/// Export writes the customization into the app's world, in the same wire
/// shape the API returns, so the file is re-readable and versionable. An
/// unsafe app name must be refused *before* anything is created.
#[tokio::test]
async fn export_writes_the_customization_into_the_app_world() {
    let pool = ferris_db::memory_db().await.unwrap();
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
        name: "Exportable Probe".to_string(),
        module: "test".to_string(),
        fields: vec![ferris_schema::FieldMeta {
            fieldname: "title".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });
    save_custom_field(
        &pool,
        &admin(),
        "Exportable Probe",
        &CustomFieldSpec {
            fieldname: "note".to_string(),
            label: Some("Note".to_string()),
            fieldtype: "Data".to_string(),
            options: None,
            insert_after: Some("title".to_string()),
            reqd: false,
            default: None,
            hidden: false,
            read_only: false,
        },
    )
    .await
    .unwrap();
    save_property_setter(
        &pool,
        &admin(),
        "Exportable Probe",
        &PropertySetterSpec {
            fieldname: Some("note".to_string()),
            property: "placeholder".to_string(),
            value: "write here".to_string(),
        },
    )
    .await
    .unwrap();

    let root = temp_plugins("customization");

    let relative =
        export_customization_at(&pool, &admin(), "Exportable Probe", "demo", None, &root)
            .await
            .unwrap();
    assert_eq!(
        relative,
        "plugins/demo/customizations/Exportable-Probe.json"
    );

    let path = root.join("demo/customizations/Exportable-Probe.json");
    let text = tokio::fs::read_to_string(&path).await.unwrap();
    let exported: WorldCustomization = serde_json::from_str(&text).unwrap();
    assert_eq!(exported.doctype, "Exportable Probe");
    assert_eq!(exported.customization.fields.len(), 1);
    assert_eq!(exported.customization.fields[0].fieldname, "note");
    assert_eq!(exported.customization.properties.len(), 1);
    assert_eq!(exported.customization.properties[0].property, "placeholder");

    // Exporting again replaces the file instead of accumulating copies.
    export_customization_at(&pool, &admin(), "Exportable Probe", "demo", None, &root)
        .await
        .unwrap();
    let dir = root.join("demo/customizations");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

    // A name that is not a safe path segment is refused and creates nothing.
    assert!(
        export_customization_at(
            &pool,
            &admin(),
            "Exportable Probe",
            "../escape",
            None,
            &root,
        )
        .await
        .is_err()
    );
    assert!(!root.join("escape").exists());

    let _ = std::fs::remove_dir_all(&root);
}

/// A `Fields` scope writes the listed custom fields and nothing else: the
/// property setters of the same DocType stay out of the file, so the export is
/// the diff the user asked for.
#[tokio::test]
async fn a_fields_scope_exports_only_the_listed_custom_fields() {
    let pool = ferris_db::memory_db().await.unwrap();
    register_probe("Scoped Field Probe");
    add_field(&pool, "Scoped Field Probe", "alpha").await;
    add_field(&pool, "Scoped Field Probe", "beta").await;
    add_setter(&pool, "Scoped Field Probe", None, "title_field").await;
    add_setter(&pool, "Scoped Field Probe", Some("alpha"), "placeholder").await;

    let root = temp_plugins("scope-fields");
    let scope = ExportScope::Fields(vec!["alpha".to_string()]);
    export_customization_at(
        &pool,
        &admin(),
        "Scoped Field Probe",
        "demo",
        Some(&scope),
        &root,
    )
    .await
    .unwrap();

    let path = root.join("demo/customizations/Scoped-Field-Probe.json");
    let text = tokio::fs::read_to_string(&path).await.unwrap();
    let exported: WorldCustomization = serde_json::from_str(&text).unwrap();
    assert_eq!(exported.doctype, "Scoped Field Probe");
    assert_eq!(exported.customization.fields.len(), 1);
    assert_eq!(exported.customization.fields[0].fieldname, "alpha");
    assert!(exported.customization.properties.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

/// A `Properties` scope is the mirror image: the `(fieldname, property)` pairs
/// listed are written, custom fields are dropped, and a `DocType`-level setter
/// is addressed by `None`.
#[tokio::test]
async fn a_properties_scope_exports_only_the_listed_setters() {
    let pool = ferris_db::memory_db().await.unwrap();
    register_probe("Scoped Property Probe");
    add_field(&pool, "Scoped Property Probe", "alpha").await;
    add_setter(&pool, "Scoped Property Probe", None, "title_field").await;
    add_setter(&pool, "Scoped Property Probe", Some("alpha"), "placeholder").await;

    let root = temp_plugins("scope-properties");
    let scope = ExportScope::Properties(vec![(None, "title_field".to_string())]);
    export_customization_at(
        &pool,
        &admin(),
        "Scoped Property Probe",
        "demo",
        Some(&scope),
        &root,
    )
    .await
    .unwrap();

    let path = root.join("demo/customizations/Scoped-Property-Probe.json");
    let text = tokio::fs::read_to_string(&path).await.unwrap();
    let exported: WorldCustomization = serde_json::from_str(&text).unwrap();
    assert_eq!(exported.doctype, "Scoped Property Probe");
    assert!(exported.customization.fields.is_empty());
    assert_eq!(exported.customization.properties.len(), 1);
    assert!(exported.customization.properties[0].fieldname.is_none());
    assert_eq!(exported.customization.properties[0].property, "title_field");

    let _ = std::fs::remove_dir_all(&root);
}

/// A scope naming a field the DocType does not have selects nothing, and an
/// empty file would read as "no customization" on `ferris migrate`: the export
/// fails with a validation error instead of writing it.
#[tokio::test]
async fn an_unknown_scope_field_is_a_validation_error() {
    let pool = ferris_db::memory_db().await.unwrap();
    register_probe("Unknown Scope Probe");
    add_field(&pool, "Unknown Scope Probe", "alpha").await;

    let root = temp_plugins("scope-unknown");
    let scope = ExportScope::Fields(vec!["missing".to_string()]);
    let err = export_customization_at(
        &pool,
        &admin(),
        "Unknown Scope Probe",
        "demo",
        Some(&scope),
        &root,
    )
    .await
    .unwrap_err();

    match err {
        Error::Validation(ValidationError::Invalid(message)) => {
            assert!(
                message.contains("scope"),
                "the message must say the scope selected nothing: {message}"
            );
        }
        other => panic!("expected a validation error, got {other:?}"),
    }

    // Nothing was written, and not even the world directory was created.
    assert!(
        !root
            .join("demo/customizations/Unknown-Scope-Probe.json")
            .exists()
    );
    assert!(!root.join("demo").exists());

    let _ = std::fs::remove_dir_all(&root);
}
