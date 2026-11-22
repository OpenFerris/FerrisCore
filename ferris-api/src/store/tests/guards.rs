use crate::types::{ListParams, Order};

use super::super::query::normalize_order_by;
use super::super::{DocStore, validate_name};
use super::harness::{admin, pool};

/// R-8: the legacy REST `order_by` is widened, aliases are translated and
/// the separate `order` parameter fills in a missing direction.
#[test]
fn order_by_is_widened_and_normalised() {
    let params = |order_by: Option<&str>, order: Option<Order>| ListParams {
        order_by: order_by.map(str::to_string),
        order,
        ..Default::default()
    };

    assert_eq!(normalize_order_by(&params(None, None)), None);
    assert_eq!(normalize_order_by(&params(Some("  "), None)), None);
    // Any document field is accepted, not just the historical three.
    assert_eq!(
        normalize_order_by(&params(Some("qty"), None)),
        Some("qty".to_string())
    );
    // Legacy aliases map to the real columns.
    assert_eq!(
        normalize_order_by(&params(Some("created_at"), None)),
        Some("creation".to_string())
    );
    assert_eq!(
        normalize_order_by(&params(Some("updated_at desc"), None)),
        Some("modified desc".to_string())
    );
    // The expression's own direction wins over `order`.
    assert_eq!(
        normalize_order_by(&params(Some("qty asc"), Some(Order::Desc))),
        Some("qty asc".to_string())
    );
    // `order` fills in when the expression carries none.
    assert_eq!(
        normalize_order_by(&params(Some("name"), Some(Order::Desc))),
        Some("name desc".to_string())
    );
}

#[tokio::test]
async fn test_duplicate_name_conflicts() {
    let pool = pool().await;
    ferris_document::with_user(Some(admin()), async {
        DocStore::create(&pool, "Probe", ferris_core::value!({ "name": "P-1" }))
            .await
            .unwrap();
        assert!(
            DocStore::create(&pool, "Probe", ferris_core::value!({ "name": "P-1" }))
                .await
                .is_err()
        );
    })
    .await;
}

/// M7: `validate_name` used to admit any string but `\0`, so a name could
/// carry `/`, backslashes or control characters straight into a route
/// parameter or file-like primary key.
#[test]
fn test_validate_name_rejects_path_hostile_names() {
    for bad in [
        "a/b",
        "../../etc/passwd",
        "a\\b",
        "a\x00b",
        " leading-space",
        "trailing-space ",
        &"x".repeat(256),
        "",
    ] {
        assert!(
            validate_name(bad).is_err(),
            "accepted a hostile name: {bad:?}"
        );
    }
    for good in ["P-1", "TODO-00001", "SAL-2026-00005", "ACC-hébergeur-01"] {
        assert!(
            validate_name(good).is_ok(),
            "rejected a valid name: {good:?}"
        );
    }
}

/// M7: on `create` the name comes from the request body, so the same gates
/// must apply there — a body minted `/../etc/passwd` would otherwise be a
/// primary key no route handler ever validated.
#[tokio::test]
async fn test_create_refuses_malicious_body_name() {
    let pool = pool().await;
    let err = DocStore::create(
        &pool,
        "Probe",
        ferris_core::value!({ "name": "../../etc/passwd", "title": "x" }),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("invalid document name"), "{err}");
}

#[tokio::test]
async fn test_anonymous_rest_write_is_denied() {
    let pool = pool().await;
    let err = DocStore::create(&pool, "Probe", ferris_core::value!({ "name": "P-1" }))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ferris_core::Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
    ));
}
