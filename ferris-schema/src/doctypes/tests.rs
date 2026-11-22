use super::*;
use crate::contract::{DashboardView, DocView};

// These tests validate the *declarations* only and deliberately do not call
// `register_doctypes()`: the schema registry is process-global, and the
// migration test in `migration.rs` counts the DocTypes it finds there, so a
// unit test that registered 20 metas would break it. Registration itself is
// exercised in `tests/doctypes_registration.rs`, which runs in its own process.

#[test]
fn every_doctype_has_fields_and_a_known_module() {
    for meta in doctype_metas() {
        assert!(!meta.fields.is_empty(), "{} needs fields", meta.name);
        assert!(
            meta.module == BUILD_MODULE || meta.module == GEO_MODULE,
            "{} has unexpected module {}",
            meta.name,
            meta.module
        );
    }
}

#[test]
fn geo_doctypes_land_in_the_geo_module() {
    for name in ["Country", "Currency", "Language", "Translation"] {
        let meta = doctype_metas()
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} declared"));
        assert_eq!(meta.module, GEO_MODULE, "{name}");
    }
}

#[test]
fn exactly_the_expected_singles() {
    let singles: Vec<_> = doctype_metas()
        .into_iter()
        .filter(|m| m.is_single)
        .map(|m| m.name)
        .collect();
    assert_eq!(
        singles,
        vec![
            "Customize Form",
            "Navbar Settings",
            "Global Search Settings"
        ]
    );
}

#[test]
fn doctype_names_are_unique() {
    let metas = doctype_metas();
    let mut names: Vec<_> = metas.iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), metas.len(), "duplicate DocType names");
}

/// The native `Dashboard` doctype is what makes opening any of its records
/// a dashboard: it declares [`DocView::Dashboard`] with no binding, so the
/// record's own name is its dashboard id. Losing the declaration would
/// send every dashboard document back through the metadata form without
/// anything failing anywhere else.
#[test]
fn the_dashboard_doctype_opens_its_records_as_dashboards() {
    let meta = doctype_metas()
        .into_iter()
        .find(|m| m.name == "Dashboard")
        .expect("the Dashboard doctype is declared");
    assert_eq!(meta.view, DocView::Dashboard);
    assert_eq!(
        meta.dashboard,
        DashboardView::default(),
        "the native binding declares nothing: the document is the dashboard"
    );
}

/// Every other doctype opens its documents in the built-in form, so a view
/// declaration is always something a developer wrote deliberately.
#[test]
fn no_other_doctype_declares_a_view() {
    for meta in doctype_metas()
        .into_iter()
        .filter(|m| m.name != "Dashboard")
    {
        assert_eq!(
            meta.view,
            DocView::Form,
            "{} declares a view kind",
            meta.name
        );
    }
}
