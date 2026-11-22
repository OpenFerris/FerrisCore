//! Architecture guard: the generic API must not depend on a vertical.
//!
//! `ferris-api` defines the native [`ferris_api::Module`] contract and the
//! generic REST surface. A vertical (`ferris-print`, `ferris-report`, …)
//! implements the contract and depends on the API, never the reverse. The
//! inversion is easy to undo with one convenient `use`, so this test fails the
//! moment the manifest grows a direct edge back to a feature.

#[test]
fn the_api_manifest_has_no_vertical_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "ferris-files",
        "ferris-email",
        "ferris-integrations",
        "ferris-jobs",
        "ferris-print",
        "ferris-report",
        "ferris-realtime",
        "ferris-scheduler",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "`ferris-api` must not depend on `{forbidden}`; implement the feature \
             as a `Module` in its own crate and let `ferris-server` compose it"
        );
    }
}
