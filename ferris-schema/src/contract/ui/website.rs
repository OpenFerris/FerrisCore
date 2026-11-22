//! Website routes an app publishes.

use serde::{Deserialize, Serialize};

/// One website route an app publishes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebsiteRoute {
    pub path: String,
    pub component: String,
    /// Route kind: `"page"` (the default) renders the page inside the desk
    /// chrome; `"bare"` renders it on its own, with no sidebar and no top bar.
    #[serde(rename = "type")]
    pub kind: String,
}

/// Website publishing declaration of an app.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Website {
    #[serde(default)]
    pub routes: Vec<WebsiteRoute>,
}
