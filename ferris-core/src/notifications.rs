//! Process-wide notification fan-out hook.
//!
//! The framework raises in-app notifications from two crates that cannot share
//! a transport directly: the API panel (`ferris-api`) and the WASM plugin host
//! (`ferris-wasm`, which only depends on `ferris-core`). Instead of coupling
//! them, they agree on this hook: whoever owns the live transport (the server)
//! installs a publisher at boot, and every producer calls [`publish`].
//!
//! With no publisher installed — tests, an embedded engine, a role that does not
//! mount the realtime API — [`publish`] is a no-op: raising a notification still
//! stores it, it just has no live audience.

use std::sync::{Arc, OnceLock};

use crate::Value;

/// Receives `(recipient_id, notification)` for every raised notification.
pub type Publisher = Arc<dyn Fn(&str, Value) + Send + Sync + 'static>;

static PUBLISHER: OnceLock<Publisher> = OnceLock::new();

/// Install the process-wide publisher.
///
/// The first call wins; a later call returns `false` so a second server mount
/// cannot silently replace the live publisher.
pub fn set_publisher(publisher: Publisher) -> bool {
    PUBLISHER.set(publisher).is_ok()
}

/// Hand a just-stored notification to the live transport, if one is installed.
pub fn publish(user_id: &str, notification: Value) {
    if let Some(publisher) = PUBLISHER.get() {
        publisher(user_id, notification);
    }
}
