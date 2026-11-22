//! Live notification delivery: the in-process bus and the SSE stream.
//!
//! `panel::push_notification` stores a `Notification` document; this module
//! carries that same payload to every open Desk session of the recipient.
//!
//! The stream endpoint is mounted inside the guarded API router and derives the
//! recipient from the **authenticated** user, never from the request. A client
//! therefore cannot subscribe to another user's notifications, whatever it asks
//! for — the bus is process-wide, but the filter is per connection and
//! server-owned.
//!
//! Scale-out caveat: the bus below is a `OnceLock` singleton in
//! *this* process, so an event pushed on one API instance is invisible to a
//! subscriber connected to another. A multi-process deployment needs a shared
//! fan-out (Redis pub/sub or similar); the current design assumes a single API
//! process, recorded here so the assumption is not discovered the hard way.

use std::sync::{Arc, OnceLock};

use axum::extract::FromRef;
use axum::response::sse::{Event, KeepAlive, Sse};
use ferris_auth::AuthState;
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::Value;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// How many notification events a slow subscriber may fall behind by before it
/// is told it lagged. On lag the oldest queued events are dropped; a dropped
/// notification is still visible the next time the feed is fetched.
const BUFFER: usize = 256;

/// One live notification addressed to one recipient.
#[derive(Clone, Debug)]
pub struct NotificationEvent {
    pub user: String,
    pub notification: Value,
}

/// The process-wide bus, created on first use so producers and the stream share
/// one channel without threading state through the router.
fn bus() -> &'static broadcast::Sender<NotificationEvent> {
    static BUS: OnceLock<broadcast::Sender<NotificationEvent>> = OnceLock::new();
    BUS.get_or_init(|| broadcast::channel(BUFFER).0)
}

/// Publish a notification that was just stored for `user`.
pub fn publish(user: &str, notification: Value) {
    // A send error only means there is no subscriber right now.
    let _ = bus().send(NotificationEvent {
        user: user.to_string(),
        notification,
    });
}

/// Wire the `ferris-core` publish hook into this bus.
///
/// The server calls this once at boot so plugin notifications raised by
/// `ferris-wasm` (which only depends on `ferris-core`) reach the same
/// subscribers. Returns `false` if a publisher was already installed.
#[must_use]
pub fn install() -> bool {
    ferris_core::notifications::set_publisher(Arc::new(|user, notification| {
        publish(user, notification);
    }))
}

/// `GET /api/notifications/stream` — a per-user Server-Sent Events stream.
///
/// Each event carries the same JSON object the feed endpoint returns, under the
/// default (unnamed) event type so the browser's `onmessage` handler receives
/// it. The connection stays open until the client goes away; `EventSource`
/// reconnects on its own if it drops.
///
/// The `S` type parameter mirrors the other panel handlers: the route is mounted
/// on the generic API router, and `AuthenticatedUser` needs `AuthState` to be
/// reachable from whatever state it resolves.
pub async fn notifications_stream<S>(
    user: AuthenticatedUser,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, axum::Error>>>
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    let user_id = user.0.id;
    let stream = BroadcastStream::new(bus().subscribe()).filter_map(move |item| {
        let event = item.ok()?;
        if event.user != user_id {
            return None;
        }
        let data = serde_json::to_string(&event.notification).ok()?;
        Some(Ok(Event::default().data(data)))
    });
    // A browser `EventSource` never closes on its own, so without this the
    // graceful drain would wait forever for `/api/notifications/stream`.
    let stream = crate::shutdown::stop_on_shutdown(stream);
    Sse::new(stream).keep_alive(KeepAlive::default())
}
