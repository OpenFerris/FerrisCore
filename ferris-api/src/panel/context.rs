//! Engine contexts and feed-shape mappers shared by the panel routes.

use ferris_core::Value;
use ferris_document::EngineContext;
use ferris_schema::contract::{DeskEvent, Notification, NotificationSettings};

/// How many notifications the panel feed returns.
pub(super) const FEED_LIMIT: u32 = 20;
/// How many upcoming events the panel returns.
pub(super) const EVENT_LIMIT: u32 = 50;

/// Purpose id of the service principal the panel acts as, so the notifications
/// it derives read as the panel's, not as the caller's.
pub(super) const SERVICE_ID: &str = "system@ferris.local";

/// Engine context for a core DocType, authored as the service principal.
///
/// The panel acts on a user's behalf here — every target is derived from the
/// authenticated user, never from the request body — so it needs the rights to
/// write a row the caller would not be allowed to create directly.
pub(crate) fn panel_context<'a>(
    doctype: &str,
    meta: Option<&'a ferris_schema::DoctypeMeta>,
) -> EngineContext<'a> {
    crate::service::service_context(doctype, meta, SERVICE_ID, "System")
}

/// Context for reading a document that **belongs to `user_id`**.
///
/// Only for the login-time lookups (`notify_on_login`), which run before a
/// session exists so there is no authenticated principal to attribute the read
/// to. Frappe does the same thing with a direct
/// `frappe.db.get_value("Notification Settings", user, ...)` and no permission
/// check whatsoever, because the row is the user's own.
///
/// Using [`panel_context`] here would also work — the service user is a
/// `System Manager` — but it would mean the *subject* is not who the read is
/// attributed to, which is exactly the kind of shortcut §C2 is about. The `User`
/// is constructed with only an `id` on purpose: nothing but the owner
/// comparison can succeed with it.
pub(crate) fn subject_context<'a>(
    doctype: &str,
    meta: Option<&'a ferris_schema::DoctypeMeta>,
    user_id: &str,
) -> EngineContext<'a> {
    let mut ctx = EngineContext::new(doctype, "");
    if let Some(meta) = meta {
        ctx.meta = Some(meta);
    }
    ctx.user = Some(ferris_auth::User {
        id: user_id.to_string(),
        ..Default::default()
    });
    ctx
}

pub(super) fn str_field(doc: &Value, key: &str) -> String {
    doc.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Context that reads as the authenticated caller of an HTTP handler.
///
/// The panel's user-facing routes must not read as the service principal: the
/// engine now enforces `Permission::Read` (§C2), and a `System Manager` context
/// would bypass it entirely — the check would be present in the engine and inert
/// in every panel route, which is the failure mode §C1 already demonstrated once.
pub(super) fn caller_context<'a>(
    doctype: &str,
    meta: Option<&'a ferris_schema::DoctypeMeta>,
    user: &ferris_auth::User,
) -> EngineContext<'a> {
    let mut ctx = EngineContext::new(doctype, "");
    if let Some(meta) = meta {
        ctx.meta = Some(meta);
    }
    ctx.user = Some(user.clone());
    ctx
}

/// Map a stored `Notification` document onto the panel's feed shape.
pub(super) fn to_notif(doc: &Value) -> Notification {
    Notification {
        id: str_field(doc, "name"),
        title: str_field(doc, "title"),
        description: str_field(doc, "description"),
        route: str_field(doc, "route"),
        created_at: str_field(doc, "creation"),
        is_read: doc
            .get("is_read")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Map a stored `Event` document onto the panel's feed shape.
pub(super) fn to_event(doc: &Value) -> DeskEvent {
    DeskEvent {
        id: str_field(doc, "name"),
        title: str_field(doc, "title"),
        starts_on: str_field(doc, "starts_on"),
        ends_on: str_field(doc, "ends_on"),
        location: str_field(doc, "location"),
        description: str_field(doc, "description"),
    }
}

/// Settings view with defaults filled in for a missing document.
pub(super) fn settings_view(doc: Option<&Value>, _user_id: &str) -> NotificationSettings {
    let flag = |key: &str| {
        doc.and_then(|d| d.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    };
    NotificationSettings {
        desktop: flag("desktop"),
        notify_on_login: flag("notify_on_login"),
    }
}
