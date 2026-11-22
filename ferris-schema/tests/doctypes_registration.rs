//! Integration coverage for `ferris-schema`'s own DocType declarations.
//!
//! `register_doctypes()` mutates the process-global schema registry, which the
//! in-crate unit tests must not do (the migration test counts what is
//! registered). An integration test is a separate process, so it is the right
//! place to assert that registration is visible and idempotent.

use ferris_schema::{doctypes, get_doctype};

#[test]
fn register_makes_every_meta_visible_with_its_single_flag() {
    doctypes::register_doctypes();
    for meta in doctypes::doctype_metas() {
        let got = get_doctype(&meta.name).unwrap_or_else(|| panic!("{} registered", meta.name));
        assert_eq!(got.is_single, meta.is_single, "{} is_single", meta.name);
        assert_eq!(got.module, meta.module, "{} module", meta.name);
    }
}

#[test]
fn registration_is_idempotent_and_covers_build_and_geo() {
    doctypes::register_doctypes();
    doctypes::register_doctypes();

    for name in [
        "Custom Field",
        "Global Search Settings",
        "Language",
        "Workspace",
    ] {
        assert!(get_doctype(name).is_some(), "{name} registered");
    }
}
