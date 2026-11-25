use axum::http::{Method, StatusCode};

use super::harness::{app, request, test_state};

/// A Single `DocType` over HTTP: `GET`/`PUT /api/single/{doctype}` read and
/// upsert its one document, while the resource `POST` verb is rejected with
/// `405` because a Single cannot hold separate documents.
#[tokio::test]
async fn single_doctype_is_served_by_the_single_routes() {
    let state = test_state().await;
    ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
        name: "Router Single Settings".to_string(),
        module: "Test".to_string(),
        is_single: true,
        fields: vec![ferris_schema::FieldMeta {
            fieldname: "company".to_string(),
            fieldtype: "Data".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });

    // The production router scopes the authenticated user as the ambient
    // engine user (`bind_ambient_user`); this harness has to do the same or
    // every write fails closed. Mirror the identity the harness login opens.
    let mut user = ferris_auth::User::system_user("user-1".to_string(), "User 1".to_string());
    user.id = "user-1".to_string();
    user.roles = vec!["System Manager".to_string()];

    ferris_document::with_user(Some(user), async {
        let (status, body) = request(
            app(&state),
            Method::GET,
            "/api/single/Router%20Single%20Settings",
            None,
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["name"], "Router Single Settings");

        let (status, body) = request(
            app(&state),
            Method::PUT,
            "/api/single/Router%20Single%20Settings",
            Some(serde_json::json!({ "company": "Ferris" }).to_string()),
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["company"], "Ferris");

        // The resource create verb does not apply to a Single.
        let (status, _) = request(
            app(&state),
            Method::POST,
            "/api/resource/Router%20Single%20Settings",
            Some(serde_json::json!({ "x": 1 }).to_string()),
            &state,
            &["System Manager"],
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    })
    .await;
}

/// `GET /api/site-settings` is served (behind the auth layer like the rest of
/// the Desk API) and answers the full `SiteSettings` contract. A blank
/// install answers the all-empty defaults rather than failing, so the Desk
/// chrome (tab title, navbar announcement) never has to special-case a
/// missing setting.
#[tokio::test]
async fn site_settings_route_is_served_with_the_contract() {
    let state = test_state().await;
    let (status, body) = request(
        app(&state),
        Method::GET,
        "/api/site-settings",
        None,
        &state,
        &["System Manager"],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Every contract field is on the wire, so the client never sees a
    // missing key it would have to guess a default for.
    let raw: serde_json::Value = serde_json::from_str(&body).unwrap();
    for key in [
        "title_prefix",
        "home_page",
        "announcement",
        "help_link",
        "logo",
        "language",
    ] {
        assert!(raw.get(key).is_some(), "missing `{key}` in {body}");
    }
    let settings: ferris_schema::contract::SiteSettings = serde_json::from_str(&body).unwrap();
    assert_eq!(settings, ferris_schema::contract::SiteSettings::default());
}
