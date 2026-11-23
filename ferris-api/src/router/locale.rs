//! The request's negotiated locale, for the handlers that return labels.
//!
//! Precedence is the same the Desk shell uses: the account's stored language,
//! then `Accept-Language`, then the instance default. Metadata endpoints need it
//! because they project registry data (`DocType` labels, field labels, workspace
//! names) that the catalogue translates; the plain REST endpoints, which return
//! documents rather than labels, do not.
//!
//! The extractor never fails: a request without a header and without a session
//! is served in the default locale.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::ACCEPT_LANGUAGE;
use axum::http::request::Parts;
use ferris_core::i18n::Locale;

use super::ApiState;

/// The locale the host negotiated for this request.
#[derive(Debug, Clone)]
pub struct RequestLocale(pub Locale);

impl<S> FromRequestParts<S> for RequestLocale
where
    S: Send + Sync,
    Arc<ApiState>: FromRef<S>,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let api = Arc::<ApiState>::from_ref(state);
        // The account's choice wins, but only while the deployment still
        // advertises it: a tag left behind by a removed catalogue must not
        // freeze the user in English (or in a language the Desk no longer has).
        let preferred = ferris_document::current_user()
            .and_then(|user| user.language.as_deref().and_then(Locale::parse))
            .filter(|locale| api.i18n.supported().contains(locale));
        let header = parts
            .headers
            .get(ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok());
        Ok(Self(
            preferred.unwrap_or_else(|| api.i18n.negotiate(header)),
        ))
    }
}
