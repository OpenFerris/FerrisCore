//! The typed handle bag the composition root installs for modules.

use std::fmt;
use std::sync::{Arc, OnceLock};

use super::{MailerPort, QueuePort};

/// Services a module may consume without knowing which crate implements them.
///
/// Each slot is installed once by the host; a second installation is ignored
/// (and reported by the caller). Cloning the bag shares the same slots.
#[derive(Clone, Default)]
pub struct ServiceHandles {
    queue: Arc<OnceLock<Arc<dyn QueuePort>>>,
    mailer: Arc<OnceLock<Arc<dyn MailerPort>>>,
}

impl ServiceHandles {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the job queue. First installation wins.
    pub fn install_queue(&self, queue: Arc<dyn QueuePort>) -> bool {
        self.queue.set(queue).is_ok()
    }

    /// Install the outbound mailer. First installation wins.
    pub fn install_mailer(&self, mailer: Arc<dyn MailerPort>) -> bool {
        self.mailer.set(mailer).is_ok()
    }

    #[must_use]
    pub fn queue(&self) -> Option<Arc<dyn QueuePort>> {
        self.queue.get().cloned()
    }

    #[must_use]
    pub fn mailer(&self) -> Option<Arc<dyn MailerPort>> {
        self.mailer.get().cloned()
    }

    #[must_use]
    pub fn has_queue(&self) -> bool {
        self.queue.get().is_some()
    }

    #[must_use]
    pub fn has_mailer(&self) -> bool {
        self.mailer.get().is_some()
    }
}

impl fmt::Debug for ServiceHandles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceHandles")
            .field("queue", &self.has_queue())
            .field("mailer", &self.has_mailer())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::Value;
    use crate::ports::{EmailMessage, JobRequest, MailerPort, QueuePort};

    struct OkQueue;
    impl QueuePort for OkQueue {
        fn enqueue(&self, _request: &JobRequest) -> Result<()> {
            Ok(())
        }
    }

    struct OkMailer;
    impl MailerPort for OkMailer {
        fn send(&self, _message: &EmailMessage) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn handles_are_install_once_and_clone_shared() {
        let handles = ServiceHandles::new();
        assert!(!handles.has_queue());
        assert!(!handles.has_mailer());

        assert!(handles.install_queue(Arc::new(OkQueue)));
        assert!(!handles.install_queue(Arc::new(OkQueue)));
        assert!(handles.install_mailer(Arc::new(OkMailer)));

        let clone = handles.clone();
        assert!(handles.has_queue(), "the original keeps its slot");
        assert!(clone.has_queue());
        assert!(clone.has_mailer());
        clone
            .queue()
            .expect("queue")
            .enqueue(&JobRequest::new("task", Value::Null))
            .expect("accepted");
        clone
            .mailer()
            .expect("mailer")
            .send(&EmailMessage::new("a@example.com", "hi", "body"))
            .expect("accepted");
    }
}
