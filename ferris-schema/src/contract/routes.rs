//! Generated desk routes from the schema registry.
//!
//! `ferris-web::generated` builds these on SSR by reading the registry. By
//! moving the struct here, the hydrate build can reason about the route shape
//! without pulling `ferris-schema`'s registry internals across the wasm
//! boundary. It also gives tests a single source of truth for the route
//! contract.

use serde::{Deserialize, Serialize};

/// A generated desk route for a doctype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskRoute {
    pub doctype: String,
    pub module: String,
    /// List route, e.g. `/app/selling/sales-order`.
    pub list_route: String,
    /// Form route, e.g. `/app/selling/sales-order/:name`.
    pub form_route: String,
}

/// Lowercase, dash-separated slug for route segments.
pub fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c == ' ' || c == '_' || c == '/' {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// Build desk routes for an explicit list of `(doctype, module)` pairs.
///
/// Pure helper so it can be tested without touching the global registry.
pub fn desk_routes_for(doctypes: &[(String, String)]) -> Vec<DeskRoute> {
    let mut routes: Vec<DeskRoute> = doctypes
        .iter()
        .map(|(doctype, module)| {
            let base = format!("/desk/{}/{}", slugify(module), slugify(doctype));
            DeskRoute {
                doctype: doctype.clone(),
                module: module.clone(),
                list_route: base.clone(),
                form_route: format!("{base}/:name"),
            }
        })
        .collect();
    routes.sort_by(|a, b| a.list_route.cmp(&b.list_route));
    routes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Sales Order"), "sales-order");
        assert_eq!(slugify("Stock_Entry"), "stock-entry");
        assert_eq!(slugify(" Buying "), "buying");
        assert_eq!(slugify("desk/test"), "desk-test");
    }

    #[test]
    fn test_desk_routes_for() {
        let routes = desk_routes_for(&[
            ("Sales Order".to_string(), "Selling".to_string()),
            ("Item".to_string(), "Stock".to_string()),
        ]);
        assert_eq!(routes.len(), 2);
        // Sorted by list route: selling/sales-order before stock/item.
        assert_eq!(routes[0].list_route, "/desk/selling/sales-order");
        assert_eq!(routes[1].list_route, "/desk/stock/item");
        assert_eq!(routes[1].form_route, "/desk/stock/item/:name");
    }
}
