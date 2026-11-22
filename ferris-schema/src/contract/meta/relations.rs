//! Document actions, links and the Connections payload.

use serde::{Deserialize, Serialize};

/// One document action, as the server reads it from a stored `DocType Action`
/// row.
///
/// Actions are declared on the DocType (form/list/report), not on each
/// document, so they travel with the metadata instead of being hardcoded in
/// the Desk. Every field is defaulted because the rows are stored data: an
/// older or hand-authored row may omit any of them and must still parse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynDocAction {
    /// Row name, so the Customize editor can delete exactly this action.
    #[serde(default)]
    pub name: String,
    /// Button label; empty when the row did not set one.
    #[serde(default)]
    pub label: String,
    /// `Server Action` or `Route` (Frappe's `action_type`).
    #[serde(default)]
    pub action_type: String,
    /// The method/route the action invokes.
    #[serde(default)]
    pub action: String,
    /// Optional grouping of actions in the Desk.
    #[serde(default)]
    pub group: Option<String>,
    /// Hidden actions stay declared but are not offered in the UI.
    #[serde(default)]
    pub hidden: bool,
}

/// One document link, as the server reads it from a stored `DocType Link` row.
///
/// A link declares a Connections-panel relationship from this doctype to
/// another (`link_doctype`) through a field (`link_fieldname`), optionally
/// nested inside a child table (`table_fieldname`). Defaulted for the same
/// reason as [`DynDocAction`]: the rows are stored data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynDocLink {
    /// Row name, so the Customize editor can delete exactly this link.
    #[serde(default)]
    pub name: String,
    /// DocType on the other side of the link.
    #[serde(default)]
    pub link_doctype: String,
    /// Field on this doctype that points at the linked document.
    #[serde(default)]
    pub link_fieldname: String,
    /// Child table carrying the link field, when it is not on the parent.
    #[serde(default)]
    pub table_fieldname: Option<String>,
    /// Optional grouping of links in the Connections panel.
    #[serde(default)]
    pub group: Option<String>,
    /// Hidden links stay declared but are not offered in the UI.
    #[serde(default)]
    pub hidden: bool,
}

/// One document in a Connections group.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionDocument {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// One connected DocType and the documents that point at the current row.
///
/// The server builds these from the `DocType Link` declarations; the Desk only
/// renders them, so the shape lives in the contract instead of in either side.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionGroup {
    #[serde(default)]
    pub link_doctype: String,
    #[serde(default)]
    pub link_fieldname: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub documents: Vec<ConnectionDocument>,
}

/// The Connections payload of one document
/// (`GET /api/doctype/connections/{doctype}/{name}`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Connections {
    #[serde(default)]
    pub connections: Vec<ConnectionGroup>,
}
