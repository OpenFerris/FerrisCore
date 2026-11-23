//! Building the desk grid out of declared workspaces and a user's arrangement.

use axum::{Json, extract::State};
use std::collections::{HashMap, HashSet};

use ferris_core::i18n::{I18n, Locale, MetaTranslate};
use ferris_core::workspaces::{DEFAULT_FOLDER_ICON, WorkspaceFolderSpec, WorkspaceSpec};
use ferris_schema::contract::{
    DeskIcon, DeskIconPlacement, DeskLayout, DeskModule, pretty_identifier,
};

use super::modules::{auto_modules, doctype_modules, workspace_module};
use crate::ApiState;
use crate::router::RequestLocale;

/// `GET /api/modules` — the desk as the apps declared it: the seeded grid and the
/// workspaces with their sidebar links.
///
/// Apps that declare `"workspaces"` in `describe()` control their sidebar: the
/// response mirrors those declarations (doctype links, internal routes,
/// external URLs and nested groups, with icons) for the app's module, plus any
/// `"folders"` they declared, which seed the desk grid. Apps that declare
/// nothing keep the legacy behaviour: one auto-derived module per schema module
/// listing its DocTypes. Doctypes that live in a module with a declared
/// workspace are intentionally absent unless the app lists them.
///
/// The workspaces are read back from `tabWorkspace` ([`crate::workspace_store`]),
/// which the host seeds from the same declarations at boot; the in-code registry
/// is the fallback when the table is empty or unreadable, so the desk still draws
/// before the first seed (and in the DB-less tests).
///
/// This is the unarranged seed. A signed-in user's own arrangement of that grid
/// lives behind `GET /api/desk-layout` (`crate::panel`), which merges their
/// saved placements over the same seed; only `ferris-web` reads either, and it
/// reads the personalized one.
pub async fn modules(
    State(state): State<std::sync::Arc<ApiState>>,
    RequestLocale(locale): RequestLocale,
) -> Json<DeskLayout> {
    Json(load_layout_localized(&state.pool, None, &state.i18n, &locale).await)
}

/// The desk for a signed-in request: the stored `Workspace` documents laid out,
/// falling back to the in-code registry when none are stored (or the `DocType` is
/// not registered, as in the DB-less tests).
pub async fn load_layout(pool: &ferris_db::Db, saved: Option<&[DeskIconPlacement]>) -> DeskLayout {
    let declared = match crate::workspace_store::load_workspaces(pool).await {
        Ok(specs) if !specs.is_empty() => specs,
        Ok(_) => ferris_core::workspaces::registered_workspaces(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not read workspaces from the database; falling back to the registry"
            );
            ferris_core::workspaces::registered_workspaces()
        }
    };
    let folders = ferris_core::workspaces::registered_folders();
    layout_from(&declared, &folders, saved)
}

/// [`load_layout`], with workspace and link labels resolved for `locale`.
pub async fn load_layout_localized(
    pool: &ferris_db::Db,
    saved: Option<&[DeskIconPlacement]>,
    i18n: &I18n,
    locale: &Locale,
) -> DeskLayout {
    let mut layout = load_layout(pool, saved).await;
    localize_layout(&mut layout, i18n, locale);
    layout
}

/// Translate every label of a built layout in place.
///
/// DocType links derive their key from the doctype (`doctype.<Name>.label`)
/// because the sidebar normally prints the raw name; links and groups the app
/// named itself are translated only when that name is a catalogue key, so an
/// app can ship `app.<app>.<section>` without the host knowing it.
pub fn localize_layout(layout: &mut DeskLayout, i18n: &I18n, locale: &Locale) {
    let t = MetaTranslate::new(i18n, locale);
    for icon in &mut layout.icons {
        localize_icon(icon, &t);
    }
    for workspace in &mut layout.workspaces {
        localize_module(workspace, &t);
    }
}

fn localize_module(module: &mut DeskModule, t: &MetaTranslate<'_>) {
    module.label = t.workspace_label(&module.name, &module.label).into_owned();
    for item in &mut module.items {
        localize_entry(item, t, &module.name);
    }
}

fn localize_entry(
    entry: &mut ferris_schema::contract::DeskSidebarEntry,
    t: &MetaTranslate<'_>,
    workspace: &str,
) {
    entry.label = if entry.kind == "doctype" {
        let doctype = entry.doctype.clone().unwrap_or_default();
        if entry.label.trim().is_empty() || entry.label == doctype {
            let fallback = if entry.label.trim().is_empty() {
                pretty_identifier(&doctype)
            } else {
                entry.label.clone()
            };
            t.doctype_label(&doctype, &fallback).into_owned()
        } else {
            t.literal(&entry.label).into_owned()
        }
    } else {
        // A link or group is named by the app: the workspace-scoped key first,
        // then the label itself when it is a key, then the text as written.
        let scoped = t.workspace_item_label(workspace, &entry.label).into_owned();
        if scoped != entry.label {
            scoped
        } else {
            t.literal(&entry.label).into_owned()
        }
    };
    for child in &mut entry.children {
        localize_entry(child, t, workspace);
    }
}

fn localize_icon(icon: &mut DeskIcon, t: &MetaTranslate<'_>) {
    icon.label = if icon.is_folder() {
        t.folder_label(&icon.id, &icon.label).into_owned()
    } else {
        t.workspace_label(&icon.id, &icon.label).into_owned()
    };
    for item in &mut icon.items {
        localize_icon(item, t);
    }
}

/// Build the desk from the compiled registry. Kept synchronous and free of I/O
/// for the DB-less callers (the tests, the boot diagnostics and
/// [`load_layout`]'s own fallback).
pub fn build_layout(saved: Option<&[DeskIconPlacement]>) -> DeskLayout {
    layout_from(
        &ferris_core::workspaces::registered_workspaces(),
        &ferris_core::workspaces::registered_folders(),
        saved,
    )
}

/// Build the desk: every workspace with its links, and the grid of icons those
/// workspaces appear on.
///
/// `declared` and `folders` are passed in so the same pure builder serves both
/// the in-code registry (`build_layout`) and the document-backed path
/// (`load_layout`). `saved` is the signed-in user's own arrangement. `None` — a
/// guest, or a user who has never moved an icon — gets the seed the apps
/// declared. The declaration is the baseline either way, so an app that ships a
/// new workspace or a new folder still shows up on a grid the user arranged last
/// month.
pub fn layout_from(
    declared: &[WorkspaceSpec],
    folders: &[WorkspaceFolderSpec],
    saved: Option<&[DeskIconPlacement]>,
) -> DeskLayout {
    let (workspaces, seed): (Vec<DeskModule>, Vec<DeskIcon>) = if declared.is_empty() {
        let modules = auto_modules();
        let icons = modules.iter().map(workspace_icon).collect();
        (modules, icons)
    } else {
        let claimed: HashSet<&str> = declared.iter().map(|w| w.module.as_str()).collect();
        let mut out: Vec<DeskModule> = declared.iter().map(workspace_module).collect();
        let mut icons = folder_layout(declared, folders);
        // Fall back to auto-derived modules for modules no app claimed. They
        // belong on the grid too, ungrouped and after everything declared.
        let derived = doctype_modules(|meta| !claimed.contains(meta.module.as_str()));
        icons.extend(derived.iter().map(workspace_icon));
        out.extend(derived);
        (out, icons)
    };

    let icons = match saved {
        Some(placements) => arrange(&seed, &workspaces, placements),
        None => seed,
    };
    DeskLayout { icons, workspaces }
}

/// The seed grid: the workspaces each declared folder holds, in declaration
/// order, after the workspaces that declared no folder.
///
/// Both directions are normalised here so the grid can group in one pass: a
/// workspace naming a folder that no app declared is ungrouped (its icon lands
/// on the grid rather than in a dialog that does not exist), and a declared
/// folder that ended up empty is dropped instead of opening an empty dialog.
/// Doing this once at the API is also what makes the seed consistent after a hot
/// reload removed a workspace a folder was declared for.
fn folder_layout(declared: &[WorkspaceSpec], folders: &[WorkspaceFolderSpec]) -> Vec<DeskIcon> {
    let known: HashSet<&str> = folders.iter().map(|f| f.name.as_str()).collect();

    let mut loose = Vec::new();
    let mut members: Vec<Vec<&WorkspaceSpec>> = folders.iter().map(|_| Vec::new()).collect();
    for ws in declared {
        match ws.folder.as_deref().filter(|name| known.contains(name)) {
            Some(folder) => {
                let slot = folders
                    .iter()
                    .position(|f| f.name == folder)
                    .expect("the folder was checked against `known`");
                members[slot].push(ws);
            }
            None => loose.push(ws),
        }
    }

    let mut icons: Vec<DeskIcon> = loose
        .iter()
        .map(|ws| workspace_icon(&workspace_module(ws)))
        .collect();
    icons.extend(
        folders
            .iter()
            .zip(members)
            .filter(|(_, members)| !members.is_empty())
            .map(|(folder, members)| DeskIcon {
                id: folder_id(&folder.name, declared),
                kind: "folder".to_string(),
                label: label_of(&folder.label, &folder.name),
                icon: Some(
                    folder
                        .icon
                        .clone()
                        .unwrap_or_else(|| DEFAULT_FOLDER_ICON.to_string()),
                ),
                color: None,
                items: members
                    .iter()
                    .map(|ws| workspace_icon(&workspace_module(ws)))
                    .collect(),
                hidden: false,
            }),
    );
    icons
}

/// The grid id of a declared folder.
///
/// A folder's id is its declared name, which is what makes the grid readable in
/// the API and keeps a saved arrangement pointing at the folder the app declared.
/// It is also one namespace shared with the workspaces, and an app is free to
/// name a folder after one of its own workspaces — `test` does, with `operations`
/// and `insights` on both sides. Two icons under one id is not a cosmetic
/// problem: every lookup on the grid is by id, so a drag, a hide or a drop
/// target naming `operations` would find the folder where the workspace was
/// meant, and the workspace would be unreachable behind it.
///
/// So the declared name is kept whenever it is free, and namespaced into
/// `folder:…` only when a workspace already answers to it. A folder's label is
/// unaffected either way, so the grid reads the same.
fn folder_id(name: &str, declared: &[WorkspaceSpec]) -> String {
    if declared.iter().any(|ws| ws.name == name) {
        return format!("folder:{name}");
    }
    name.to_string()
}

/// The user's arrangement laid over the seed.
///
/// The saved placements decide the order and the grouping; the seed only
/// supplies what they leave out. Three things are true by construction here,
/// and all three are what keeps a stale layout from breaking the grid:
///
/// - a placement naming a workspace no app declares any more is dropped, since
///   there would be no icon to show and nothing to navigate to;
/// - a folder keeps whatever of its members still exist, so an app dropping a
///   workspace empties the folder rather than corrupting it;
/// - a workspace the arrangement never mentions is appended at the root in seed
///   order, so a workspace an app added afterwards still appears instead of
///   being invisible because the user had once saved a layout.
fn arrange(
    seed: &[DeskIcon],
    workspaces: &[DeskModule],
    saved: &[DeskIconPlacement],
) -> Vec<DeskIcon> {
    let by_name: HashMap<&str, &DeskModule> =
        workspaces.iter().map(|ws| (ws.name.as_str(), ws)).collect();
    let mut placed: HashSet<&str> = HashSet::new();
    let mut icons: Vec<DeskIcon> = Vec::new();

    for placement in saved {
        if placement.is_folder() {
            let items: Vec<DeskIcon> = placement
                .items
                .iter()
                .filter(|name| by_name.contains_key(name.as_str()))
                .map(|name| {
                    placed.insert(name.as_str());
                    workspace_icon(by_name[name.as_str()])
                })
                .collect();
            icons.push(DeskIcon {
                id: placement.folder.clone(),
                kind: "folder".to_string(),
                label: label_of(&placement.label, &placement.folder),
                icon: Some(if placement.icon.trim().is_empty() {
                    DEFAULT_FOLDER_ICON.to_string()
                } else {
                    placement.icon.clone()
                }),
                color: Some(placement.color.clone()).filter(|c| !c.trim().is_empty()),
                items,
                hidden: false,
            });
            continue;
        }

        if placement.workspace.trim().is_empty() {
            continue;
        }
        let Some(ws) = by_name.get(placement.workspace.as_str()) else {
            continue;
        };
        placed.insert(ws.name.as_str());
        let mut icon = workspace_icon(ws);
        icon.hidden = placement.hidden;
        icons.push(icon);
    }

    for name in seed_workspace_names(seed) {
        if placed.contains(name) {
            continue;
        }
        if let Some(ws) = by_name.get(name) {
            icons.push(workspace_icon(ws));
        }
    }
    icons
}

/// Every workspace icon id in the seed, in seed order, folders flattened.
///
/// Renamed from the old `seed_workspaces`, which collided with
/// [`crate::workspace_store::seed_workspaces`] (the DocType mirror) and made
/// either name ambiguous at the call site.
fn seed_workspace_names(seed: &[DeskIcon]) -> Vec<&str> {
    let mut out = Vec::new();
    for icon in seed {
        if icon.is_folder() {
            out.extend(seed_workspace_names(&icon.items));
        } else {
            out.push(icon.id.as_str());
        }
    }
    out
}

/// The icon for a workspace: it is named by `id`, which is also the identity the
/// grid keys dragging and editing on, so it has to be the workspace name.
fn workspace_icon(ws: &DeskModule) -> DeskIcon {
    DeskIcon {
        id: ws.name.clone(),
        kind: "workspace".to_string(),
        label: ws.label.clone(),
        icon: ws.icon.clone(),
        color: None,
        items: Vec::new(),
        hidden: false,
    }
}

/// An app or a user may leave a label empty, and an icon with no label shows an
/// empty tooltip, so humanize the id instead.
fn label_of(label: &str, id: &str) -> String {
    if label.trim().is_empty() {
        pretty_identifier(id)
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_schema::contract::DeskSidebarEntry;

    fn i18n(locale: &str) -> I18n {
        I18n::new(
            Locale::parse(locale).expect("the fixture tag parses"),
            Vec::new(),
        )
    }

    fn entry(kind: &str, label: &str, doctype: Option<&str>) -> DeskSidebarEntry {
        DeskSidebarEntry {
            kind: kind.to_string(),
            label: label.to_string(),
            doctype: doctype.map(str::to_string),
            ..DeskSidebarEntry::default()
        }
    }

    /// The sidebar and the grid are translated in one pass: workspace, DocType
    /// link and the group the app named itself (through the workspace-scoped
    /// key). Nothing that identifies a link changes.
    #[test]
    fn a_layout_is_localized_without_touching_identifiers() {
        let mut layout = DeskLayout {
            icons: vec![DeskIcon {
                id: "users".into(),
                kind: "workspace".into(),
                label: "Users".into(),
                ..DeskIcon::default()
            }],
            workspaces: vec![DeskModule {
                name: "users".into(),
                label: "Users".into(),
                module: "users".into(),
                icon: None,
                items: vec![
                    entry("doctype", "User", Some("User")),
                    entry("group", "Logs", None),
                    entry("url", "evhttps", None),
                ],
            }],
        };

        let spanish = Locale::parse("es").expect("es parses");
        localize_layout(&mut layout, &i18n("es"), &spanish);

        assert_eq!(layout.workspaces[0].label, "Usuarios");
        assert_eq!(layout.workspaces[0].name, "users");
        assert_eq!(layout.workspaces[0].items[0].label, "Usuario");
        assert_eq!(
            layout.workspaces[0].items[0].doctype.as_deref(),
            Some("User")
        );
        assert_eq!(layout.workspaces[0].items[1].label, "Registros");
        assert_eq!(layout.workspaces[0].items[2].label, "evhttps");
        assert_eq!(layout.icons[0].label, "Usuarios");
    }

    /// A language without a catalogue entry keeps the declared text instead of
    /// printing the key.
    #[test]
    fn an_untranslated_label_falls_back_to_the_declared_text() {
        let mut layout = DeskLayout {
            icons: Vec::new(),
            workspaces: vec![DeskModule {
                name: "custom".into(),
                label: "Custom".into(),
                module: "custom".into(),
                icon: None,
                items: vec![entry("route", "app.shop.orders", None)],
            }],
        };
        let english = Locale::parse("en").expect("en parses");
        localize_layout(&mut layout, &i18n("en"), &english);
        assert_eq!(layout.workspaces[0].label, "Custom");
        assert_eq!(layout.workspaces[0].items[0].label, "app.shop.orders");
    }
}
