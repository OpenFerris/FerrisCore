//! WASM apps the host publishes at boot.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// WASM apps the host publishes at boot.
///
/// `wasm_dir` is the directory the server reads components from. Two layouts
/// are accepted: `plugins/my-app.wasm` (the contract `ferris build` writes, and
/// the one that wins when both exist) and `plugins/my-app/my-app.wasm`, the
/// component inside the app's own world. The scan never goes deeper than one
/// level, so a world's `customizations/`, `files/` or `secrets/` are not
/// mistaken for apps. The default matches the build output directory, so a
/// project built with `ferris build` needs no config.
///
/// `enabled` is an allow-list of plugin names. Empty means "every component in
/// the directory"; a name listed here that has no matching file is a boot error,
/// because an app the operator asked for silently missing is worse than a
/// server that refuses to start.
/// Capabilities are not configured here anymore. An app always has the
/// primordial set (its world — files, blobs and `secrets/.env` —, its own
/// routes including HTTPS, outbound HTTP under policy, the document store,
/// config, schema, the scheduler, notifications and pubsub); the specific ones
/// (egress that widens a host boundary, and API overrides) are granted by
/// declaring them in `describe()`. There is no `trusted`, no capability list and
/// no per-app grant table: one policy, in one place.
///
/// ```toml
/// [apps]
/// wasm_dir = "plugins"
/// enabled = ["my-app"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppsConfig {
    pub enabled: Vec<String>,
    pub wasm_dir: PathBuf,
    /// Port the host serves the apps' `tls-server` routes on. A single-app
    /// install uses this default; two apps with TLS routes need
    /// `plugin_tls_ports` entries, because one port cannot carry two
    /// certificates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_tls_port: Option<u16>,
    /// Per-app TLS listener ports, overriding `plugin_tls_port`.
    #[serde(default)]
    pub plugin_tls_ports: BTreeMap<String, u16>,
}

impl AppsConfig {
    /// The TLS listener port for `app`, if the install configures one.
    #[must_use]
    pub fn tls_port_for(&self, app: &str) -> Option<u16> {
        self.plugin_tls_ports
            .get(app)
            .copied()
            .or(self.plugin_tls_port)
    }
}
