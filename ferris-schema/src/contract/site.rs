//! Install-wide presentation settings exposed on the wire.
//!
//! These are the values behind `GET /api/site-settings`: the settings the Desk
//! chrome (document title, navbar announcement and help link) and a public
//! website read. `ferris-api` resolves them from the `Website Settings`,
//! `Navbar Settings` and `System Settings` Singles; `ferris-web` deserializes
//! them here. Defining the shape once keeps the two sides from drifting.
//!
//! Every field defaults, so a blank install answers a valid object instead of
//! failing — the same "absent means the compiled default" decision the server
//! makes when a setting was never saved.

use serde::{Deserialize, Serialize};

/// `GET /api/site-settings` — the install-wide presentation settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteSettings {
    /// `Website Settings.title_prefix` — the site name shown in the tab title.
    #[serde(default)]
    pub title_prefix: String,
    /// `Website Settings.home_page` — the route the site treats as its home.
    #[serde(default)]
    pub home_page: String,
    /// `Navbar Settings.announcement` — the message shown across the navbar.
    #[serde(default)]
    pub announcement: String,
    /// `Navbar Settings.help_link` — where the navbar help affordance points.
    #[serde(default)]
    pub help_link: String,
    /// `Navbar Settings.logo` — the logo URL, when one is set.
    #[serde(default)]
    pub logo: String,
    /// `System Settings.language` — the install's default language.
    #[serde(default)]
    pub language: String,
}
