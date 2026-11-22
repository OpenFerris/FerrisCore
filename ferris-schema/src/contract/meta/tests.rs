use super::*;

/// The declaration defaults to the form, so a doctype that says nothing
/// opens exactly what it opened before views were declarable.
#[test]
fn a_doctype_without_a_declaration_opens_the_form() {
    let meta = DynMeta::default();
    assert_eq!(meta.view, DocView::Form);
    assert_eq!(meta.dashboard, DashboardView::default());
}

/// A payload emitted before `view` existed must still deserialize: both
/// fields are additive and every new one defaults. The fixture is built
/// from today's shape with the two keys *removed*, so it cannot drift
/// from the rest of the contract, and it also pins the names they travel
/// under on the wire.
#[test]
fn a_payload_without_the_view_fields_still_deserializes() {
    let mut value = serde_json::to_value(DynMeta::default()).expect("serialises");
    let object = value.as_object_mut().expect("object payload");
    assert!(object.remove("view").is_some(), "the key is `view`");
    assert!(
        object.remove("dashboard").is_some(),
        "the key is `dashboard`"
    );

    let meta: DynMeta = serde_json::from_value(value).expect("older payload parses");
    assert_eq!(meta.view, DocView::Form);
    assert_eq!(meta.dashboard, DashboardView::default());
}

/// `snake_case` on the wire: the values are a public contract, and a
/// spelling change would silently un-declare every existing dashboard.
#[test]
fn the_view_declares_itself_on_the_wire_in_snake_case() {
    let json = serde_json::to_string(&DocView::Dashboard).expect("serialises");
    assert_eq!(json, "\"dashboard\"");
    let back: DocView = serde_json::from_str("\"custom\"").expect("parses");
    assert_eq!(back, DocView::Custom);
}

/// The declaration survives the only path the Desk gets it through: the
/// registry's metadata serialized and read back as the wire mirror. A
/// field renamed or dropped in that trip would silently un-declare every
/// dashboard while both sides kept compiling.
#[cfg(feature = "native")]
#[test]
fn a_registry_declaration_reaches_the_wire_mirror() {
    let registry_meta = crate::registry::DoctypeMeta {
        view: DocView::Dashboard,
        dashboard: DashboardView::link_field("panel"),
        ..crate::registry::DoctypeMeta::default()
    };
    let meta = DynMeta::from_serde_meta(&registry_meta).expect("mirror parses");
    assert_eq!(meta.view, DocView::Dashboard);
    assert_eq!(meta.dashboard, DashboardView::link_field("panel"));
}

/// H3: a `Secret` field is a string on the wire, so it needs no new
/// `DynField` member — but the registry → mirror trip has to preserve it,
/// or the Desk would render a plain text input and a credential would be
/// typed in clear. Also pins that the registry canonicalizes the name.
#[cfg(feature = "native")]
#[test]
fn a_secret_field_reaches_the_wire_mirror() {
    let registry_meta = crate::registry::DoctypeMeta {
        name: "Credential".to_string(),
        module: "Test".to_string(),
        fields: vec![
            crate::registry::FieldMeta::data("label"),
            crate::registry::FieldMeta::secret("client_secret"),
        ],
        ..crate::registry::DoctypeMeta::default()
    };
    let meta = DynMeta::from_serde_meta(&registry_meta).expect("mirror parses");
    let field = meta
        .fields
        .iter()
        .find(|field| field.fieldname == "client_secret")
        .expect("the secret field travelled");
    assert_eq!(field.fieldtype, "Secret");

    let registry = crate::registry::SchemaRegistry::new();
    registry.register(registry_meta);
    let stored = registry.get("Credential").expect("registered");
    let field = stored
        .fields
        .iter()
        .find(|field| field.fieldname == "client_secret")
        .expect("declared");
    assert_eq!(field.fieldtype, "Secret");
}

/// The four bindings a `dashboard` declaration can carry.
#[test]
fn a_declaration_resolves_to_its_binding() {
    let native = DashboardView::default().binding_for("SALES-1");
    assert_eq!(
        native,
        DashboardBinding::Direct {
            id: "SALES-1".to_string(),
            native: true
        }
    );

    let fixed = DashboardView::dashboard("ventas").binding_for("ANY");
    assert_eq!(
        fixed,
        DashboardBinding::Direct {
            id: "ventas".to_string(),
            native: false
        }
    );

    let wrapper = DashboardView::link_field("panel").binding_for("R-1");
    assert_eq!(
        wrapper,
        DashboardBinding::Link {
            field: "panel".to_string()
        }
    );

    let document = DashboardView::document().binding_for("R-1");
    assert_eq!(
        document,
        DashboardBinding::Document {
            format: DocumentFormat::default()
        }
    );
}

/// The wrapper wins when a declaration carries both: the field's value is
/// the dashboard *this* document chose, which is the more specific answer.
#[test]
fn a_link_field_wins_over_a_fixed_dashboard() {
    let both = DashboardView {
        link_field: Some("panel".to_string()),
        dashboard: Some("ventas".to_string()),
        ..DashboardView::default()
    };
    assert_eq!(
        both.binding_for("R-1"),
        DashboardBinding::Link {
            field: "panel".to_string()
        }
    );
}

/// A document declared to *be* a dashboard is one: the document binding
/// beats a fixed id riding on the same declaration (both are backing
/// declarations, and the document is the more specific one).
#[test]
fn the_document_binding_wins_over_a_fixed_dashboard() {
    let both = DashboardView {
        dashboard: Some("ventas".to_string()),
        ..DashboardView::document()
    };
    assert_eq!(
        both.binding_for("R-1"),
        DashboardBinding::Document {
            format: DocumentFormat::default()
        }
    );
}

/// And a wrapper still beats the document binding: a doctype that links
/// each document to some other panel says so with its field.
#[test]
fn a_link_field_wins_over_the_document_binding() {
    let both = DashboardView {
        link_field: Some("panel".to_string()),
        ..DashboardView::document()
    };
    assert_eq!(
        both.binding_for("R-1"),
        DashboardBinding::Link {
            field: "panel".to_string()
        }
    );
}

/// The format minimum (`widgets`/`layout`/`label`, plus the optional
/// `config`) reads under its canonical names when nothing is bound, each
/// bound name overrides its own field only, and an empty declaration means
/// the canonical field — never a field called "".
#[test]
fn bound_fields_fall_back_to_the_canonical_names() {
    let canonical = DocumentFormat::default();
    assert_eq!(canonical.widgets(), "widgets");
    assert_eq!(canonical.layout(), "layout");
    assert_eq!(canonical.label(), "label");
    assert_eq!(canonical.config(), "config");

    let renamed = DocumentFormat::default()
        .with_layout("panel_layout")
        .with_label("titulo");
    assert_eq!(renamed.layout(), "panel_layout");
    assert_eq!(renamed.label(), "titulo");
    // The unrenamed field keeps reading the canonical one.
    assert_eq!(renamed.widgets(), "widgets");
    assert_eq!(renamed.config(), "config");

    // A guest that maps an empty string cannot point at a field named "".
    let empty = DocumentFormat {
        widgets: Some(String::new()),
        layout: Some(String::new()),
        label: Some(String::new()),
        config: Some(String::new()),
    };
    assert_eq!(empty.widgets(), "widgets");
    assert_eq!(empty.layout(), "layout");
    assert_eq!(empty.label(), "label");
    assert_eq!(empty.config(), "config");
}

/// The `document` binding rides the same wire trip every other declaration
/// takes (registry → endpoint → Desk mirror): it appears when declared,
/// survives a payload serialized before it existed, and a field rename
/// survives the round trip intact.
#[test]
fn the_document_binding_survives_the_wire() {
    let declared = DashboardView::document_fields(
        DocumentFormat::default()
            .with_widgets("panel_widgets")
            .with_label("titulo"),
    );
    let json = serde_json::to_value(&declared).expect("serialises");
    assert_eq!(
        json["document"],
        serde_json::json!({
            "widgets": "panel_widgets",
            "layout": null,
            "label": "titulo",
            "config": null,
        }),
        "each field travels under its own key"
    );
    let back: DashboardView = serde_json::from_value(json).expect("parses back");
    assert_eq!(back, declared);

    // A payload from before the binding existed: no `document` key at all.
    let nullable = serde_json::json!({"link_field": null, "dashboard": null});
    let parsed: DashboardView = serde_json::from_value(nullable).expect("null fields parse");
    assert_eq!(parsed.document, None);
    assert_eq!(parsed, DashboardView::default());
}

/// The registry declaration reaches the Desk's mirror carrying the
/// `document` binding, the same trip that proves `view` and the wrapper
/// make it end to end.
#[cfg(feature = "native")]
#[test]
fn a_document_declaration_reaches_the_wire_mirror() {
    let declared = DashboardView::document_fields(DocumentFormat::default().with_layout("grid"));
    let registry_meta = crate::registry::DoctypeMeta {
        view: DocView::Dashboard,
        dashboard: declared.clone(),
        ..crate::registry::DoctypeMeta::default()
    };
    let meta = DynMeta::from_serde_meta(&registry_meta).expect("mirror parses");
    assert_eq!(meta.view, DocView::Dashboard);
    assert_eq!(meta.dashboard, declared);
}

/// `widgets_in` reads the bound field — renamed or canonical — out of a
/// document, and an unbound-format document's own `widgets` too: the
/// binding names the field, the parse policy is the shared one.
#[test]
fn the_document_binding_reads_its_bound_field() {
    let doc = serde_json::json!({
        "name": "PANEL-1",
        "titulo": "Planta norte",
        "mis_widgets": "[{\"id\":\"w1\",\"widget\":\"host.stat\"}]"
    });
    let format = DocumentFormat::default()
        .with_widgets("mis_widgets")
        .with_label("titulo");

    let widgets = format.widgets_in(&doc);
    assert_eq!(widgets.len(), 1, "read from the renamed field");
    assert_eq!(widgets[0].id, "w1");
    assert_eq!(
        doc.get(format.label()).and_then(|v| v.as_str()),
        Some("Planta norte")
    );

    // The canonical binding reads `widgets`, and a document without the
    // field has an empty dashboard rather than an error.
    let canonical = DocumentFormat::default();
    let plain = serde_json::json!({"widgets": [{"id": "w2", "widget": "host.kpi"}]});
    assert_eq!(canonical.widgets_in(&plain).len(), 1);
    assert!(canonical.widgets_in(&serde_json::json!({})).is_empty());
}

/// Every `FieldMeta` property the engine understands reaches the wire mirror.
///
/// The mirror used to be a hand-picked subset, so a customization that set
/// `no_copy`, `search_index` or a print width was effective in the engine and
/// invisible to the Desk. The trip is through the same JSON projection the REST
/// endpoint uses, which is what makes this test cover the endpoint too.
#[cfg(feature = "native")]
#[test]
fn every_field_property_reaches_the_wire_mirror() {
    let registry_meta = crate::registry::DoctypeMeta {
        name: "Parity".into(),
        module: "Test".into(),
        fields: vec![crate::registry::FieldMeta {
            fieldname: "amount".into(),
            label: Some("Amount".into()),
            fieldtype: "Currency".into(),
            options: Some("EUR".into()),
            default: Some("0".into()),
            reqd: true,
            read_only: true,
            hidden: true,
            depends_on: Some("doc.status == 'Open'".into()),
            description: Some("The amount".into()),
            in_list_view: true,
            in_standard_filter: true,
            in_global_search: true,
            in_preview: true,
            bold: true,
            print_hide: true,
            report_hide: true,
            permlevel: 2,
            unique: true,
            search_index: true,
            sort_options: true,
            allow_on_submit: true,
            no_copy: true,
            ignore_user_permissions: true,
            ignore_xss_filter: true,
            convert_to_user_timezone: true,
            columns: 6,
            fetch_from: Some("currency.symbol".into()),
            fetch_if_empty: true,
            print_width: Some("80px".into()),
            width: Some("120px".into()),
            placeholder: Some("0.00".into()),
            max_length: Some(32),
            max_value: Some("1000".into()),
            min_value: Some("0".into()),
            precision: Some(2),
            length: Some(16),
            read_only_on_load: true,
            non_negative: true,
            collapsible: true,
            collapsible_depends_on: Some("doc.show".into()),
            mandatory_depends_on: Some("doc.required".into()),
            read_only_depends_on: Some("doc.locked".into()),
            hide_border: true,
            in_filter: true,
            remember_last_selected_value: true,
            ignore_xss_filter_field: true,
            r#virtual: true,
            mask: true,
        }],
        ..crate::registry::DoctypeMeta::default()
    };
    let meta = DynMeta::from_serde_meta(&registry_meta).expect("mirror parses");
    let field = &meta.fields[0];
    assert_eq!(field.fieldname, "amount");
    assert_eq!(field.fieldtype, "Currency");
    assert_eq!(field.options.as_deref(), Some("EUR"));
    assert!(field.unique);
    assert!(field.r#virtual);
    assert!(field.mask);
    assert!(field.allow_on_submit);
    assert!(field.search_index);
    assert!(field.no_copy);
    assert!(field.in_global_search);
    assert!(field.in_preview);
    assert_eq!(field.permlevel, 2);
    assert_eq!(field.fetch_from.as_deref(), Some("currency.symbol"));
    assert_eq!(field.print_width.as_deref(), Some("80px"));
    assert_eq!(field.width.as_deref(), Some("120px"));
    assert_eq!(field.precision, Some(2));
}

/// The DocType flags a Desk needs for the listed features travel too: tree,
/// quick entry, tracked changes, the generic field roles and the `allow-*`
/// surface.
#[cfg(feature = "native")]
#[test]
fn doctype_flags_reach_the_wire_mirror() {
    let registry_meta = crate::registry::DoctypeMeta {
        name: "Treeish".into(),
        module: "Test".into(),
        is_tree: true,
        track_changes: true,
        quick_entry: true,
        workflow: Some("Approval".into()),
        image_field: Some("photo".into()),
        subject_field: Some("subject".into()),
        sender_field: Some("from".into()),
        recipient_field: Some("to".into()),
        content_field: Some("body".into()),
        color_field: Some("color".into()),
        icon_field: Some("icon".into()),
        allow_email: true,
        allow_attach: true,
        allow_import: true,
        allow_export: true,
        allow_share: true,
        in_create: true,
        in_dialog: true,
        in_preview: true,
        in_global_search: true,
        in_sidebar: true,
        in_dashboard: true,
        ..crate::registry::DoctypeMeta::default()
    };
    let meta = DynMeta::from_serde_meta(&registry_meta).expect("mirror parses");
    assert!(meta.is_tree);
    assert!(meta.track_changes);
    assert!(meta.quick_entry);
    assert_eq!(meta.workflow.as_deref(), Some("Approval"));
    assert_eq!(meta.image_field.as_deref(), Some("photo"));
    assert!(meta.allow_email && meta.allow_attach && meta.allow_import);
    assert!(meta.allow_export && meta.allow_share);
    assert!(meta.in_create && meta.in_dialog && meta.in_preview);
    assert!(meta.in_global_search && meta.in_sidebar && meta.in_dashboard);
}
