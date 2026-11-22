//! CORS for the authentication surface.
//!
//! [`auth_cors`](crate::middleware::auth_cors) used to echo **any** `Origin`
//! back in `Access-Control-Allow-Origin` together with
//! `Access-Control-Allow-Credentials: true`. That is an origin-reflection hole:
//! any site could read the authenticated responses of a signed-in user, because
//! the browser is told that the attacker's origin is allowed *and* that
//! credentials may travel. The wildcard was correctly avoided (a wildcard cannot
//! be combined with credentials), but reflecting the caller is not better — it
//! simply makes every origin the allow-list.
//!
//! The policy is now an explicit allow-list owned by the deployment:
//!
//! - no policy, or an empty allow-list → **no** CORS header at all, which is the
//!   browser's default (same-origin only);
//! - a request whose `Origin` is not on the list → no header either, and the
//!   browser blocks the response;
//! - a listed origin → the exact origin, never `*`, never a reflection of a
//!   value that was not configured.
//!
//! The list comes from the server settings (`enable_cors` + `cors_origins`), not
//! from the request, so a deployment that disables CORS cannot be talked into it
//! by a header.

use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, header};

/// The origins the auth surface may answer with CORS headers.
///
/// Cheap to clone (`Arc`), which is what lets it be the layer's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorsPolicy {
    origins: Arc<[String]>,
}

impl CorsPolicy {
    /// CORS disabled: every response is same-origin only.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    /// The policy of a deployment: `enabled` is the server's CORS switch and
    /// `origins` its allow-list.
    ///
    /// A disabled deployment yields [`CorsPolicy::disabled`] even when origins
    /// are configured, so turning CORS off cannot leave a half-open surface.
    /// Invalid header values are dropped (they could never be sent); `*` is
    /// refused because it cannot be combined with credentials and is not an
    /// allow-list.
    #[must_use]
    pub fn from_origins(enabled: bool, origins: &[String]) -> Self {
        if !enabled {
            return Self::disabled();
        }
        let allowed: Vec<String> = origins
            .iter()
            .map(|origin| origin.trim())
            .filter(|origin| !origin.is_empty() && *origin != "*")
            .filter(|origin| HeaderValue::from_str(origin).is_ok())
            .map(str::to_string)
            .collect();
        Self {
            origins: allowed.into(),
        }
    }

    /// Build a policy from an explicit allow-list (already known good).
    #[must_use]
    pub fn allow(origins: impl IntoIterator<Item = String>) -> Self {
        Self {
            origins: origins.into_iter().collect(),
        }
    }

    /// Whether `origin` is on the allow-list. Compared exactly: an origin is a
    /// scheme + host + port tuple, and a near-miss (`https://desk.example.com.`)
    /// must not be treated as the configured one.
    #[must_use]
    pub fn allows(&self, origin: &str) -> bool {
        self.origins.iter().any(|allowed| allowed == origin.trim())
    }

    /// The CORS headers for one request, empty when the origin is absent or not
    /// allowed.
    ///
    /// Nothing is emitted for a denied origin — not even
    /// `Access-Control-Allow-Methods`: a response with no `Allow-Origin` is
    /// already blocked by the browser, and advertising the methods would only
    /// describe capabilities the caller may not use.
    #[must_use]
    pub fn headers_for(&self, origin: Option<&str>) -> Vec<(HeaderName, HeaderValue)> {
        let Some(origin) = origin else {
            return Vec::new();
        };
        if !self.allows(origin) {
            return Vec::new();
        }
        let Ok(value) = HeaderValue::from_str(origin.trim()) else {
            return Vec::new();
        };
        vec![
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, value),
            // Only meaningful with a concrete origin, which is exactly the case
            // the allow-list makes possible.
            (
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            ),
            (header::VARY, HeaderValue::from_static("Origin")),
            (
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
            ),
            (
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("Authorization, Content-Type, X-CSRF-Token"),
            ),
            (
                header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("86400"),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CorsPolicy {
        CorsPolicy::from_origins(
            true,
            &[
                "https://desk.example.com".to_string(),
                "http://localhost:3000".to_string(),
            ],
        )
    }

    fn value_of(headers: &[(HeaderName, HeaderValue)], name: HeaderName) -> Option<String> {
        headers
            .iter()
            .find(|(header, _)| *header == name)
            .map(|(_, value)| value.to_str().unwrap_or_default().to_string())
    }

    #[test]
    fn a_listed_origin_is_echoed_with_credentials() {
        let headers = policy().headers_for(Some("https://desk.example.com"));
        assert_eq!(
            value_of(&headers, header::ACCESS_CONTROL_ALLOW_ORIGIN).as_deref(),
            Some("https://desk.example.com")
        );
        assert_eq!(
            value_of(&headers, header::ACCESS_CONTROL_ALLOW_CREDENTIALS).as_deref(),
            Some("true")
        );
        assert_eq!(
            value_of(&headers, header::VARY).as_deref(),
            Some("Origin"),
            "a per-origin answer must not be cached for another origin"
        );
    }

    #[test]
    fn an_unlisted_origin_gets_no_cors_header_at_all() {
        let headers = policy().headers_for(Some("https://evil.example"));
        assert!(headers.is_empty(), "{headers:?}");
    }

    /// The hole this module closes: the request's `Origin` is never reflected
    /// back unless the deployment listed it.
    #[test]
    fn the_request_origin_is_never_reflected_without_a_list() {
        for policy in [
            CorsPolicy::disabled(),
            CorsPolicy::from_origins(true, &[]),
            // CORS switched off, origins configured: still same-origin only.
            CorsPolicy::from_origins(false, &["https://desk.example.com".to_string()]),
        ] {
            assert!(
                policy
                    .headers_for(Some("https://desk.example.com"))
                    .is_empty()
            );
            assert!(policy.headers_for(None).is_empty());
        }
    }

    #[test]
    fn a_wildcard_is_not_an_allow_list_entry() {
        let policy = CorsPolicy::from_origins(true, &["*".to_string()]);
        assert!(
            policy
                .headers_for(Some("https://anything.example"))
                .is_empty()
        );
    }

    #[test]
    fn a_near_miss_origin_is_denied() {
        let policy = policy();
        for origin in [
            "https://desk.example.com.",
            "http://desk.example.com",
            "https://desk.example.com:443",
            "https://desk.example.com.evil.example",
        ] {
            assert!(!policy.allows(origin), "{origin}");
        }
        assert!(policy.allows("  https://desk.example.com  "));
    }

    #[test]
    fn a_request_without_origin_gets_nothing() {
        assert!(policy().headers_for(None).is_empty());
    }
}
