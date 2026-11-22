//! Desk workspace/sidebar and panel feeds.
//!
//! These are the payloads behind `/api/modules`, `/api/search`,
//! `/api/notifications`, `/api/events` and `/api/notification-settings`.
//! `ferris-api` emits them from raw documents (the `to_*` functions there);
//! `ferris-web` deserializes them in `desk/session`. Defining them once means
//! a renamed field is a compile error on the server rather than a `null` in
//! the browser.

use serde::{Deserialize, Serialize};

/// One sidebar link: a doctype shortcut, an internal desk route, an external
/// URL, or a group holding nested links.
///
/// `kind` is the discriminant the Desk switches on; it stays a string because
/// the API emits `{"kind": "doctype" | "route" | "url" | "group"}` and an
/// unknown kind must degrade to a plain link rather than fail the whole
/// `/api/modules` response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeskSidebarEntry {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub doctype: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub open_in_new_tab: bool,
    #[serde(default)]
    pub title_field: Option<String>,
    #[serde(default)]
    pub image_field: Option<String>,
    /// Nested links of a group.
    ///
    /// Serialized as `"items"` — the key `/api/modules` has always used for
    /// nested links, since a group and a module both list their contents under
    /// `items` — while `"children"` is accepted on the way in so in-process
    /// fixtures can use the clearer name.
    #[serde(default, rename = "items")]
    pub children: Vec<DeskSidebarEntry>,
}

impl DeskSidebarEntry {
    /// A module-scoped doctype link, the shape a module auto-derives.
    ///
    /// The route keeps the doctype name verbatim (`/desk/selling/Sales Order`)
    /// rather than slugifying it: the desk router declares
    /// `/desk/:module/:doctype`, so the segment has to be the real name.
    pub fn doctype_link(doctype: &str, module: &str) -> Self {
        Self {
            kind: "doctype".into(),
            name: Some(doctype.to_string()),
            label: crate::contract::pretty_identifier(doctype),
            doctype: Some(doctype.to_string()),
            route: Some(format!("/desk/{module}/{doctype}")),
            ..Default::default()
        }
    }
}

/// A workspace (sidebar section) with its links, in display order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeskModule {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub module: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub items: Vec<DeskSidebarEntry>,
}

/// One icon on the desk grid, in the order the user arranged it.
///
/// A workspace icon names one of `DeskLayout::workspaces`; a folder icon names
/// its members in `items`. Folders are one level deep by construction — a
/// folder's members are workspaces, never folders — so the grid opens a folder
/// in a dialog and stops there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeskIcon {
    /// Stable identity: the workspace name, or a folder id. The grid keys its
    /// drag-and-drop and its editing on this, so it has to survive a reload
    /// unchanged.
    pub id: String,
    /// `workspace` or `folder` — the discriminant the grid switches on.
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Accent colour of a folder icon. `None` is the desk default.
    #[serde(default)]
    pub color: Option<String>,
    /// The folder's workspaces, in the order the user arranged them. Always
    /// empty on a workspace icon.
    #[serde(default)]
    pub items: Vec<DeskIcon>,
    /// A workspace the user keeps off the grid without removing it from the
    /// sidebar. Only meaningful on a workspace icon.
    #[serde(default)]
    pub hidden: bool,
}

impl DeskIcon {
    /// Whether this icon is a folder, so the grid reads one accessor instead of
    /// matching on the `kind` spelling at every call site.
    pub fn is_folder(&self) -> bool {
        self.kind == "folder"
    }
}

/// The whole desk behind `GET /api/modules`: the grid the user arranged, and
/// the workspaces both the grid and the sidebar are drawn from.
///
/// `icons` is the user's own arrangement — seeded from what the apps declared
/// and reshaped by whatever they saved — while `workspaces` is every workspace
/// that exists, hidden ones included, so a hidden icon is still one click away
/// from the editor that brings it back.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeskLayout {
    pub icons: Vec<DeskIcon>,
    #[serde(default)]
    pub workspaces: Vec<DeskModule>,
}

/// One entry of the user's saved arrangement: what `PUT /api/desk-layout` takes
/// and what the layout document stores.
///
/// One flat struct rather than a tagged union because the browser writes it by
/// hand, and which kind a placement is follows from the fields it sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeskIconPlacement {
    /// Name of the workspace placed. Set on a workspace placement.
    #[serde(default)]
    pub workspace: String,
    /// Id of the folder placed. Set on a folder placement.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    /// Workspace names the folder holds, in order.
    #[serde(default)]
    pub items: Vec<String>,
    /// Keep this workspace off the grid without removing it from the sidebar.
    #[serde(default)]
    pub hidden: bool,
}

impl DeskIconPlacement {
    /// Whether this placement is a folder.
    pub fn is_folder(&self) -> bool {
        !self.folder.trim().is_empty()
    }
}

/// One global-search hit (`GET /api/search`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeskSearchHit {
    pub doctype: String,
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub module: String,
    #[serde(default)]
    pub route: String,
}

/// One notification row.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub route: String,
    #[serde(default)]
    pub created_at: String,
    /// Serialized as `"read"`, not `"is_read"`: that is the wire name the
    /// panel feed has always used.
    #[serde(default, rename = "read")]
    pub is_read: bool,
}

/// Latest notifications plus the unread badge count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationFeed {
    #[serde(default)]
    pub items: Vec<Notification>,
    #[serde(default)]
    pub unread: u64,
}

/// The signed-in user's notification preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    #[serde(default = "default_true")]
    pub desktop: bool,
    #[serde(default = "default_true")]
    pub notify_on_login: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            desktop: true,
            notify_on_login: true,
        }
    }
}

/// One entry on the upcoming events feed (`GET /api/events`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeskEvent {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub starts_on: String,
    #[serde(default)]
    pub ends_on: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub description: String,
}

/// Upcoming events for the signed-in user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFeed {
    #[serde(default)]
    pub items: Vec<DeskEvent>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module with a group whose children arrive under `"items"` must
    /// deserialize, and the nested links must keep their own flags.
    #[test]
    fn group_children_deserialize_from_api_items_field() {
        let json = r#"{
            "name": "operations",
            "label": "Operations",
            "module": "test",
            "icon": "LayoutGrid",
            "items": [
                { "kind": "route", "label": "Dashboard", "route": "/dashboard", "icon": "House" },
                {
                    "kind": "group",
                    "label": "Management",
                    "icon": "Folder",
                    "items": [
                        { "kind": "route", "label": "Progress report", "route": "/report/ops" },
                        { "kind": "url", "label": "Docs", "href": "https://wiki.ferris.local", "open_in_new_tab": true }
                    ]
                }
            ]
        }"#;
        let module: DeskModule = serde_json::from_str(json).expect("module parses");
        let group = module
            .items
            .iter()
            .find(|i| i.kind == "group")
            .expect("group entry present");

        let labels: Vec<_> = group.children.iter().map(|c| c.label.clone()).collect();
        assert_eq!(labels, vec!["Progress report", "Docs"]);
        assert!(group.children[1].open_in_new_tab);
    }

    /// Nested links travel under `"items"` in both directions.
    #[test]
    fn nested_links_travel_as_items() {
        let group = DeskSidebarEntry {
            kind: "group".into(),
            label: "Management".into(),
            children: vec![DeskSidebarEntry {
                kind: "route".into(),
                label: "Progress report".into(),
                route: Some("/report/ops".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let json = serde_json::to_value(&group).expect("group serializes");
        assert!(
            json.get("children").is_none(),
            "nested links must not be emitted as children"
        );
        assert_eq!(
            json["items"][0]["label"],
            serde_json::json!("Progress report")
        );

        let body = serde_json::json!({
            "kind": "group",
            "label": "Management",
            "items": [{ "kind": "route", "label": "Progress report" }],
        });
        let parsed: DeskSidebarEntry =
            serde_json::from_value(body).unwrap_or_else(|e| panic!("items parse: {e}"));
        assert_eq!(parsed.children.len(), 1, "children read from items");
    }

    /// Notification preferences default to enabled on both sides of the wire:
    /// a user who has never saved settings must not be silently opted out.
    #[test]
    fn notification_settings_default_to_enabled() {
        assert!(NotificationSettings::default().desktop);
        assert!(NotificationSettings::default().notify_on_login);

        let partial: NotificationSettings =
            serde_json::from_value(serde_json::json!({ "desktop": false }))
                .expect("partial settings parse");
        assert!(!partial.desktop);
        assert!(partial.notify_on_login);
    }

    /// The read flag crosses the wire as `"read"`; if that rename is lost the
    /// feed silently reads every row as unread.
    #[test]
    fn the_read_flag_is_serialized_as_read() {
        let feed = NotificationFeed {
            items: vec![Notification {
                id: "NT-1".into(),
                is_read: true,
                ..Default::default()
            }],
            unread: 0,
        };
        let body = serde_json::to_value(&feed).expect("feed serializes");
        assert_eq!(body["items"][0]["read"], serde_json::json!(true));
        assert!(body["items"][0].get("is_read").is_none());

        let back: NotificationFeed = serde_json::from_value(body).expect("feed parses");
        assert!(back.items[0].is_read);
        assert_eq!(back.unread, 0);
    }

    /// The grid travels as an ordered icon tree and a flat workspace list, and
    /// the user's saved arrangement travels as placements. Both have to survive
    /// the round trip through the document field as JSON: the desk grid is
    /// rebuilt from the stored string on every load, so a field that silently
    /// dropped `color` or `hidden` would lose the user's edits on reload.
    #[test]
    fn the_arranged_grid_and_the_saved_placements_round_trip() {
        let layout: DeskLayout = serde_json::from_str(
            r#"{
                "icons": [
                    { "id": "sales", "kind": "workspace", "label": "Sales", "icon": "Receipt" },
                    { "id": "f1", "kind": "folder", "label": "Operations",
                      "icon": "Boxes", "color": "blue",
                      "items": [ { "id": "stock", "kind": "workspace", "label": "Stock" } ] },
                    { "id": "billing", "kind": "workspace", "label": "Billing", "hidden": true }
                ],
                "workspaces": [ { "name": "sales", "label": "Sales", "items": [] } ]
            }"#,
        )
        .expect("layout parses");

        assert_eq!(layout.icons.len(), 3);
        assert!(!layout.icons[0].is_folder());
        assert!(layout.icons[1].is_folder());
        assert_eq!(layout.icons[1].color.as_deref(), Some("blue"));
        assert_eq!(layout.icons[1].items[0].id, "stock");
        assert!(layout.icons[2].hidden, "a hidden icon stays in the tree");

        // Through `serde_json::Value`, which is how it lands in the document's
        // Code field.
        let stored = serde_json::to_value(&layout).expect("layout serializes");
        let back: DeskLayout = serde_json::from_value(stored).expect("layout parses back");
        assert_eq!(back.icons[1].color.as_deref(), Some("blue"));
        assert!(back.icons[2].hidden);
        assert_eq!(back.icons[1].items[0].id, "stock");

        let placements: Vec<DeskIconPlacement> = serde_json::from_str(
            r#"[
                { "workspace": "sales" },
                { "folder": "f1", "label": "Operations", "icon": "Boxes",
                  "color": "blue", "items": ["stock"] },
                { "workspace": "billing", "hidden": true }
            ]"#,
        )
        .expect("placements parse");
        assert!(!placements[0].is_folder());
        assert!(placements[1].is_folder());
        assert_eq!(placements[1].items, vec!["stock".to_string()]);
        assert!(placements[2].hidden);

        // A placement the browser wrote with only the fields it cares about still
        // parses: every field past the discriminant is optional.
        let sparse: DeskIconPlacement =
            serde_json::from_str(r#"{"folder":"f2"}"#).expect("sparse placement parses");
        assert!(sparse.is_folder());
        assert_eq!(sparse.label, "");
        assert!(sparse.items.is_empty());

        // An empty layout is a valid answer, not an error: that is what a desk
        // with no workspaces yet returns.
        let empty: DeskLayout = serde_json::from_str(r#"{"icons":[]}"#).expect("empty parses");
        assert!(empty.icons.is_empty());
        assert!(empty.workspaces.is_empty());
    }

    /// An auto-derived doctype link is a first-class sidebar entry, not a
    /// special case: the Desk renders it from the same struct as a declared
    /// one, so it must round-trip and carry the list route.
    #[test]
    fn an_auto_derived_doctype_link_round_trips() {
        let entry = DeskSidebarEntry::doctype_link("Sales Order", "selling");
        assert_eq!(entry.kind, "doctype");
        assert_eq!(entry.route.as_deref(), Some("/desk/selling/Sales Order"));

        let json = serde_json::to_value(&entry).expect("entry serializes");
        let back: DeskSidebarEntry = serde_json::from_value(json).expect("entry parses");
        assert_eq!(back.doctype.as_deref(), Some("Sales Order"));
        assert_eq!(back.label, "Sales order");
    }

    /// The label is humanized by lowercasing every word after the first, which
    /// is what `ferris-api` has always sent. Pinned here because the desk
    /// shows this string in the sidebar: a change here is a visible rename on
    /// every existing app.
    #[test]
    fn a_doctype_label_is_humanized() {
        assert_eq!(
            DeskSidebarEntry::doctype_link("Sales Order", "selling").label,
            "Sales order"
        );
        assert_eq!(
            DeskSidebarEntry::doctype_link("Item", "stock").label,
            "Item"
        );
    }
}
