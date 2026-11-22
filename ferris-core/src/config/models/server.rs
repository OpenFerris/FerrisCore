//! HTTP listener, TLS/mTLS and the internal certificate authority.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub request_timeout_secs: u64,
    pub body_limit_mb: usize,
    pub enable_cors: bool,
    pub cors_origins: Vec<String>,
    /// Peer addresses (or CIDR networks) whose `X-Forwarded-For` / `X-Real-IP`
    /// headers are believed. Empty is the safe default: a forwarding header
    /// from an unlisted peer is ignored and the audit row records the TCP peer
    /// instead, so a caller cannot choose its own address.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    pub tls: Option<TlsConfig>,
    /// Internal certificate authority that signs device CSRs (mTLS plan Phase
    /// 3). `None` leaves the plugin `pki` imports reporting the capability as
    /// unavailable rather than fabricating a certificate.
    #[serde(default)]
    pub pki: Option<PkiConfig>,
    /// Public origin this deployment is reached at, e.g.
    /// `https://erp.example.com`. Used for absolute URLs that leave the process
    /// (OAuth redirect URIs today). `None` falls back to the bind `host:port`,
    /// which is only right for local development: behind a reverse proxy or on
    /// a wildcard bind the advertised origin has to be explicit or providers
    /// reject the callback (SRV2).
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default = "default_roles")]
    pub roles: Vec<String>,
}

pub(crate) fn default_roles() -> Vec<String> {
    vec!["all".into()]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// PEM certificate chain for the server (leaf first).
    pub cert_path: PathBuf,
    /// PEM private key for `cert_path` (PKCS#8, PKCS#1 or SEC1).
    pub key_path: PathBuf,
    /// PEM bundle of the CA(s) that sign *client* certificates. When set the
    /// server requests a client certificate and verifies it against these
    /// trust anchors. `None` disables client-certificate authentication
    /// (ordinary server-only TLS).
    #[serde(default)]
    pub client_ca_path: Option<PathBuf>,
    /// Require a verified client certificate rather than accepting anonymous
    /// clients. Only meaningful together with `client_ca_path`; setting it
    /// without a CA is rejected at boot. This is what turns TLS into mTLS.
    #[serde(default)]
    pub require_client_cert: bool,
    /// Minimum negotiated TLS version. Accepts `"1.2"` (default) or `"1.3"`.
    #[serde(default)]
    pub min_version: Option<String>,
}

/// The floor set by [`TlsConfig::min_version`], resolved from its free-form
/// string form so the config layer stays independent of the TLS backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMinVersion {
    Tls12,
    Tls13,
}

impl TlsConfig {
    /// Resolve [`Self::min_version`] into a typed floor, defaulting to TLS 1.2.
    ///
    /// Kept in `ferris-core` (and free of any rustls types) so a bad value is
    /// caught the same way regardless of which crate builds the listener.
    pub fn min_version(&self) -> std::result::Result<TlsMinVersion, String> {
        match self.min_version.as_deref().map(str::trim) {
            None | Some("") | Some("1.2") | Some("tls1.2") | Some("tls12") => {
                Ok(TlsMinVersion::Tls12)
            }
            Some("1.3") | Some("tls1.3") | Some("tls13") => Ok(TlsMinVersion::Tls13),
            Some(other) => Err(format!(
                "unknown tls.min_version {other:?}: expected \"1.2\" or \"1.3\""
            )),
        }
    }
}

/// Internal certificate authority (mTLS plan Phase 3).
///
/// The host holds the CA key and signs the PKCS#10 requests devices submit; it
/// never hands the key to a plugin (the `pki` WIT import only returns issued
/// material). Revocation is tracked in a host-owned table and consulted during
/// the TLS handshake, so a revoked certificate is refused before any route runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkiConfig {
    /// PEM certificate of the CA that signs device certificates. The same
    /// material is returned by the `pki::ca-cert` import so a client can be
    /// provisioned to trust the issued leaves.
    pub ca_cert_path: PathBuf,
    /// PEM private key for `ca_cert_path` (PKCS#8). It must use a key type the
    /// `ring` backend can sign with — ECDSA P-256/P-384 or Ed25519. An RSA CA
    /// key is rejected at boot rather than producing an unusable certificate.
    pub ca_key_path: PathBuf,
    /// Leaf lifetime in seconds used when a caller asks for a TTL of `0`.
    #[serde(default = "default_pki_ttl_seconds")]
    pub default_ttl_seconds: u64,
}

/// 90 days: short enough that a leaked device key ages out, long enough that a
/// fleet does not re-enrol constantly.
const fn default_pki_ttl_seconds() -> u64 {
    90 * 24 * 60 * 60
}
