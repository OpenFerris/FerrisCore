//! The job-queue port.

use crate::{Result, Value};

/// One job a module wants the host to enqueue.
///
/// The shape is intentionally transport-neutral: the host's queue adapter maps
/// it onto `ferris_jobs::Job`, while a test can implement [`QueuePort`] without
/// pulling the Redis/SQL queue.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRequest {
    /// The job type routed to `handle-job` (or a host worker).
    pub job_type: String,
    /// The job's JSON payload.
    pub payload: Value,
    /// Queue name; `None` means the host's default queue.
    pub queue: Option<String>,
}

impl JobRequest {
    #[must_use]
    pub fn new(job_type: impl Into<String>, payload: Value) -> Self {
        Self {
            job_type: job_type.into(),
            payload,
            queue: None,
        }
    }

    #[must_use]
    pub fn on_queue(mut self, queue: impl Into<String>) -> Self {
        self.queue = Some(queue.into());
        self
    }
}

/// Enqueue a job without owning the queue implementation.
// `ferris_core::Error` is the shared taxonomy; boxing it here would make every
// module match on a Box it never inspects.
#[allow(clippy::result_large_err)]
pub trait QueuePort: Send + Sync {
    /// Accept `request` for delivery.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the host cannot accept the job (a closed queue, a
    /// serialization failure, or no runtime to hand it to). A module must not
    /// retry blindly: the error is the host's classification.
    fn enqueue(&self, request: &JobRequest) -> Result<()>;
}
