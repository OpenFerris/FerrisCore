//! Multi-site model: `sites/<site>/` and the global `apps/` catalogue.
//!
//! One process serves one site (`ferris serve --site <name>`), but a project may
//! hold several: each owns its database settings, its files roots and its plugin
//! **instances**. An app is installed globally once and then copied into a site,
//! so upgrading the app never overwrites the site's data. That two-scope rule is
//! the whole design; [`SiteManager`] documents the layout it enforces.
//!
//! ```text
//! sites/<site>/plugins/<app>/   instance: the directory a guest mounts at /world
//! apps/<app>/world/             template: read-only source of that instance
//! ```

mod apps;
mod config;
mod fsx;
mod manager;
mod manifest;
#[cfg(test)]
mod tests;

pub use config::SiteConfig;
pub use manager::{
    APPS_FILE, ASSETS_DIR, COMMON_CONFIG_FILE, CONFIG_FILE, SITE_CONFIG_FILE, SiteManager,
    is_valid_site_name,
};
pub use manifest::{AppManifest, PLUGIN_MANIFEST, TEMPLATE_WORLD};
