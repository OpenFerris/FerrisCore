//! The document-notification port.
//!
//! `ferris-api` used to call `ferris-realtime` directly, which made the generic
//! REST crate depend on one optional vertical. The realtime engine is optional
//! (a build can omit it), so the dependency is now a port: the API publishes
//! committed document changes to a sink installed by the composition root, and
//! the default sink does nothing.
//!
//! The realtime adapter lives in `ferris-server` (the crate that knows both
//! sides). A module that wants the events subscribes through this port without
//! `ferris-api` importing it.

use std::sync::{Arc, RwLock};

use ferris_core::Value;

/// A sink for document lifecycle events.
///
/// Implementations must not block the write path; the realtime adapter only
/// fans out to already-connected sessions and logs a delivery failure instead
/// of failing the request.
pub trait Notifier: Send + Sync {
    fn doc_inserted(&self, _doctype: &str, _name: &str, _doc: &Value) {}

    fn doc_updated(
        &self,
        _doctype: &str,
        _name: &str,
        _doc: &Value,
        _changes: &Value,
        _previous: &Value,
    ) {
    }

    fn doc_deleted(&self, _doctype: &str, _name: &str, _previous: Option<&Value>) {}

    fn doc_submitted(&self, _doctype: &str, _name: &str, _doc: &Value) {}

    fn doc_cancelled(&self, _doctype: &str, _name: &str, _doc: &Value) {}
}

/// The default sink: no realtime engine is compiled or configured.
pub struct NoopNotifier;

impl Notifier for NoopNotifier {}

/// Process-wide sink. The host installs one implementation at boot, after the
/// realtime handler exists; until then every notification is a no-op.
static NOTIFIER: RwLock<Option<Arc<dyn Notifier>>> = RwLock::new(None);

/// Install the process-wide notifier.
///
/// Returns `false` when another notifier is already installed. The first
/// installation wins, matching the realtime handler's own semantics: a second
/// boot (or a hot reload) must not replace a live sink.
pub fn install(notifier: Arc<dyn Notifier>) -> bool {
    match NOTIFIER.write() {
        Ok(mut slot) => {
            if slot.is_some() {
                return false;
            }
            *slot = Some(notifier);
            true
        }
        Err(error) => {
            tracing::warn!("document notifier lock is poisoned: {error}");
            false
        }
    }
}

fn current() -> Option<Arc<dyn Notifier>> {
    NOTIFIER.read().ok().and_then(|slot| slot.clone())
}

/// Publish a committed insert. See [`Notifier::doc_inserted`].
pub fn notify_doc_insert(doctype: &str, name: &str, doc: &Value) {
    if let Some(notifier) = current() {
        notifier.doc_inserted(doctype, name, doc);
    }
}

/// Publish a committed update. See [`Notifier::doc_updated`].
pub fn notify_doc_update(
    doctype: &str,
    name: &str,
    doc: &Value,
    changes: &Value,
    previous: &Value,
) {
    if let Some(notifier) = current() {
        notifier.doc_updated(doctype, name, doc, changes, previous);
    }
}

/// Publish a committed delete. See [`Notifier::doc_deleted`].
pub fn notify_doc_delete(doctype: &str, name: &str, previous: Option<&Value>) {
    if let Some(notifier) = current() {
        notifier.doc_deleted(doctype, name, previous);
    }
}

/// Publish a submitted document. See [`Notifier::doc_submitted`].
pub fn notify_doc_submit(doctype: &str, name: &str, doc: &Value) {
    if let Some(notifier) = current() {
        notifier.doc_submitted(doctype, name, doc);
    }
}

/// Publish a cancelled document. See [`Notifier::doc_cancelled`].
pub fn notify_doc_cancel(doctype: &str, name: &str, doc: &Value) {
    if let Some(notifier) = current() {
        notifier.doc_cancelled(doctype, name, doc);
    }
}
