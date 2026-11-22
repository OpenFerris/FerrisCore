//! Install-wide presentation settings, served at `GET /api/site-settings`.
//!
//! The Desk chrome (tab title, navbar announcement and help link) and a public
//! website read these values so the install's own configuration drives the UI
//! rather than a hardcoded string. They live in three Singles — `Website
//! Settings`, `Navbar Settings` and `System Settings` — which are guarded by
//! `System Manager`: a visitor, or any non-manager, has no `Read` on them. The
//! values are install-wide and non-secret (a site name, an announcement), so
//! they are resolved here once, with the read check bypassed, and projected
//! onto the public [`SiteSettings`] contract.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use ferris_core::Value;
use ferris_schema::contract::SiteSettings;

use crate::router::ApiState;
use crate::store::DocStore;

/// `GET /api/site-settings` — the install's presentation settings.
///
/// Every field defaults, so a blank install answers a valid object instead of
/// failing; a missing or unreadable Single simply contributes empty strings.
pub async fn site_settings(State(state): State<Arc<ApiState>>) -> Response {
    let website = DocStore::get_single_system(&state.pool, "Website Settings")
        .await
        .ok();
    let navbar = DocStore::get_single_system(&state.pool, "Navbar Settings")
        .await
        .ok();
    let system = DocStore::get_single_system(&state.pool, "System Settings")
        .await
        .ok();

    Json(project(website.as_ref(), navbar.as_ref(), system.as_ref())).into_response()
}

/// Project the three settings documents onto the wire contract.
fn project(
    website: Option<&Value>,
    navbar: Option<&Value>,
    system: Option<&Value>,
) -> SiteSettings {
    SiteSettings {
        title_prefix: field(website, "title_prefix"),
        home_page: field(website, "home_page"),
        announcement: field(navbar, "announcement"),
        help_link: field(navbar, "help_link"),
        logo: field(navbar, "logo"),
        language: field(system, "language"),
    }
}

/// A string field of an optional document, or `""` when the document or field
/// is absent.
fn field(doc: Option<&Value>, name: &str) -> String {
    doc.and_then(|d| d.get(name))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each value is sourced from its own Single, and a missing document (an
    /// installation that never saved a setting) yields the empty default rather
    /// than a panic or a `null`.
    #[test]
    fn project_sources_each_field_from_its_single() {
        let website = ferris_core::value!({
            "title_prefix": "Acme",
            "home_page": "/home",
        });
        let navbar = ferris_core::value!({
            "announcement": "Maintenance tonight",
            "help_link": "https://help.acme.test",
        });

        let settings = project(Some(&website), Some(&navbar), None);
        assert_eq!(settings.title_prefix, "Acme");
        assert_eq!(settings.home_page, "/home");
        assert_eq!(settings.announcement, "Maintenance tonight");
        assert_eq!(settings.help_link, "https://help.acme.test");
        assert_eq!(settings.logo, "");
        assert_eq!(settings.language, "");
    }

    /// A blank install answers the all-empty contract instead of failing.
    #[test]
    fn project_defaults_when_nothing_is_saved() {
        assert_eq!(
            project(None, None, None),
            ferris_schema::contract::SiteSettings::default()
        );
    }
}
