//! Client-certificate identity facts.
//!
//! When the host terminates mutual TLS it has verified a client certificate
//! against the operator's CA, and the only thing it can honestly say about the
//! caller is what the certificate declares. [`ClientIdentity`] is exactly that
//! set of facts — subject, common name, issuer and SHA-256 fingerprint — with
//! no opinion about what they *mean*. Mapping a certificate to a device, a
//! gateway or an enrolment record is domain knowledge and lives in the plugin,
//! not here; keeping the host type this thin is what preserves that split.
//!
//! The type is deliberately plain (no TLS/X.509 dependency) so it can travel
//! through `ferris-server` (which extracts it from the peer certificate),
//! `ferris-wasm` (which exposes it to the guest) and the WIT boundary without
//! dragging a TLS stack into either.

use serde::{Deserialize, Serialize};

/// The verifiable facts of a client certificate: who it says the caller is and
/// which CA vouched for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientIdentity {
    /// Full RFC 2253 subject, e.g. `O=Solmec,CN=SOLMEC-001`.
    pub subject: String,
    /// The subject's common name, when the certificate carries one.
    pub common_name: Option<String>,
    /// Lowercase hex SHA-256 of the DER certificate. Stable for the lifetime of
    /// the certificate, so it is the natural revocation key.
    pub fingerprint: String,
    /// Full RFC 2253 issuer distinguished name.
    pub issuer: String,
}
