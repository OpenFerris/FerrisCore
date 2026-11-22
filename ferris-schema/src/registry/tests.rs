use super::*;
use crate::contract::meta::{DashboardView, DocView};

fn named(name: &str, module: &str) -> DoctypeMeta {
    DoctypeMeta {
        name: name.to_string(),
        module: module.to_string(),
        ..Default::default()
    }
}

/// Re-registering is the normal case — the `#[doctype]` macro does it on
/// first access, `register_core_doctypes` does it on every boot, a plugin
/// reload does it again — and `list_by_module` used to append
/// unconditionally, so a DocType registered twice was listed twice under its
/// module while `list`, reading the map, showed it once.
#[test]
fn re_registering_does_not_duplicate_the_module_index() {
    let registry = SchemaRegistry::new();
    registry.register(named("Invoice", "Accounts"));
    registry.register(named("Invoice", "Accounts"));
    registry.register(named("Invoice", "Accounts"));

    assert_eq!(
        registry.list_by_module("Accounts"),
        vec!["Invoice".to_string()]
    );
    assert_eq!(registry.list().len(), 1);
    assert_eq!(registry.count(), 1);
}

/// Same name, different module: the old index entry has to go, or the DocType
/// stays reachable under a module it no longer belongs to.
#[test]
fn moving_module_does_not_leave_a_stale_index_entry() {
    let registry = SchemaRegistry::new();
    registry.register(named("Invoice", "Accounts"));
    registry.register(named("Invoice", "Selling"));

    assert!(
        registry.list_by_module("Accounts").is_empty(),
        "{:?}",
        registry.list_by_module("Accounts")
    );
    assert_eq!(
        registry.list_by_module("Selling"),
        vec!["Invoice".to_string()]
    );
}

/// Two DocTypes sharing a module both stay listed, in registration order.
#[test]
fn distinct_doctypes_in_one_module_are_both_indexed() {
    let registry = SchemaRegistry::new();
    registry.register(named("Invoice", "Accounts"));
    registry.register(named("Payment Entry", "Accounts"));
    assert_eq!(
        registry.list_by_module("Accounts"),
        vec!["Invoice".to_string(), "Payment Entry".to_string()]
    );
}

#[test]
fn test_registry() {
    let registry = SchemaRegistry::new();

    let meta = DoctypeMeta {
        name: "Test".to_string(),
        module: "test".to_string(),
        description: None,
        fields: vec![],
        permissions: vec![],
        naming_series: "TEST-.#####".to_string(),
        naming_type: None,
        naming_field: None,
        workflow: None,
        is_submittable: false,
        is_tree: false,
        is_virtual: false,
        is_single: false,
        is_child_table: false,
        view: DocView::default(),
        dashboard: DashboardView::default(),
        default_view: None,
        kanban_group_field: None,
        kanban_card_fields: vec![],
        calendar_end_field: None,
        calendar_title_field: None,
        tree_parent_field: None,
        tree_label_field: None,
        tree_expanded_levels: None,
        gantt_start_field: None,
        gantt_end_field: None,
        gantt_dependency_field: None,
        gantt_progress_field: None,
        track_changes: false,
        track_views: false,
        allow_rename: true,
        allow_copy: true,
        allow_delete: true,
        allow_print: true,
        allow_email: true,
        allow_attach: true,
        allow_import: true,
        allow_export: true,
        allow_share: true,
        allow_submit: false,
        allow_cancel: false,
        allow_amend: false,
        indexes: vec![],
        search_fields: vec![],
        sort_field: None,
        sort_order: None,
        image_field: None,
        title_field: None,
        subject_field: None,
        sender_field: None,
        recipient_field: None,
        content_field: None,
        date_field: None,
        color_field: None,
        icon_field: None,
        custom: false,
        beta: false,
        hidden: false,
        read_only: false,
        quick_entry: false,
        in_create: true,
        in_list_view: true,
        in_dialog: true,
        in_preview: true,
        in_global_search: true,
        in_sidebar: true,
        in_dashboard: true,
        in_calendar: true,
        in_kanban: true,
        in_gantt: true,
        in_tree_view: true,
        in_image_view: true,
        in_file_view: true,
        in_report_view: true,
        in_print_view: true,
        in_email_view: true,
        in_communication_view: true,
        in_assignment_view: true,
        in_timeline_view: true,
        in_activity_log: true,
        in_version_log: true,
        in_comment_log: true,
        in_share_log: true,
        in_permission_log: true,
        in_workflow_log: true,
        in_audit_log: true,
        in_error_log: true,
        in_scheduler_log: true,
        in_background_job_log: true,
        in_webhook_log: true,
        in_integration_log: true,
        in_oauth_log: true,
        in_ldap_log: true,
        in_saml_log: true,
        in_mfa_log: true,
        in_session_log: true,
        in_login_log: true,
        in_access_log: true,
        in_api_log: true,
        in_file_access_log: true,
        in_print_log: true,
        in_email_log: true,
        in_sms_log: true,
        in_notification_log: true,
        in_like_log: true,
        in_follow_log: true,
        in_tag_log: true,
        in_view_log: true,
        in_search_log: true,
        in_report_log: true,
        in_dashboard_log: true,
        in_workspace_log: true,
        in_desk_log: true,
        in_mobile_log: true,
        in_offline_log: true,
    };

    registry.register(meta);
    assert!(registry.has("Test"));
    assert_eq!(registry.get("Test").unwrap().name, "Test");
}

#[test]
fn json_indexed_fields_collects_single_field_json_indexes() {
    let meta = DoctypeMeta {
        name: "SolmecRead".to_string(),
        indexes: vec![
            IndexMeta {
                name: "idx_reads_device".to_string(),
                fields: vec!["device_id".to_string()],
                unique: false,
                index_type: 0,
            },
            IndexMeta {
                name: "idx_reads_device_ts".to_string(),
                fields: vec!["device_id".to_string(), "ts".to_string()],
                unique: false,
                index_type: 0,
            },
            IndexMeta {
                name: "idx_reads_status".to_string(),
                fields: vec!["status".to_string()],
                unique: false,
                index_type: 0,
            },
            IndexMeta {
                name: "idx_owner".to_string(),
                fields: vec!["owner".to_string()],
                unique: false,
                index_type: 0,
            },
        ],
        ..Default::default()
    };
    let fields = meta.json_indexed_fields();
    // `owner` is a column, so not returned; the mixed composite `idx_reads_device_ts`
    // is unsupported, so only single blob fields.
    assert_eq!(fields, vec!["device_id".to_string(), "status".to_string()]);
    // Duplicates are coalesced: the same field declared twice would appear once.
}

/// Every registered `DocType` carries the `tags` standard field, without
/// declaring it. Registration is the single choke point, so this covers the
/// core catalogue and plugin declarations alike.
#[test]
fn registering_adds_the_tags_standard_field() {
    let registry = SchemaRegistry::new();
    registry.register(named("Invoice", "Accounts"));

    let meta = registry.get("Invoice").expect("registered");
    let tags = meta
        .fields
        .iter()
        .find(|f| f.fieldname == TAGS_FIELD)
        .expect("every DocType has a `tags` field");
    assert_eq!(tags.fieldtype, "Data");
    assert_eq!(tags.label.as_deref(), Some("Tags"));
}

/// Registering twice must not duplicate the standard field — the same
/// idempotence problem the module index had.
#[test]
fn re_registering_does_not_duplicate_the_tags_field() {
    let registry = SchemaRegistry::new();
    registry.register(named("Invoice", "Accounts"));
    registry.register(named("Invoice", "Accounts"));

    let meta = registry.get("Invoice").expect("registered");
    let count = meta
        .fields
        .iter()
        .filter(|f| f.fieldname == TAGS_FIELD)
        .count();
    assert_eq!(count, 1, "{:?}", meta.fields);
}

/// A declaration keeps the last word: a `DocType` that already declares
/// `tags` (a different fieldtype, label or flags) is not overwritten.
#[test]
fn a_declared_tags_field_is_not_overwritten() {
    let registry = SchemaRegistry::new();
    let declared = FieldMeta {
        fieldname: TAGS_FIELD.to_string(),
        label: Some("Etiquetas".to_string()),
        fieldtype: "Small Text".to_string(),
        hidden: true,
        ..Default::default()
    };
    let meta = DoctypeMeta {
        name: "Invoice".to_string(),
        module: "Accounts".to_string(),
        fields: vec![declared],
        ..Default::default()
    };
    registry.register(meta);

    let got = registry.get("Invoice").expect("registered");
    assert_eq!(got.fields.len(), 1, "{:?}", got.fields);
    assert_eq!(got.fields[0].label.as_deref(), Some("Etiquetas"));
    assert_eq!(got.fields[0].fieldtype, "Small Text");
    assert!(got.fields[0].hidden);
}

// ---------------------------------------------------------------------------
// H4 — app DocTypes cannot shadow the framework or another app
// ---------------------------------------------------------------------------

fn assert_reserved(result: ferris_core::Result<()>, name: &str) {
    match result {
        Err(ferris_core::Error::Schema(ferris_core::SchemaError::ReservedDocType(got))) => {
            assert_eq!(got, name)
        }
        other => panic!("expected ReservedDocType({name}), got {other:?}"),
    }
}

/// The security-critical names are refused even before the framework crate has
/// registered them in this process, which is the window a plugin could exploit
/// by being described first.
#[test]
fn reserved_names_are_refused_before_the_framework_registers_them() {
    let registry = SchemaRegistry::new();
    for name in ["User", "Role", "DocPerm", "Session", "DocType", "File"] {
        assert_reserved(registry.try_register_app("crm", named(name, "crm")), name);
    }
    // Normalization closes the whitespace/case spelling of the same name.
    assert_reserved(
        registry.try_register_app("crm", named("  user ", "crm")),
        "user",
    );
    assert_reserved(
        registry.try_register_app("crm", named("ROLE", "crm")),
        "ROLE",
    );
}

/// A framework registration wins over an app with the same name, and the app
/// cannot replace it later either.
#[test]
fn an_app_cannot_replace_a_framework_doctype() {
    let registry = SchemaRegistry::new();
    registry.register(named("Invoice", "Accounts"));
    assert_reserved(
        registry.try_register_app("crm", named("Invoice", "crm")),
        "Invoice",
    );
    assert_eq!(
        registry.get("Invoice").expect("kept").module,
        "Accounts",
        "the framework metadata must not be replaced"
    );
}

/// The first app owns a name; another app is refused, the owner may hot-reload
/// it, and the host-supplied identity — not the guest's `module` — is stored.
#[test]
fn app_ownership_is_exclusive_and_the_host_sets_the_module() {
    let registry = SchemaRegistry::new();
    registry
        .try_register_app("crm", named("Ticket", "crm"))
        .expect("the first app registers a new name");

    match registry.try_register_app("shop", named("Ticket", "shop")) {
        Err(ferris_core::Error::Schema(ferris_core::SchemaError::DocTypeOwnedBy {
            name,
            owner,
        })) => {
            assert_eq!(name, "Ticket");
            assert_eq!(owner, "crm");
        }
        other => panic!("expected DocTypeOwnedBy, got {other:?}"),
    }

    // A malicious `module` in the guest metadata is overwritten by the app the
    // host knows it is talking to.
    let mut reload = named("Ticket", "Auth");
    reload.fields = vec![FieldMeta::new("subject", "Data")];
    registry
        .try_register_app("crm", reload)
        .expect("the owner may hot-reload");
    let stored = registry.get("Ticket").expect("registered");
    assert_eq!(stored.module, "crm");
    assert_eq!(stored.fields.len(), 2, "its field plus the standard `tags`");
}

/// `document_table` strips whitespace, so `ToDo` and `To Do` are one physical
/// table. Two registry entries pointing at it would make them overwrite each
/// other's rows.
#[test]
fn a_doctype_cannot_collide_on_the_physical_table() {
    let registry = SchemaRegistry::new();
    registry.register(named("ToDo", "Desk"));
    match registry.try_register_app("crm", named("To Do", "crm")) {
        Err(ferris_core::Error::Schema(ferris_core::SchemaError::DocTypeTableCollision {
            name,
            other,
            table,
        })) => {
            assert_eq!(name, "To Do");
            assert_eq!(other, "ToDo");
            assert_eq!(table, "tabToDo");
        }
        other => panic!("expected DocTypeTableCollision, got {other:?}"),
    }
}

/// Once the framework's own DocTypes are registered, the origin check protects
/// all of them — not just the static reserved list.
#[test]
fn every_declared_framework_doctype_is_protected_by_origin() {
    let registry = SchemaRegistry::new();
    let metas = crate::doctypes::doctype_metas();
    for meta in &metas {
        registry.register(meta.clone());
    }
    for meta in metas {
        let result = registry.try_register_app("crm", meta.clone());
        assert!(
            result.is_err(),
            "app replaced framework DocType {}: {result:?}",
            meta.name
        );
    }
}

// ---------------------------------------------------------------------------
// Partial metadata: `#[serde(default)]` on the structs
// ---------------------------------------------------------------------------

/// A DocType JSON written by an app may omit every property it does not need.
/// The struct-level default is what makes a minimal declaration parse; without
/// it, adding a field to `DoctypeMeta` would break every stored declaration.
#[test]
fn a_partial_doctype_json_defaults_the_missing_properties() {
    let meta: DoctypeMeta = serde_json::from_str(
        r#"{"name":"Ticket","module":"crm","fields":[{"fieldname":"subject","fieldtype":"Data"}]}"#,
    )
    .expect("a minimal declaration parses");
    assert_eq!(meta.name, "Ticket");
    assert_eq!(meta.module, "crm");
    assert_eq!(meta.fields.len(), 1);
    assert_eq!(meta.fields[0].fieldname, "subject");
    assert_eq!(meta.fields[0].fieldtype, "Data");
    assert!(!meta.is_submittable);
    assert!(meta.permissions.is_empty());
    assert_eq!(meta.view, DocView::Form);
    assert_eq!(meta.dashboard, DashboardView::default());
}

/// A field JSON is equally partial: only `fieldname` and `fieldtype` are
/// required by the schema, and the validation step below rejects a missing one
/// with a named error instead of storing an unusable field.
#[test]
fn a_partial_field_json_defaults_the_missing_properties() {
    let field: FieldMeta = serde_json::from_str(r#"{"fieldname":"phone","fieldtype":"Phone"}"#)
        .expect("a minimal field parses");
    assert_eq!(field.fieldname, "phone");
    assert_eq!(field.fieldtype, "Phone");
    assert!(!field.reqd);
    assert!(!field.mask);
    assert_eq!(field.columns, 0);
    assert_eq!(field.permlevel, 0);
}

/// The validation step is where a partial declaration is made complete: a
/// missing name, an empty fieldname, a duplicate or an unknown fieldtype is a
/// named error, never a DocType with a broken field list.
#[test]
fn app_doctypes_are_validated_before_registration() {
    let registry = SchemaRegistry::new();

    let mut empty_name = named("", "crm");
    empty_name.fields = vec![FieldMeta::new("title", "Data")];
    assert!(registry.validate_app_doctype("crm", &empty_name).is_err());

    let mut empty_field = named("Ticket", "crm");
    empty_field.fields = vec![FieldMeta::new("", "Data")];
    assert!(registry.validate_app_doctype("crm", &empty_field).is_err());

    let mut unknown = named("Ticket", "crm");
    unknown.fields = vec![FieldMeta::new("title", "Sparkle")];
    assert!(registry.validate_app_doctype("crm", &unknown).is_err());

    let mut duplicate = named("Ticket", "crm");
    duplicate.fields = vec![
        FieldMeta::new("title", "Data"),
        FieldMeta::new("title", "Data"),
    ];
    assert!(registry.validate_app_doctype("crm", &duplicate).is_err());

    let mut naming = named("Ticket", "crm");
    naming.naming_type = Some(crate::contract::meta::NamingType::Field);
    assert!(registry.validate_app_doctype("crm", &naming).is_err());

    let mut valid = named("Ticket", "crm");
    valid.fields = vec![FieldMeta::new("title", "Data")];
    registry
        .validate_app_doctype("crm", &valid)
        .expect("a clean declaration is accepted");
}
