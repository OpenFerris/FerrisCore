use super::registry::reset_registry;

use super::*;
use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap()
}

fn reset() {
    reset_registry();
}

fn spec(name: &str, module: &str, items: Vec<WorkspaceItemSpec>) -> WorkspaceSpec {
    WorkspaceSpec {
        name: name.into(),
        label: String::new(),
        module: module.into(),
        icon: None,
        folder: None,
        items,
    }
}

fn folder_spec(name: &str) -> WorkspaceFolderSpec {
    WorkspaceFolderSpec {
        name: name.into(),
        label: String::new(),
        icon: None,
    }
}

fn dt(doctype: &str) -> WorkspaceItemSpec {
    WorkspaceItemSpec {
        label: doctype.into(),
        doctype: Some(doctype.into()),
        ..Default::default()
    }
}

#[test]
fn kind_detection() {
    let _g = lock();
    reset();
    assert_eq!(dt("Task").kind(), WorkspaceItemKind::Doctype);
    let url = WorkspaceItemSpec {
        label: "Docs".into(),
        href: Some("https://x".into()),
        ..Default::default()
    };
    assert_eq!(url.kind(), WorkspaceItemKind::Url);
    let group = WorkspaceItemSpec {
        label: "Ops".into(),
        children: vec![dt("Task")],
        ..Default::default()
    };
    assert_eq!(group.kind(), WorkspaceItemKind::Group);
    let route = WorkspaceItemSpec {
        label: "Dash".into(),
        route: Some("/dashboard".into()),
        ..Default::default()
    };
    assert_eq!(route.kind(), WorkspaceItemKind::Route);
}

#[test]
fn collect_doctypes_walks_groups() {
    let _g = lock();
    reset();
    let item = WorkspaceItemSpec {
        label: "Ops".into(),
        children: vec![
            dt("Task"),
            dt("Project"),
            WorkspaceItemSpec {
                label: "Deep".into(),
                children: vec![dt("Subtask")],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut out = Vec::new();
    item.collect_doctypes(&mut out);
    assert_eq!(out, vec!["Task", "Project", "Subtask"]);
}

#[test]
fn registering_replaces_own_previous_set_in_place() {
    let _g = lock();
    reset();
    register_app_workspaces("app-a", vec![], vec![spec("one", "m1", vec![])]);
    register_app_workspaces("app-b", vec![], vec![spec("two", "m2", vec![])]);
    register_app_workspaces("app-a", vec![], vec![spec("three", "m1", vec![dt("Task")])]);
    // app-a keeps its first-appearance slot, so a hot reload never bumps it
    // to the end of the desk order.
    let all = registered_workspaces();
    let names: Vec<_> = all.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, vec!["three", "two"]);
}

#[test]
fn builders_shape_the_declaration() {
    let folder = WorkspaceFolderSpec::new("ops")
        .label("Operations")
        .icon("Boxes");
    assert_eq!(folder.name, "ops");
    assert_eq!(folder.label, "Operations");
    assert_eq!(folder.icon.as_deref(), Some("Boxes"));

    let ws = WorkspaceSpec::new("printing", "Printing")
        .module("Printing")
        .icon("Printer")
        .folder("ops")
        .item(WorkspaceItemSpec::doctype("Print Format").icon("FileText"))
        .group("More", vec![WorkspaceItemSpec::route("/desk/more")]);
    assert_eq!(ws.name, "printing");
    assert_eq!(ws.label, "Printing");
    assert_eq!(ws.module, "Printing");
    assert_eq!(ws.icon.as_deref(), Some("Printer"));
    assert_eq!(ws.folder.as_deref(), Some("ops"));
    assert_eq!(ws.items.len(), 2);
    assert_eq!(ws.items[0].label, "Print Format");
    assert_eq!(ws.items[0].doctype.as_deref(), Some("Print Format"));
    assert_eq!(ws.items[1].kind(), WorkspaceItemKind::Group);
    assert_eq!(ws.items[1].children[0].kind(), WorkspaceItemKind::Route);

    let titled = WorkspaceItemSpec::doctype_titled("ToDo", "My Tasks");
    assert_eq!(titled.label, "My Tasks");
    assert_eq!(titled.doctype.as_deref(), Some("ToDo"));
    let url = WorkspaceItemSpec::url("https://x").label("Docs").new_tab();
    assert_eq!(url.kind(), WorkspaceItemKind::Url);
    assert!(url.open_in_new_tab);
}

#[test]
fn workspace_set_registers_names() {
    let _g = lock();
    reset();
    let outcome = WorkspaceSet::default()
        .workspace(
            WorkspaceSpec::new("printing", "Printing")
                .module("Printing")
                .item(WorkspaceItemSpec::doctype("Print Format")),
        )
        .register("ferris-print");
    assert_eq!(outcome.names, vec!["printing"]);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(registered_workspaces()[0].name, "printing");
}

#[test]
fn workspace_set_validate_does_not_touch_the_registry() {
    let _g = lock();
    reset();
    let outcome = WorkspaceSet::default()
        .workspace(WorkspaceSpec::new("printing", "Printing"))
        .validate("ferris-print");
    assert_eq!(outcome.names, vec!["printing"]);
    assert!(registered_workspaces().is_empty());
    assert!(!has_workspaces());
}

#[test]
fn empty_and_duplicate_names_are_reported() {
    let _g = lock();
    reset();
    let outcome = WorkspaceSet::default()
        .workspace(WorkspaceSpec::new("   ", "Blank"))
        .workspace(WorkspaceSpec::new("dup", "First"))
        .workspace(WorkspaceSpec::new("dup", "Second"))
        .validate("owner");
    assert_eq!(outcome.names, vec!["dup"]);
    assert!(
        outcome.warnings.iter().any(|w| w.contains("empty name")),
        "{:?}",
        outcome.warnings
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("duplicate name")),
        "{:?}",
        outcome.warnings
    );
}

#[test]
fn dead_items_are_reported() {
    let _g = lock();
    reset();
    let outcome = WorkspaceSet::default()
        .workspace(WorkspaceSpec::new("ops", "Ops").item(WorkspaceItemSpec::default()))
        .validate("owner");
    assert_eq!(outcome.names, vec!["ops"]);
    assert!(
        outcome.warnings.iter().any(|w| w.contains("dead link")),
        "{:?}",
        outcome.warnings
    );
}

#[test]
fn re_registering_empty_drops_the_previous_layout() {
    let _g = lock();
    reset();
    register_app_workspaces("app-a", vec![], vec![spec("one", "m1", vec![])]);
    register_app_workspaces("app-a", vec![], vec![]);
    assert!(registered_workspaces().is_empty());
}

#[test]
fn empty_label_and_module_are_humanized() {
    let _g = lock();
    reset();
    register_app_workspaces("sales", vec![], vec![spec("sales_order", "sales", vec![])]);
    let ws = registered_workspaces().remove(0);
    assert_eq!(ws.label, "Sales");
    assert_eq!(ws.module, "sales");
    register_app_workspaces("sales", vec![], vec![spec("without_module", "", vec![])]);
    let ws = registered_workspaces().remove(0);
    assert_eq!(ws.module, "sales");
    assert_eq!(ws.label, "Without Module");
}

#[test]
fn folders_are_replaced_with_the_layout_that_declares_them() {
    let _g = lock();
    reset();
    let mut grouped = spec("sales", "m1", vec![dt("Task")]);
    grouped.folder = Some("operations".into());
    register_app_workspaces(
        "app-a",
        vec![folder_spec("operations")],
        vec![grouped.clone(), spec("stock", "m1", vec![])],
    );

    let folders = registered_folders();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].label, "Operations");
    assert_eq!(folders[0].icon.as_deref(), Some(DEFAULT_FOLDER_ICON));
    assert_eq!(
        registered_workspaces()[0].folder.as_deref(),
        Some("operations")
    );
    assert!(registered_workspaces()[1].folder.is_none());

    // Re-registering the owner with no folder removes the folder with it: a
    // hot reload must not leave an empty folder behind on the desk grid.
    register_app_workspaces("app-a", vec![], vec![grouped]);
    assert!(registered_folders().is_empty());
}

#[test]
fn a_layout_of_nothing_does_not_register() {
    let _g = lock();
    reset();
    register_app_workspaces("app-a", vec![], vec![]);
    assert!(!has_workspaces());
    assert!(registered_workspaces().is_empty());
}

#[test]
fn a_folder_keeps_the_icon_the_app_declared() {
    let _g = lock();
    reset();
    register_app_workspaces(
        "app-a",
        vec![WorkspaceFolderSpec {
            name: "ops".into(),
            label: "Operations".into(),
            icon: Some("Boxes".into()),
        }],
        vec![],
    );
    assert_eq!(registered_folders()[0].icon.as_deref(), Some("Boxes"));
}

#[test]
fn describe_json_roundtrips() {
    let _g = lock();
    reset();
    let ws = spec(
        "operations",
        "test",
        vec![
            dt("Task"),
            WorkspaceItemSpec {
                label: "Docs".into(),
                href: Some("https://wiki.example.test".into()),
                open_in_new_tab: true,
                icon: Some("BookOpen".into()),
                ..Default::default()
            },
        ],
    );
    let json = serde_json::to_string(&ws).unwrap();
    let back: WorkspaceSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(ws, back);
}
