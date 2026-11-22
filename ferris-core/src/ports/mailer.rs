//! The outbound-email port.

use crate::Result;

/// One message a module asks the host to deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    /// Explicit sender; `None` lets the host pick its default account.
    pub from: Option<String>,
    pub to: Vec<String>,
    pub subject: String,
    /// Plain-text body.
    pub text: String,
}

impl EmailMessage {
    #[must_use]
    pub fn new(to: impl Into<String>, subject: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            from: None,
            to: vec![to.into()],
            subject: subject.into(),
            text: text.into(),
        }
    }

    #[must_use]
    pub fn from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }
}

/// Deliver an email without owning the SMTP/queue implementation.
// `ferris_core::Error` is the shared taxonomy; see `QueuePort`.
#[allow(clippy::result_large_err)]
pub trait MailerPort: Send + Sync {
    /// Accept `message` for delivery.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the transport could not accept the message. The
    /// caller decides whether that fails the run or is logged and retried.
    fn send(&self, message: &EmailMessage) -> Result<()>;
}
