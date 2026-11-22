//! Workspace declaration types: folders, workspaces, items and the app layout.
//!
//! Pure data plus builders, split out of the old `workspaces.rs`; no registry
//! state lives here.

use serde::{Deserialize, Serialize};

/// Icon a folder falls back to when the app declares none.
pub const DEFAULT_FOLDER_ICON: &str = "Folder";

/// Folder the host files its built-in workspaces under on the desk grid.
///
/// Declared once by the host (`ferris-server`) and referenced by every built-in
/// workspace's [`WorkspaceSpec::folder`]. App/plugin workspaces keep their own
/// folders; this one is reserved for the framework.
pub const FERRIS_FOLDER: &str = "ferris";

/// One declared desk folder: a group of workspace icons on the desk grid.
///
/// Folders are one level deep by construction. A folder holds workspaces, and a
/// workspace names at most one folder, so there is nothing for a folder to hold
/// in turn — the grid renders a folder's contents in a dialog and stops there.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceFolderSpec {
    /// Unique id within the owning app, e.g. `"operations"`. Referenced by
    /// [`WorkspaceSpec::folder`].
    pub name: String,
    /// Human label shown under the folder icon; humanized from `name` when
    /// empty.
    #[serde(default)]
    pub label: String,
    /// Icon for the folder on the desk grid: a lucide icon name
    /// (`"Briefcase"`), or an inline SVG document (`"<svg …>…</svg>"`) that the
    /// Desk sanitizes and renders in the tile. Defaults to
    /// [`DEFAULT_FOLDER_ICON`].
    #[serde(default)]
    pub icon: Option<String>,
}

/// One declared workspace (sidebar section) owned by an app.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceSpec {
    /// Unique id within the owning app, e.g. `"operations"`.
    pub name: String,
    /// Human label shown in the switcher; humanized from `module`/`name` when
    /// empty.
    #[serde(default)]
    pub label: String,
    /// The module this workspace belongs to. Defaults to the owning app id
    /// when empty.
    #[serde(default)]
    pub module: String,
    /// Icon for the workspace on the desk grid: a lucide icon name
    /// (`"Boxes"`), or an inline SVG document (`"<svg …>…</svg>"`) that the Desk
    /// sanitizes and renders in the tile. Sidebar/menu rows keep using the name
    /// form (they are one line high).
    #[serde(default)]
    pub icon: Option<String>,
    /// Name of the [`WorkspaceFolderSpec`] this workspace is grouped under on
    /// the desk grid. Empty (or naming a folder that does not exist) puts the
    /// workspace's icon on the grid itself.
    #[serde(default)]
    pub folder: Option<String>,
    /// Sidebar links, in display order.
    #[serde(default)]
    pub items: Vec<WorkspaceItemSpec>,
}

/// One sidebar link (or group of links) inside a workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceItemSpec {
    /// Display label.
    #[serde(default)]
    pub label: String,
    /// Icon key for the link.
    #[serde(default)]
    pub icon: Option<String>,
    /// `doctype` links open the Desk list/form for that doctype; the internal
    /// route is derived from the module + name. Takes precedence over `href`.
    #[serde(default)]
    pub doctype: Option<String>,
    /// Explicit internal Desk route (or an override for a `doctype` link).
    #[serde(default)]
    pub route: Option<String>,
    /// External URL.
    #[serde(default)]
    pub href: Option<String>,
    /// Open an external URL in a new tab.
    #[serde(default)]
    pub open_in_new_tab: bool,
    /// Non-empty => this item is a group (collapsible dropdown).
    #[serde(default)]
    pub children: Vec<WorkspaceItemSpec>,
}

/// What a sidebar item renders as.
///
/// Serialized lowercase because these strings are the wire `kind` on
/// `/api/modules`: `ferris-api` sends it, `ferris-web` switches on it. The
/// rename is what keeps both sides agreeing on the spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceItemKind {
    /// Link to a doctype's list view.
    Doctype,
    /// External URL.
    Url,
    /// Internal desk route.
    Route,
    /// Group holding nested items (dropdown).
    Group,
}

impl WorkspaceItemKind {
    /// The wire token, matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Doctype => "doctype",
            Self::Url => "url",
            Self::Route => "route",
            Self::Group => "group",
        }
    }
}

impl WorkspaceItemSpec {
    /// Classify the item from its fields: children => group, doctype =>
    /// doctype link, href => external URL, otherwise an internal route.
    pub fn kind(&self) -> WorkspaceItemKind {
        if !self.children.is_empty() {
            WorkspaceItemKind::Group
        } else if self.doctype.is_some() {
            WorkspaceItemKind::Doctype
        } else if self.href.is_some() {
            WorkspaceItemKind::Url
        } else {
            WorkspaceItemKind::Route
        }
    }

    /// Collect every doctype referenced by this item tree (used by the API to
    /// know which doctypes the app already surfaces in a workspace).
    pub fn collect_doctypes<'a>(&'a self, out: &mut Vec<&'a str>) {
        if let Some(dt) = self.doctype.as_deref() {
            out.push(dt);
        }
        for child in &self.children {
            child.collect_doctypes(out);
        }
    }
}

impl WorkspaceFolderSpec {
    /// A new folder with only its id set; label/icon are normalized on
    /// registration.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Replace the human label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Set the icon: a lucide name, or inline SVG markup for the desk grid.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// The host's desk folder: one grid folder holding every built-in workspace.
///
/// The host declares it once and each built-in workspace names it through
/// [`WorkspaceSpec::folder`], so the grid shows one **Ferris** folder instead of
/// nine loose workspace icons. Apps are free to declare their own folders; this
/// one is the framework's.
#[must_use]
pub fn ferris_folder() -> WorkspaceFolderSpec {
    WorkspaceFolderSpec::new(FERRIS_FOLDER)
        .label("Ferris")
        .icon("Boxes")
}

impl WorkspaceSpec {
    /// A new workspace with only its id and label set.
    #[must_use]
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            ..Self::default()
        }
    }

    /// Set the module this workspace belongs to.
    #[must_use]
    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.module = module.into();
        self
    }

    /// Set the icon: a lucide name, or inline SVG markup for the desk grid.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Group this workspace under the named grid folder.
    #[must_use]
    pub fn folder(mut self, folder: impl Into<String>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// Append a sidebar link.
    #[must_use]
    pub fn item(mut self, item: WorkspaceItemSpec) -> Self {
        self.items.push(item);
        self
    }

    /// Append a collapsible group of links.
    #[must_use]
    pub fn group(mut self, label: impl Into<String>, children: Vec<WorkspaceItemSpec>) -> Self {
        self.items.push(WorkspaceItemSpec::group(label, children));
        self
    }
}

impl WorkspaceItemSpec {
    /// A link to a `DocType`'s list (or its form when it is a Single). The label
    /// defaults to the `DocType` name.
    #[must_use]
    pub fn doctype(doctype: impl Into<String>) -> Self {
        let doctype = doctype.into();
        Self {
            label: doctype.clone(),
            doctype: Some(doctype),
            ..Self::default()
        }
    }

    /// A `DocType` link with an explicit display label.
    #[must_use]
    pub fn doctype_titled(doctype: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            doctype: Some(doctype.into()),
            ..Self::default()
        }
    }

    /// A link to an internal Desk route.
    #[must_use]
    pub fn route(route: impl Into<String>) -> Self {
        Self {
            route: Some(route.into()),
            ..Self::default()
        }
    }

    /// A link to an external URL.
    #[must_use]
    pub fn url(href: impl Into<String>) -> Self {
        Self {
            href: Some(href.into()),
            ..Self::default()
        }
    }

    /// A collapsible group holding nested links.
    #[must_use]
    pub fn group(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            children,
            ..Self::default()
        }
    }

    /// Set the icon key (a lucide name).
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Replace the display label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Open an external URL in a new tab.
    ///
    /// The only `const` builder in this impl, and deliberately so: its body is a
    /// plain `bool` assignment, whereas the others take `impl Into<String>` and
    /// cannot be `const` — dropping `const` here would only make
    /// `missing_const_for_fn` fire.
    #[must_use]
    pub const fn new_tab(mut self) -> Self {
        self.open_in_new_tab = true;
        self
    }
}

/// One app's declared desk layout, as it arrived from its `describe()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppWorkspaces {
    /// Owner tag: the plugin/app id that declared this layout.
    pub owner: String,
    /// Folders, in declaration order.
    pub folders: Vec<WorkspaceFolderSpec>,
    /// Workspaces, in declaration order.
    pub workspaces: Vec<WorkspaceSpec>,
}
