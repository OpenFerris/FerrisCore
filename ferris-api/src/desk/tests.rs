//! Desk layout and search tests.

use ferris_core::Value;
use ferris_core::workspaces::WorkspaceSpec;
use ferris_schema::contract::{DeskIconPlacement, DeskLayout, DeskModule};

use super::layout::build_layout;
use super::modules::{auto_modules, workspace_warnings};
use super::search::{SEARCH_MAX_TOTAL, SEARCH_MAX_TOTAL_CAP, resolve_max_total};

/// The workspace registry is process-global and keyed by owner, so these
/// tests must not overlap: one registering app's layout would otherwise be
/// visible inside another's `/api/modules` build.
static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn declared_workspace_controls_sidebar() {
    let _guard = serialized();
    // The link is only drawn while its DocType is registered; register the
    // fixture one so this test exercises the declared shape, not the
    // dead-link filter (which `workspace_hides_links_to_missing_doctypes`
    // covers).
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new("Task", "test"));
    ferris_core::workspaces::register_app_workspaces(
        "test",
        vec![],
        vec![ferris_core::workspaces::WorkspaceSpec {
            name: "operations".into(),
            label: "Operations".into(),
            module: "test".into(),
            icon: Some("Settings".into()),
            folder: None,
            items: vec![
                ferris_core::workspaces::WorkspaceItemSpec {
                    label: "Tareas".into(),
                    doctype: Some("Task".into()),
                    icon: Some("SquareCheck".into()),
                    ..Default::default()
                },
                ferris_core::workspaces::WorkspaceItemSpec {
                    label: "Docs".into(),
                    href: Some("https://wiki.test".into()),
                    open_in_new_tab: true,
                    icon: Some("BookOpen".into()),
                    ..Default::default()
                },
                ferris_core::workspaces::WorkspaceItemSpec {
                    label: "Grupo".into(),
                    icon: Some("Folder".into()),
                    children: vec![ferris_core::workspaces::WorkspaceItemSpec {
                        label: "Enlace interno".into(),
                        route: Some("/dashboard".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        }],
    );
    let layout = build_layout(None);
    let ws = &workspace(&layout, "operations").items;
    assert_eq!(ws[1].kind, "url");
    assert_eq!(ws[1].href.as_deref(), Some("https://wiki.test"));
    assert_eq!(ws[1].open_in_new_tab, true);
    assert_eq!(ws[2].kind, "group");
    assert_eq!(ws[2].children[0].kind, "route");
}

/// A declared link to a DocType this install does not have is hidden, a
/// group whose links all vanished is dropped with it, and the start-up
/// diagnostic names the missing DocType.
#[test]
fn workspace_hides_links_to_missing_doctypes() {
    let _guard = serialized();
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new("Filter Present", "test"));
    ferris_core::workspaces::register_app_workspaces(
        "filter-test",
        vec![],
        vec![WorkspaceSpec {
            name: "filter-ws".into(),
            label: "Filter".into(),
            module: "test".into(),
            items: vec![
                ferris_core::workspaces::WorkspaceItemSpec::doctype("Filter Present"),
                ferris_core::workspaces::WorkspaceItemSpec::doctype("Filter Missing"),
                ferris_core::workspaces::WorkspaceItemSpec::group(
                    "Empty",
                    vec![ferris_core::workspaces::WorkspaceItemSpec::doctype(
                        "Filter Missing",
                    )],
                ),
                ferris_core::workspaces::WorkspaceItemSpec::group(
                    "Kept",
                    vec![
                        ferris_core::workspaces::WorkspaceItemSpec::doctype("Filter Missing"),
                        ferris_core::workspaces::WorkspaceItemSpec::doctype("Filter Present"),
                    ],
                ),
            ],
            ..Default::default()
        }],
    );

    let layout = build_layout(None);
    let items = &workspace(&layout, "filter-ws").items;
    let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels, vec!["Filter Present", "Kept"]);
    let kept = &items[1];
    assert_eq!(kept.kind, "group");
    assert_eq!(kept.children.len(), 1);
    assert_eq!(kept.children[0].label, "Filter Present");

    let warnings = workspace_warnings();
    assert!(
        warnings.iter().any(|w| w.contains("Filter Missing")),
        "{warnings:?}"
    );
}

#[test]
fn workspaces_that_declare_nothing_stay_legacy() {
    let _guard = serialized();
    // Relative, not absolute. The schema registry is process-wide and every
    // other test in this binary registers DocTypes into it, so asserting
    // `len() == 0` made the test depend on which tests happened to run first
    // — it passed with 42 tests in the binary and failed as soon as another
    // module added a fixture. The claim under test is that registering an app
    // that declares nothing adds nothing.
    let before = auto_modules().len();
    ferris_core::workspaces::register_app_workspaces("a", vec![], vec![]);
    assert_eq!(auto_modules().len(), before);
}

/// The seed grid puts a declared workspace's icon inside its folder, with
/// the workspaces that declared none sitting before the folders.
#[test]
fn a_declared_folder_groups_its_workspaces() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "grouping-test",
        vec![ferris_core::workspaces::WorkspaceFolderSpec {
            name: "seed-grouping".into(),
            ..Default::default()
        }],
        vec![
            spec("grouping-test", "grouping-sales", Some("seed-grouping")),
            spec("grouping-test", "grouping-stock", None),
        ],
    );

    let layout = build_layout(None);
    let folder = layout
        .icons
        .iter()
        .find(|i| i.id == "seed-grouping")
        .expect("the declared folder reaches the grid");
    assert!(folder.is_folder());
    assert_eq!(
        folder.label, "Seed Grouping",
        "an undeclared label is humanized"
    );
    assert_eq!(folder.icon.as_deref(), Some("Folder"));
    assert_eq!(folder.items.len(), 1);
    assert_eq!(folder.items[0].id, "grouping-sales");

    let loose = layout
        .icons
        .iter()
        .position(|i| i.id == "grouping-stock")
        .expect("the ungrouped workspace is on the grid");
    let grouped = layout
        .icons
        .iter()
        .position(|i| i.id == "seed-grouping")
        .expect("the folder is on the grid");
    assert!(loose < grouped, "loose workspaces come before the folders");
}

/// A workspace pointing at a folder no app declared lands on the grid
/// instead of in a dialog that does not exist, and a folder nothing landed
/// in is dropped rather than opening an empty dialog. Both are what a hot
/// reload leaves behind when it removes a workspace a folder was declared
/// for.
#[test]
fn declared_folders_are_normalized_against_the_workspaces_that_land_in_them() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "orphan-test",
        vec![
            ferris_core::workspaces::WorkspaceFolderSpec {
                name: "orphan-kept".into(),
                ..Default::default()
            },
            ferris_core::workspaces::WorkspaceFolderSpec {
                name: "orphan-nobody-lands-here".into(),
                ..Default::default()
            },
        ],
        vec![
            spec("orphan-test", "orphan-kept-ws", Some("orphan-kept")),
            spec(
                "orphan-test",
                "orphan-misfiled",
                Some("orphan-never-declared"),
            ),
        ],
    );

    let layout = build_layout(None);
    assert!(
        layout
            .icons
            .iter()
            .all(|i| i.id != "orphan-nobody-lands-here"),
        "a folder with no workspaces must not become an empty dialog"
    );
    let kept = layout
        .icons
        .iter()
        .find(|i| i.id == "orphan-kept")
        .expect("the folder a workspace landed in survives");
    assert_eq!(kept.items.len(), 1);
    assert!(
        layout
            .icons
            .iter()
            .find(|i| i.id == "orphan-misfiled")
            .is_some(),
        "a workspace naming a folder nobody declared lands on the grid itself"
    );
}

/// An app is free to name a folder after one of its own workspaces, and the
/// grid cannot carry two icons under one id: every lookup on it is by id, so
/// the folder would answer for the workspace and the workspace would be
/// unreachable behind it. A folder whose name is free keeps it.
#[test]
fn a_folder_named_after_a_workspace_gets_an_id_of_its_own() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "collide-test",
        vec![
            ferris_core::workspaces::WorkspaceFolderSpec {
                name: "collide-sales".into(),
                label: "Sales".into(),
                ..Default::default()
            },
            ferris_core::workspaces::WorkspaceFolderSpec {
                name: "collide-free".into(),
                ..Default::default()
            },
        ],
        vec![
            spec("collide-test", "collide-sales", Some("collide-sales")),
            spec("collide-test", "collide-lead", Some("collide-sales")),
            spec("collide-test", "collide-nobody", Some("collide-free")),
        ],
    );

    let layout = build_layout(None);
    let mut ids: Vec<&str> = Vec::new();
    for icon in &layout.icons {
        ids.push(icon.id.as_str());
        ids.extend(icon.items.iter().map(|item| item.id.as_str()));
    }
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "no id is used twice: {ids:?}");

    let folder = layout
        .icons
        .iter()
        .find(|i| i.is_folder() && i.label == "Sales")
        .expect("the folder is on the grid");
    assert_ne!(
        folder.id, "collide-sales",
        "a folder named after a workspace cannot answer to that name"
    );
    assert_eq!(
        folder.items.len(),
        2,
        "and it still holds what landed in it"
    );

    assert!(
        layout.icons.iter().any(|i| i.id == "collide-free"),
        "a name no workspace answers to is kept as declared"
    );
}

/// A user rearranges the seed: their order wins, a workspace moves into a
/// folder of their own, one is hidden, one that no longer exists is dropped,
/// and one they never mentioned — an app added it after they saved — is
/// appended so it is not invisible.
#[test]
fn a_users_arrangement_is_laid_over_the_seed() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "arrange-test",
        vec![ferris_core::workspaces::WorkspaceFolderSpec {
            name: "arrange-seed".into(),
            ..Default::default()
        }],
        vec![
            spec("arrange-test", "arrange-sales", Some("arrange-seed")),
            spec("arrange-test", "arrange-billing", None),
            spec("arrange-test", "arrange-stock", None),
        ],
    );

    let saved = vec![
        placement("arrange-stock", false),
        folder("mine", "My things", &["arrange-billing", "gone-forever"]),
        placement("arrange-sales", true),
    ];
    let layout = build_layout(Some(&saved));

    let ids: Vec<&str> = layout
        .icons
        .iter()
        .filter(|i| i.id.starts_with("arrange-") || i.id == "mine")
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(ids, vec!["arrange-stock", "mine", "arrange-sales"]);

    let mine = layout
        .icons
        .iter()
        .find(|i| i.id == "mine")
        .expect("a folder the user made reaches the grid");
    assert_eq!(
        mine.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec!["arrange-billing"],
        "a member that no longer exists is dropped, not rendered broken"
    );
    assert_eq!(mine.label, "My things");
    assert!(
        layout
            .icons
            .iter()
            .find(|i| i.id == "arrange-sales")
            .expect("sales is on the grid")
            .hidden,
        "hiding a workspace keeps its icon in the layout"
    );
    assert_eq!(
        workspace(&layout, "arrange-sales").name,
        "arrange-sales",
        "a hidden workspace stays in the workspace list, so the editor can bring it back"
    );
}

/// A workspace the arrangement never mentions still reaches the grid, so an
/// app shipping a new workspace does not need the user to reset their desk.
#[test]
fn a_workspace_the_arrangement_forgot_is_appended() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "forgot-test",
        vec![],
        vec![
            spec("forgot-test", "forgot-old", None),
            spec("forgot-test", "forgot-new", None),
        ],
    );

    // The user saved a layout while only `forgot-old` existed.
    let saved = vec![placement("forgot-old", false)];
    let layout = build_layout(Some(&saved));

    let ids: Vec<&str> = layout
        .icons
        .iter()
        .filter(|i| i.id.starts_with("forgot-"))
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(ids, vec!["forgot-old", "forgot-new"]);
}

/// The user made an empty folder on purpose, which is a folder waiting for a
/// drop — unlike a declared folder nothing landed in, which is an artifact.
#[test]
fn a_users_empty_folder_survives() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "scratch-test",
        vec![],
        vec![spec("scratch-test", "scratch-ws", None)],
    );

    let layout = build_layout(Some(&[folder("scratch", "Scratch", &[])]));
    let scratch = layout
        .icons
        .iter()
        .find(|i| i.id == "scratch")
        .expect("the folder the user made reaches the grid");
    assert!(scratch.is_folder());
    assert!(scratch.items.is_empty());
}

/// A layout saved against an install that has none of the workspaces it names —
/// every app uninstalled, or the layouts from two different installs mixed — is
/// not an error: the grid is simply empty. A folder survives it (the user's own,
/// waiting for a drop); a workspace cannot, having nowhere to navigate to.
#[test]
fn an_arrangement_of_nothing_builds_an_empty_grid() {
    let _guard = serialized();
    ferris_core::workspaces::register_app_workspaces(
        "stale-test",
        vec![],
        vec![spec("stale-test", "stale-present", None)],
    );

    let saved = vec![
        placement("vanished-workspace", false),
        folder("stale-folder", "Gone", &["also-vanished"]),
    ];
    let layout = build_layout(Some(&saved));

    assert!(
        layout.icons.iter().all(|i| i.id != "vanished-workspace"),
        "a workspace icon with nothing behind it must not be drawn"
    );
    let kept = layout
        .icons
        .iter()
        .find(|i| i.id == "stale-folder")
        .expect("the user's own folder survives, to drop workspaces into");
    assert!(kept.items.is_empty());
    assert!(
        layout.icons.iter().any(|i| i.id == "stale-present"),
        "what does exist still reaches the grid"
    );
}

fn placement(workspace: &str, hidden: bool) -> DeskIconPlacement {
    DeskIconPlacement {
        workspace: workspace.into(),
        hidden,
        ..Default::default()
    }
}

fn folder(id: &str, label: &str, items: &[&str]) -> DeskIconPlacement {
    DeskIconPlacement {
        folder: id.into(),
        label: label.into(),
        items: items.iter().map(|name| (*name).to_string()).collect(),
        ..Default::default()
    }
}

fn spec(module: &str, name: &str, folder: Option<&str>) -> WorkspaceSpec {
    WorkspaceSpec {
        name: name.into(),
        module: module.into(),
        folder: folder.map(str::to_string),
        ..Default::default()
    }
}

/// The layout carries every registered owner's workspaces, so a test looks
/// its own up by name instead of indexing the first entry.
fn workspace<'a>(layout: &'a DeskLayout, name: &str) -> &'a DeskModule {
    layout
        .workspaces
        .iter()
        .find(|w| w.name == name)
        .unwrap_or_else(|| panic!("workspace {name} reaches the layout"))
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn max_total_defaults_and_clamps_to_the_hard_ceiling() {
        let settings = |raw: serde_json::Value| Value::from_json(raw);
        assert_eq!(resolve_max_total(None), SEARCH_MAX_TOTAL);

        let five = settings(serde_json::json!({ "max_results": 5 }));
        assert_eq!(resolve_max_total(Some(&five)), 5);

        // Zero and negative fall back to the default rather than disabling the
        // cap (the old code filtered them out the same way).
        let zero = settings(serde_json::json!({ "max_results": 0 }));
        assert_eq!(resolve_max_total(Some(&zero)), SEARCH_MAX_TOTAL);
        let negative = settings(serde_json::json!({ "max_results": -1 }));
        assert_eq!(resolve_max_total(Some(&negative)), SEARCH_MAX_TOTAL);

        let huge = settings(serde_json::json!({ "max_results": 10_000 }));
        assert_eq!(resolve_max_total(Some(&huge)), SEARCH_MAX_TOTAL_CAP);
    }
}
