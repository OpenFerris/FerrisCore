//! Ports between the host and the optional service verticals.
//!
//! The composition root owns the concrete queue and email transport, but a
//! module must be able to enqueue a job or send a message without depending on
//! `ferris-jobs` or `ferris-email` (that would invert the dependency direction).
//! These traits are the seam: they live in `ferris-core`, the bottom of the
//! graph, and each vertical implements them in its own crate.
//!
//! [`ServiceHandles`] is the typed bag the host installs once and hands to a
//! module through `ServiceContext`. It uses `OnceLock` (first installation wins)
//! so a second boot or a hot reload cannot replace the live service.

mod handles;
mod mailer;
mod queue;

pub use handles::ServiceHandles;
pub use mailer::{EmailMessage, MailerPort};
pub use queue::{JobRequest, QueuePort};
