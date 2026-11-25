//! Install-wide feature switches and the `Secret` key material.

use serde::{Deserialize, Serialize};

use super::redact::Redacted;
use std::fmt;

/// Install-wide feature switches (`[features]`).
///
/// Each flag removes a concrete piece of the host; `true` never adds anything
/// the operator did not request.
///
/// * `realtime`, `jobs`, `scheduler`: the corresponding **server role** is not
///   started, even when `--all` or `roles = ["all"]` asks for it. The server
///   logs which feature removed it.
/// * `email`: the email hub and worker are not built.
/// * `files`: the `/files` signed-download route is not mounted.
/// * `print`, `report`, `search`: the `/api/print*`, `/api/report*` and
///   `/api/search` routes are not mounted at all (the request answers 404).
/// * `webhooks`: the integrations runtime does not dispatch webhook rows.
///   Automation, assignment, milestones and push delivery keep running.
///
/// A disabled route is absent, not a warning: the API state carries these flags
/// and the router omits the routes, so a client cannot mistake "disabled" for
/// "no results".
///
/// These are **runtime** gates. They are checked against the compiled Cargo
/// feature set at boot (`ferris_server::host_features`): a flag that asks for a
/// subsystem the binary was not built with is a boot error, never a silent
/// no-op.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlags {
    #[serde(default = "default_true")]
    pub realtime: bool,
    #[serde(default = "default_true")]
    pub jobs: bool,
    #[serde(default = "default_true")]
    pub scheduler: bool,
    #[serde(default = "default_true")]
    pub email: bool,
    #[serde(default = "default_true")]
    pub files: bool,
    #[serde(default = "default_true")]
    pub print: bool,
    #[serde(default = "default_true")]
    pub report: bool,
    #[serde(default = "default_true")]
    pub search: bool,
    #[serde(default = "default_true")]
    pub webhooks: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            realtime: true,
            jobs: true,
            scheduler: true,
            email: true,
            files: true,
            print: true,
            report: true,
            search: true,
            webhooks: true,
        }
    }
}

/// Key material for `Secret` fields: AES-256-GCM at rest, recoverable by a
/// trusted server-side context.
///
/// The key is **not** derived from any authentication setting: rotating a
/// login credential must not silently make every stored OAuth/webhook secret
/// undecryptable.
/// Rotation is explicit — install the new key as `key` and keep the old one in
/// `previous_keys` until every row has been rewritten.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SecretsConfig {
    /// 32 bytes, as 64 hex characters or base64. `None` means "not configured".
    pub key: Option<String>,
    /// Retired keys, still accepted for decryption while rows are rewritten.
    #[serde(default)]
    pub previous_keys: Vec<String>,
}

impl fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("key", &self.key.as_ref().map(|_| Redacted))
            .field("previous_keys", &self.previous_keys.len())
            .finish()
    }
}
