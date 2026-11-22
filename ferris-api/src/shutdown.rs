//! Process-wide shutdown signal for long-lived response streams.
//!
//! `axum::serve(..).with_graceful_shutdown(..)` stops accepting new
//! connections and then waits for every open connection to finish. That is the
//! correct default for ordinary requests, but a Server-Sent Events endpoint or
//! an upgraded WebSocket never finishes on its own: the connection stays open
//! for as long as the browser does. Before this module the server therefore
//! looked hung after SIGTERM/SIGINT whenever a Desk tab had `/events`,
//! `/api/notifications/stream` or socket.io open.
//!
//! The signal is process-wide on purpose: the notification stream lives in
//! `ferris-api`, the SSE bridge in `ferris-server`, and socket.io in
//! `ferris-realtime`; threading a receiver through every router/state type
//! would make each of them know about a lifecycle concern that is the same for
//! the whole process. It is a `watch` channel, not a broadcast of messages:
//! subscribers only need the latest state (`false` while serving, `true` once
//! shutdown begins), and a late subscriber must observe `true` immediately
//! instead of waiting for another notification.
//!
//! This is not a security boundary and it is not used to authorise anything.
//! `stop_on_shutdown` only ends a stream; the handler that created it has
//! already passed its own authentication and channel checks.

use std::sync::{Arc, OnceLock};

use tokio::sync::{mpsc, watch};
use tokio_stream::{Stream, StreamExt as _, wrappers::ReceiverStream};

/// One cloneable handle to the process shutdown state.
#[derive(Clone)]
pub struct Shutdown {
    tx: Arc<watch::Sender<bool>>,
}

impl Shutdown {
    /// A fresh, not-yet-triggered signal.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx: Arc::new(tx) }
    }

    /// Mark the process as shutting down and wake every subscriber.
    ///
    /// Sending the same value twice is harmless; the channel still notifies
    /// waiting receivers, which keeps a repeated SIGTERM from being ignored.
    pub fn trigger(&self) {
        // `send` fails when no receiver is currently subscribed, and the
        // global signal is created with its initial receiver dropped. Use
        // `send_replace` so a trigger before the first SSE/WebSocket
        // subscription is still visible to a receiver created during drain.
        self.tx.send_replace(true);
    }

    /// A receiver that starts with the current state and wakes on changes.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }

    /// Whether shutdown has already been requested.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

/// The signal shared by the server, the API streams and the realtime bridge.
fn global() -> &'static Shutdown {
    static GLOBAL: OnceLock<Shutdown> = OnceLock::new();
    GLOBAL.get_or_init(Shutdown::new)
}

/// Trigger the process shutdown signal.
pub fn trigger() {
    global().trigger();
}

/// Subscribe to the process shutdown signal.
#[must_use]
pub fn subscribe() -> watch::Receiver<bool> {
    global().subscribe()
}

/// End `stream` as soon as the process starts shutting down.
///
/// The returned stream yields exactly the original items, in order, until the
/// shutdown signal flips to `true`; then it yields `None` on the next poll. An
/// SSE response wrapped with this helper closes its connection at that point,
/// which is what lets `axum` finish its graceful drain instead of waiting for
/// the browser to close a stream it never closes itself.
///
/// A small forwarding task is deliberate: a plain `merge` can yield one ready
/// source item before it polls the shutdown branch, while the task can select
/// with `biased;` and stop before forwarding anything once shutdown is set. It
/// also exits when the response stream is dropped (`tx.closed()`), so a browser
/// closing a tab does not leave a task waiting on a broadcast channel forever.
pub fn stop_on_shutdown<S>(stream: S) -> impl Stream<Item = S::Item>
where
    S: Stream + Send + 'static,
    S::Item: Send + 'static,
{
    stop_on_shutdown_with(stream, subscribe())
}

fn stop_on_shutdown_with<S>(
    stream: S,
    shutdown: watch::Receiver<bool>,
) -> impl Stream<Item = S::Item>
where
    S: Stream + Send + 'static,
    S::Item: Send + 'static,
{
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut shutdown = shutdown;
        if *shutdown.borrow() {
            return;
        }
        tokio::pin!(stream);
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
                // The SSE response was dropped (client disconnected).
                () = tx.closed() => break,
                item = stream.next() => {
                    let Some(item) = item else { break };
                    tokio::select! {
                        biased;
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                break;
                            }
                        }
                        sent = tx.send(item) => {
                            if sent.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
    ReceiverStream::new(rx)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::watch;
    use tokio_stream::StreamExt as _;

    use super::stop_on_shutdown_with;

    #[tokio::test]
    async fn a_stream_ends_when_shutdown_is_signalled() {
        let (tx, rx) = watch::channel(false);
        let source = tokio_stream::iter([1, 2]).chain(tokio_stream::pending());
        let mut stream = Box::pin(stop_on_shutdown_with(source, rx));

        assert_eq!(stream.next().await, Some(1));
        assert_eq!(stream.next().await, Some(2));

        tx.send(true).expect("the receiver is alive");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .expect("shutdown must wake the stream"),
            None
        );
    }

    #[test]
    fn a_trigger_before_the_first_subscriber_is_not_lost() {
        let shutdown = super::Shutdown::new();
        shutdown.trigger();
        assert!(shutdown.is_triggered());
        assert!(*shutdown.subscribe().borrow());
    }

    #[tokio::test]
    async fn an_already_triggered_signal_ends_on_the_first_poll() {
        let (_tx, rx) = watch::channel(true);
        let source = tokio_stream::iter([1, 2]);
        let mut stream = Box::pin(stop_on_shutdown_with(source, rx));

        assert_eq!(stream.next().await, None);
    }
}
