//! Process-wide audit events.
//!
//! The writers live in the host (`ferris-server`'s `observability`), but the
//! operations worth recording happen *below* it: a login is completed in
//! `ferris-api`, a file read in `ferris-files`. Depending on the server from
//! either crate would invert the graph, so this module is the seam: a plain
//! event type, a sink installed once by the composition root, and a `record`
//! that is a no-op when no host is listening (tests, a library embedder).
//!
//! A sink is a `Fn` that must not block the caller: the server's sink spawns
//! the database write and returns immediately, so an audit failure can never
//! turn a successful login into an error.

use std::sync::{Arc, OnceLock};

/// One auditable operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEvent {
    /// A session was opened (`Activity Log`, operation `login`).
    Login { user: String, ip: String },
    /// A session was closed by the caller (`Activity Log`, operation `logout`).
    Logout { user: String, ip: String },
    /// A login was refused (`Activity Log`, operation `login_failed`). The
    /// `user` is the resolved account id, or `unknown` when the submitted
    /// identity matched no row; the password and any second-factor code are
    /// never part of the event.
    LoginFailed { user: String, ip: String },
    /// A stored file was read or exported (`Access Log`).
    FileAccess {
        user: String,
        file_type: String,
        reference: String,
    },
}

type Sink = dyn Fn(&AuditEvent) + Send + Sync;

static SINK: OnceLock<Arc<Sink>> = OnceLock::new();

/// Install the process-wide audit sink.
///
/// Returns `false` when a sink was already installed (the first one wins, so a
/// hot reload cannot swap the writer under an in-flight request).
pub fn set_sink(sink: Arc<Sink>) -> bool {
    SINK.set(sink).is_ok()
}

/// Record `event`. A no-op until a sink is installed.
pub fn record(event: AuditEvent) {
    if let Some(sink) = SINK.get() {
        sink(&event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recording without a host is safe: the library embedder and the unit
    /// tests run every login path without a sink.
    #[test]
    fn recording_without_a_sink_is_a_no_op() {
        record(AuditEvent::Login {
            user: "ada@example.com".into(),
            ip: "127.0.0.1".into(),
        });
    }
}
