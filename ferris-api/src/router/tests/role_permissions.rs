use axum::http::{Method, StatusCode};
use ferris_auth::middleware::AuthenticatedUser;

use super::super::role_permissions::is_permissions_admin;
use super::harness::{app, request, test_state};

/// §C3(a): the REST surface. An Administrator may read the Role Permissions
/// store; a plain user is refused before any row is touched.
#[tokio::test]
async fn role_permissions_answers_only_to_permissions_admins() {
    let state = test_state().await;
    let (status, body) = request(
        app(&state),
        Method::GET,
        "/api/authz/role-permissions",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows, serde_json::json!([]), "an empty store lists nothing");

    let (status, body) = request(
        app(&state),
        Method::GET,
        "/api/authz/role-permissions",
        None,
        &state,
        &["Sales"],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// §C3(a): an upsert writes a row, live-reinstalls the engine, and echoes
/// the store's view of that `DocType` back. The POST accepts a partial row —
/// the desk sends only the rights it is changing.
#[tokio::test]
async fn an_admin_upsert_persists_and_echoes_the_row() {
    let state = test_state().await;
    let (status, body) = request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(
            serde_json::json!({
                "doctype": "ToDo",
                "role": "Sales",
                "read": true,
            })
            .to_string(),
        ),
        &state,
        &["Administrator"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["doctype"], "ToDo");
    assert_eq!(rows[0]["role"], "Sales");
    assert_eq!(rows[0]["read"], true);
    assert_eq!(rows[0]["write"], false, "untouched rights stay off");

    let (status, body) = request(
        app(&state),
        Method::GET,
        "/api/authz/role-permissions?doctype=ToDo",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.len(), 1, "the row survived a second read");
}

/// §C3(a): DELETE removes every row for the pair and the live reinstall
/// drops the grant with it.
#[tokio::test]
async fn an_admin_delete_removes_the_rows_and_the_grant() {
    let state = test_state().await;
    let path = "/api/authz/role-permissions/ToDo/Sales";
    let (status, _) = request(
        app(&state),
        Method::DELETE,
        path,
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "deleting nothing is not an error"
    );

    let (status, _) = request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(
            serde_json::json!({
                "doctype": "ToDo",
                "role": "Sales",
                "read": true,
            })
            .to_string(),
        ),
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, mut body) = request(
        app(&state),
        Method::DELETE,
        path,
        None,
        &state,
        &["System Manager"],
    )
    .await;
    body.clear(); // NO_CONTENT carries no body
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = request(
        app(&state),
        Method::GET,
        "/api/authz/role-permissions?doctype=ToDo",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert!(rows.is_empty(), "the deleted row is not read back");
}

/// §C3(a): a non-admin POST is refused too — the gate covers every verb,
/// not just reads.
#[tokio::test]
async fn a_non_admin_cannot_write_role_permissions() {
    let state = test_state().await;
    let (status, body) = request(
        app(&state),
        Method::POST,
        "/api/authz/role-permissions",
        Some(
            serde_json::json!({
                "doctype": "ToDo",
                "role": "Sales",
                "read": true,
            })
            .to_string(),
        ),
        &state,
        &["Sales"],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// The gate alone: exactly the roles that can restructure the engine may
/// edit Role Permissions — ferris-auth's `Administrator` and `System
/// Manager`, and nobody else.
#[test]
fn the_permissions_admin_gate_is_exactly_administrator_and_system_manager() {
    let user = |roles: &[&str]| {
        let mut u = ferris_auth::User::new(
            "user@example.com".to_string(),
            "User One".to_string(),
            ferris_auth::UserType::SystemUser,
        );
        u.id = "user-1".to_string();
        u.roles = roles.iter().map(|r| r.to_string()).collect();
        AuthenticatedUser(u)
    };
    assert!(is_permissions_admin(&user(&["System Manager"])));
    assert!(is_permissions_admin(&user(&["Administrator"])));
    assert!(is_permissions_admin(&user(&["Sales", "System Manager"])));
    assert!(!is_permissions_admin(&user(&["Sales"])));
    assert!(!is_permissions_admin(&user(&[])));
}
