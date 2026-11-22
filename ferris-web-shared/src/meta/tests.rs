//! Metadata helpers: columns, labels, view modes, filtering and the create
//! affordance.

use super::*;
use crate::meta_types::{DynField, DynIndex, DynMeta, DynPermissionRule};

fn meta_with(flags: &[(&str, bool)]) -> DynMeta {
    let mut m = DynMeta {
        name: "Task".into(),
        module: "test".into(),
        in_list_view: true,
        ..Default::default()
    };
    for (k, v) in flags {
        match *k {
            "in_kanban" => m.in_kanban = *v,
            "in_calendar" => m.in_calendar = *v,
            "in_tree_view" => m.in_tree_view = *v,
            "in_gantt" => m.in_gantt = *v,
            "in_report_view" => m.in_report_view = *v,
            "read_only" => m.read_only = *v,
            "is_virtual" => m.is_virtual = *v,
            "is_single" => m.is_single = *v,
            "is_tree" => m.is_tree = *v,
            _ => {}
        }
    }
    m
}

/// A doctype with fields that declare nothing for the list shows only the
/// name, not every field it happens to have.
#[test]
fn list_shows_only_declared_columns() {
    let meta = DynMeta {
        name: "Task".into(),
        fields: vec![
            field("subject", false),
            field("status", true),
            field("notes", false),
        ],
        ..Default::default()
    };

    let names: Vec<String> = list_columns(&meta)
        .into_iter()
        .map(|c| c.fieldname)
        .collect();
    // `name` is the fallback, `status` is the only declared column, and
    // `creation` comes along as a timestamp. `modified` is left to the
    // table's own fixed column.
    assert_eq!(names, vec!["name", "status", "creation"]);
}

/// An index says the app queries these fields, which is reason enough to
/// show them even when the doctype never flagged them for the list.
#[test]
fn list_includes_fields_covered_by_an_index() {
    let meta = DynMeta {
        name: "Task".into(),
        fields: vec![field("subject", false), field("status", false)],
        indexes: vec![DynIndex {
            fields: vec!["status".into(), "owner".into()],
            unique: false,
        }],
        ..Default::default()
    };

    let names: Vec<String> = list_columns(&meta)
        .into_iter()
        .map(|c| c.fieldname)
        .collect();
    // `status` is indexed and declared, so it keeps its declared position
    // and type; `owner` is indexed but undeclared, so it is appended.
    assert_eq!(names, vec!["name", "status", "creation", "owner"]);
    let status = list_columns(&meta)
        .into_iter()
        .find(|c| c.fieldname == "status")
        .unwrap();
    assert_eq!(status.fieldtype, "Data");
}

/// A field that is both flagged and indexed appears once, not twice.
#[test]
fn list_never_repeats_a_column() {
    let meta = DynMeta {
        name: "Task".into(),
        fields: vec![field("status", true)],
        indexes: vec![DynIndex {
            fields: vec!["status".into(), "creation".into()],
            unique: false,
        }],
        ..Default::default()
    };

    let names: Vec<String> = list_columns(&meta)
        .into_iter()
        .map(|c| c.fieldname)
        .collect();
    assert_eq!(names, vec!["name", "status", "creation"]);
}

/// The creation timestamp the engine stamps is always listable, even when
/// the doctype declares no fields at all.
#[test]
fn timestamps_survive_a_doctype_with_no_listable_fields() {
    let meta = DynMeta {
        name: "Task".into(),
        ..Default::default()
    };
    let names: Vec<String> = list_columns(&meta)
        .into_iter()
        .map(|c| c.fieldname)
        .collect();
    assert_eq!(names, vec!["name", "creation"]);
}

/// Filters are not restricted to the list's visible columns: `list_view` is
/// only the seed for the table, and a field that never appears there must
/// still be filterable. Layout breaks are the one exception because they
/// carry no value.
#[test]
fn filter_fields_cover_every_stored_field_not_just_the_list_view() {
    let meta = DynMeta {
        name: "Task".into(),
        fields: vec![
            field("subject", false),
            field("status", true),
            DynField {
                fieldname: "details".into(),
                fieldtype: "Section Break".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let names: Vec<String> = filter_fields(&meta)
        .into_iter()
        .map(|c| c.fieldname)
        .collect();
    assert_eq!(
        names,
        vec![
            "name",
            "subject",
            "status",
            "owner",
            "docstatus",
            "creation",
            "modified"
        ]
    );
    assert!(!names.contains(&"details".to_string()));
}

/// A doctype that declares one of the engine fields keeps its own label,
/// position and type instead of being duplicated by the system pass.
#[test]
fn filter_fields_do_not_duplicate_declared_system_fields() {
    let mut creation = field("creation", false);
    creation.label = Some("Creado a mano".into());
    let meta = DynMeta {
        name: "Task".into(),
        fields: vec![creation],
        ..Default::default()
    };

    let cols = filter_fields(&meta);
    let creation_cols: Vec<&Column> = cols.iter().filter(|c| c.fieldname == "creation").collect();
    assert_eq!(creation_cols.len(), 1);
    assert_eq!(creation_cols[0].label, "Creado a mano");
    // The declared field keeps its declared (first) position.
    assert_eq!(cols[0].fieldname, "name");
    assert_eq!(cols[1].fieldname, "creation");
}

#[test]
fn filters_compare_across_the_value_kinds() {
    let row = serde_json::json!({
        "name": "TASK-0001",
        "qty": 7,
        "done": false,
        "tags": ["a", "b"],
    });

    let f = |field: &str, op: crate::types::FilterOp, value: &str| crate::types::Filter {
        field: field.into(),
        op,
        value: value.into(),
        or: false,
    };

    // A number and a bool are coerced to text before comparing.
    assert!(matches_filter(
        &row,
        &f("qty", crate::types::FilterOp::Equals, "7")
    ));
    assert!(!matches_filter(
        &row,
        &f("qty", crate::types::FilterOp::Equals, "8")
    ));
    assert!(matches_filter(
        &row,
        &f("done", crate::types::FilterOp::Equals, "false")
    ));

    // `Like` ignores case on both sides.
    assert!(matches_filter(
        &row,
        &f("name", crate::types::FilterOp::Like, "task")
    ));

    // A missing field is empty, not an error.
    assert!(matches_filter(
        &row,
        &f("nope", crate::types::FilterOp::Equals, "")
    ));
    assert!(!matches_filter(
        &row,
        &f("nope", crate::types::FilterOp::Equals, "x")
    ));

    // `In` splits on commas and trims each part.
    assert!(matches_filter(
        &row,
        &f("name", crate::types::FilterOp::In, "x, TASK-0001 ,y")
    ));

    // Ranges compare numerically when both sides parse as numbers.
    assert!(matches_filter(
        &row,
        &f("qty", crate::types::FilterOp::Gte, "7")
    ));
    assert!(matches_filter(
        &row,
        &f("qty", crate::types::FilterOp::Lt, "8")
    ));
    assert!(!matches_filter(
        &row,
        &f("qty", crate::types::FilterOp::Gt, "7")
    ));
    // 10 is not <= 9 numerically, which is the point of coercing.
    assert!(matches_filter(
        &row,
        &f("qty", crate::types::FilterOp::Lt, "10")
    ));
}

#[test]
fn a_row_passes_only_when_every_filter_passes() {
    let rows = vec![
        serde_json::json!({ "name": "TASK-0001", "status": "Open" }),
        serde_json::json!({ "name": "TASK-0002", "status": "Done" }),
    ];
    let status = crate::types::Filter {
        field: "status".into(),
        op: crate::types::FilterOp::Equals,
        value: "Open".into(),
        or: false,
    };
    let kept = apply_filters(rows, std::slice::from_ref(&status));
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0]["name"], "TASK-0001");
}

/// No filters is a no-op, not "match nothing": the rows come back as they
/// went in. The old assertion used an empty input, so the early-return
/// branch it claimed to test could never be observed (WS9c).
#[test]
fn no_filters_returns_every_row() {
    let rows = vec![
        serde_json::json!({ "name": "TASK-0001" }),
        serde_json::json!({ "name": "TASK-0002" }),
    ];
    assert_eq!(apply_filters(rows.clone(), &[]).len(), rows.len());
}

/// WS9a: `or: true` starts a new OR group, so a row that satisfies one
/// group but not the other is kept — the engine's rule.
#[test]
fn an_or_filter_starts_a_new_group() {
    let rows = vec![
        serde_json::json!({ "name": "A", "status": "Open" }),
        serde_json::json!({ "name": "B", "status": "Done" }),
        serde_json::json!({ "name": "C", "status": "Other" }),
    ];
    let filter = |field: &str, value: &str, or: bool| crate::types::Filter {
        field: field.into(),
        op: crate::types::FilterOp::Equals,
        value: value.into(),
        or,
    };
    // (status = Open) OR (status = Done)
    let filters = vec![
        filter("status", "Open", false),
        filter("status", "Done", true),
    ];
    let kept = apply_filters(rows, &filters);
    let names: Vec<&str> = kept.iter().filter_map(|r| r["name"].as_str()).collect();
    assert_eq!(names, vec!["A", "B"]);
}

/// WS4: an acronym is one word, not one letter per capital.
#[test]
fn prettify_keeps_acronyms_together() {
    assert_eq!(prettify("total_amount"), "Total Amount");
    assert_eq!(prettify("api_url"), "Api Url");
    assert_eq!(prettify("id"), "Id");
    assert_eq!(prettify("URLValue"), "URL Value");
}

/// WS1: the label helper reads the field, and nothing else.
#[test]
fn label_of_prefers_the_declared_label() {
    let mut declared = field("total_amount", false);
    declared.label = Some("Total".into());
    assert_eq!(label_of(&declared), "Total");

    let mut undeclared = field("total_amount", false);
    undeclared.label = None;
    assert_eq!(label_of(&undeclared), "Total Amount");
}

fn field(fieldname: &str, in_list_view: bool) -> DynField {
    DynField {
        fieldname: fieldname.into(),
        label: Some(fieldname.into()),
        fieldtype: "Data".into(),
        in_list_view,
        ..Default::default()
    }
}

#[test]
fn view_modes_are_definition_driven() {
    // In-list is the entry mode; extra modes come only from the definition.
    assert_eq!(
        view_modes(&meta_with(&[]), "test", "Task")
            .iter()
            .map(|v| v.id)
            .collect::<Vec<_>>(),
        vec!["list"]
    );
    let ids: Vec<&str> = view_modes(&meta_with(&[("in_kanban", true)]), "test", "Task")
        .iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(ids, vec!["list", "kanban"]);
    let ids: Vec<&str> = view_modes(
        &meta_with(&[("in_kanban", true), ("in_calendar", true)]),
        "test",
        "Task",
    )
    .iter()
    .map(|v| v.id)
    .collect();
    assert_eq!(ids, vec!["list", "kanban", "calendar"]);

    let ids: Vec<&str> = view_modes(
        &meta_with(&[
            ("in_kanban", true),
            ("in_calendar", true),
            ("in_tree_view", true),
            ("in_gantt", true),
            ("in_report_view", true),
        ]),
        "test",
        "Task",
    )
    .iter()
    .map(|v| v.id)
    .collect();
    assert_eq!(
        ids,
        vec!["list", "kanban", "calendar", "tree", "gantt", "report"],
        "every declared collection view gets a mode, in reading order"
    );
}

/// `is_tree` is Frappe's source of truth: a doctype declared with it offers the
/// tree view even when the older `in_tree_view` flag is absent. The two are
/// ORed so neither a macro doctype nor a stored one loses the view.
#[test]
fn a_tree_doctype_offers_the_tree_view_without_the_legacy_flag() {
    let ids: Vec<&str> = view_modes(&meta_with(&[("is_tree", true)]), "test", "Account")
        .iter()
        .map(|v| v.id)
        .collect();
    assert!(ids.contains(&"tree"), "{ids:?}");
}

/// Every collection view lives under the doctype's own URL, so switching views
/// never leaves the module/doctype the user was looking at.
#[test]
fn every_collection_mode_uses_the_doctype_route() {
    let modes = view_modes(
        &meta_with(&[
            ("in_kanban", true),
            ("in_calendar", true),
            ("in_tree_view", true),
            ("in_gantt", true),
            ("in_report_view", true),
        ]),
        "test",
        "Sales Invoice",
    );
    let routes: Vec<&str> = modes.iter().map(|mode| mode.route.as_str()).collect();
    assert_eq!(
        routes,
        [
            "/desk/test/Sales%20Invoice",
            "/desk/test/Sales%20Invoice/kanban",
            "/desk/test/Sales%20Invoice/calendar",
            "/desk/test/Sales%20Invoice/tree",
            "/desk/test/Sales%20Invoice/gantt",
            "/desk/test/Sales%20Invoice/report",
        ]
    );
}

/// The index opens the view the doctype declares, and a declaration whose view
/// was turned off degrades to the list.
#[test]
fn the_default_view_is_declared_and_degrades_to_the_list() {
    use crate::meta_types::CollectionView;

    let mut meta = meta_with(&[("in_kanban", true)]);
    assert_eq!(default_view(&meta), CollectionView::List);
    meta.default_view = Some(CollectionView::Kanban);
    assert_eq!(default_view(&meta), CollectionView::Kanban);
    // `in_kanban` off: the stale declaration cannot offer a board.
    meta.in_kanban = false;
    assert_eq!(default_view(&meta), CollectionView::List);
}

#[test]
fn a_report_view_is_a_declared_mode_with_the_doctype_route() {
    let modes = view_modes(
        &meta_with(&[("in_report_view", true)]),
        "test",
        "Sales Invoice",
    );
    let report = modes
        .iter()
        .find(|mode| mode.id == "report")
        .expect("report mode");
    assert_eq!(report.label, "Informe");
    assert_eq!(report.route, "/desk/test/Sales%20Invoice/report");
}

#[test]
fn view_modes_build_routes() {
    let modes = view_modes(&meta_with(&[("in_kanban", true)]), "test", "Task");
    assert_eq!(modes[0].route, "/desk/test/Task");
    assert_eq!(modes[1].route, "/desk/test/Task/kanban");
}

/// A Single offers exactly one mode (its form) and no "create" affordance,
/// even when its definition turns other views on: there is no second
/// document to list, filter or create.
#[test]
fn single_has_only_its_form_mode_and_no_create() {
    let modes = view_modes(
        &meta_with(&[("is_single", true), ("in_kanban", true)]),
        "test",
        "System Settings",
    );
    assert_eq!(modes.len(), 1);
    assert_eq!(modes[0].id, "single");
    // A doctype name is a path segment: the space is encoded, so the route
    // reads back as one segment and the form link resolves.
    assert_eq!(modes[0].route, "/desk/test/System%20Settings");

    assert!(!can_create(
        &meta_with(&[("is_single", true)]),
        &["System Manager".into()]
    ));
}

#[test]
fn can_create_respects_capability_and_permissions() {
    // Doctype disabled → no create.
    assert!(!can_create(
        &meta_with(&[("read_only", true)]),
        &["System Manager".into()]
    ));
    assert!(!can_create(
        &meta_with(&[("is_virtual", true)]),
        &["System Manager".into()]
    ));
    // System Manager passes regardless.
    assert!(can_create(&meta_with(&[]), &["System Manager".into()]));
    // A matching rule with create grants it; a rule without does not.
    let rule = |role: &str, create: bool| DynPermissionRule {
        role: role.into(),
        create,
        ..Default::default()
    };
    let mut m = meta_with(&[]);
    m.permissions = vec![rule("User", true)];
    assert!(can_create(&m, &["User".into()]));
    assert!(!can_create(&m, &["Guest".into()]));
    m.permissions = vec![rule("User", false)];
    assert!(!can_create(&m, &["User".into()]));
}
