//! The service identity the framework reads and writes as on its own behalf.
//!
//! Several framework paths are not the signed-in caller acting:
//!
//! - the notification panel writes the rows a sidebar view needs,
//! - the tag service keeps a catalogue and an association registry in step with
//!   a document that was just saved,
//! - the dashboard feed reads the dashboard definitions that belong to the
//!   whole site.
//!
//! All of them must run as a **service principal** holding `System Manager`,
//! never as the ambient request user, for two reasons:
//!
//! - the derived `DocType`s grant `Create`/`Delete` to `System Manager` only, so
//!   running as the caller would either fail the derived write or force every
//!   caller to hold catalogue rights;
//! - the read is a site-level one (all dashboards, every tag), so it must not be
//!   filtered by whoever happens to be signed in.
//!
//! This is the one place that principal is minted. Each subsystem passes its own
//! purpose id, so a row's `owner`/`modified_by` still says which of them produced
//! it (`tags@ferris.local`, `dashboards@ferris.local`, …), while the *decision*
//! — "the framework runs as `System Manager`" — lives here and nowhere else.

use ferris_auth::User;
use ferris_document::EngineContext;
use ferris_schema::DoctypeMeta;

/// A `System Manager` system user, id'd by the subsystem that mints it.
///
/// `label` is the human name carried alongside the id; both end up in the
/// documents the principal writes.
#[must_use]
pub(crate) fn service_user(id: &str, label: &str) -> User {
    let mut user = User::system_user(id.to_string(), label.to_string());
    user.roles = vec!["System Manager".to_string()];
    user
}

/// An engine context for `doctype` authored as the service principal.
///
/// `meta` is `None` when the `DocType` is not registered; the engine then skips
/// metadata-driven validation, which is the right escape hatch for a catalogue
/// that has not loaded yet.
pub(crate) fn service_context<'a>(
    doctype: &str,
    meta: Option<&'a DoctypeMeta>,
    id: &str,
    label: &str,
) -> EngineContext<'a> {
    let mut ctx = EngineContext::new(doctype, "");
    if let Some(meta) = meta {
        ctx.meta = Some(meta);
    }
    ctx.user = Some(service_user(id, label));
    ctx
}
