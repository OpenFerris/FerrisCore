//! Cookie and lifetime settings for the `tower-sessions` cookie.
//!
//! The old `SessionConfig` lived in the bespoke session module and described a
//! hand-rolled cookie plus an in-memory session row. This one describes only
//! what `tower_sessions::SessionManagerLayer` needs: the cookie name and its
//! attributes, and the two expiry windows the framework enforces on top of it.
//!
//! # Secure cookies
//!
//! When the deployment terminates TLS (or a trusted proxy does), the cookie is
//! issued as `__Host-ferris_session`: the `__Host-` prefix is a browser-enforced
//! contract that the cookie is `Secure`, has `Path=/` and carries no `Domain`.
//! It therefore cannot be shadowed by a subdomain. On plain HTTP (the dev
//! profile) the plain `ferris_session` name is used, because a browser discards
//! a `Secure` cookie on an `http://` origin.

/// Name used when the deployment is TLS-terminated.
pub const SECURE_COOKIE_NAME: &str = "__Host-ferris_session";

/// Name used on plain HTTP (development).
pub const INSECURE_COOKIE_NAME: &str = "ferris_session";

/// Cookie and lifetime settings for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// Cookie name. [`SessionConfig::with_secure`] derives it from the TLS
    /// decision; set it explicitly only for a non-standard deployment.
    pub cookie_name: String,
    /// Cookie domain, when the Desk is served from a parent domain. Never set
    /// together with the `__Host-` prefix.
    pub cookie_domain: Option<String>,
    /// Cookie path. `__Host-` requires `/`.
    pub cookie_path: String,
    /// Whether the cookie carries the `Secure` attribute.
    pub secure_cookies: bool,
    /// Absolute session lifetime, in minutes. Enforced by the auth middleware
    /// in addition to the browser cookie's own lifetime.
    pub timeout_minutes: u64,
    /// Sliding idle window, in minutes. This is what
    /// `tower_sessions::Expiry::OnInactivity` enforces in the store.
    pub idle_timeout_minutes: u64,
    /// Concurrent sessions one account may hold. A login beyond this is
    /// refused (429) instead of silently evicting an older tab.
    pub max_sessions_per_user: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            cookie_name: INSECURE_COOKIE_NAME.to_string(),
            cookie_domain: None,
            cookie_path: "/".to_string(),
            secure_cookies: false,
            timeout_minutes: 60,
            idle_timeout_minutes: 30,
            max_sessions_per_user: 5,
        }
    }
}

impl SessionConfig {
    /// Whether the session cookie must be issued as `Secure`.
    ///
    /// `true` when this process terminates TLS itself (`tls_configured`) **or**
    /// when the deployment is publicly reachable over `https://`
    /// (`external_url`): behind a reverse proxy the listener speaks plain HTTP
    /// while the browser sees an HTTPS origin, and a cookie without `Secure`
    /// would then travel over an insecure transport. Deciding on
    /// `tls.is_some()` alone left exactly that deployment insecurely marked.
    ///
    /// The scheme comparison ignores ASCII case, because URL schemes are
    /// case-insensitive; leading whitespace is tolerated for a hand-edited
    /// config value.
    #[must_use]
    pub fn detect_secure(tls_configured: bool, external_url: Option<&str>) -> bool {
        tls_configured
            || external_url.is_some_and(|url| {
                url.trim_start()
                    .get(.."https://".len())
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
            })
    }

    /// Derive the cookie name and `Secure` attribute from the TLS decision.
    ///
    /// `true` selects the `__Host-` form (which the caller must not combine
    /// with a domain; [`SessionConfig::with_cookie_domain`] refuses it).
    #[must_use]
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure_cookies = secure;
        self.cookie_name = if secure {
            SECURE_COOKIE_NAME.to_string()
        } else {
            INSECURE_COOKIE_NAME.to_string()
        };
        self
    }

    /// Set an explicit cookie domain, forcing the plain cookie name.
    ///
    /// A `__Host-` cookie cannot carry a `Domain`, so a deployment that needs
    /// one gets the plain name; the caller still controls `Secure`.
    #[must_use]
    pub fn with_cookie_domain(mut self, domain: impl Into<String>) -> Self {
        let domain = domain.into();
        self.cookie_domain = Some(domain);
        if self.cookie_name == SECURE_COOKIE_NAME {
            self.cookie_name = INSECURE_COOKIE_NAME.to_string();
        }
        self
    }

    /// Set the absolute session lifetime.
    #[must_use]
    pub const fn with_timeout_minutes(mut self, minutes: u64) -> Self {
        self.timeout_minutes = minutes;
        self
    }

    /// Set the sliding idle window.
    #[must_use]
    pub const fn with_idle_timeout_minutes(mut self, minutes: u64) -> Self {
        self.idle_timeout_minutes = minutes;
        self
    }

    /// Set the concurrent-session cap (`0` disables it).
    #[must_use]
    pub const fn with_max_sessions_per_user(mut self, max: u32) -> Self {
        self.max_sessions_per_user = max;
        self
    }

    /// The `Set-Cookie` value that removes the session cookie.
    ///
    /// Built as a literal instead of through a `cookie` crate value so it can
    /// never drift from the attributes `tower-sessions` writes: it names the
    /// same cookie, the same path and the same `Secure`/`Domain` decision, with
    /// `Max-Age=0`.
    #[must_use]
    pub fn clear_cookie(&self) -> String {
        let mut value = format!(
            "{}=\"\"; Path={}; Max-Age=0; HttpOnly; SameSite=Lax",
            self.cookie_name, self.cookie_path
        );
        if self.secure_cookies {
            value.push_str("; Secure");
        }
        if let Some(domain) = &self.cookie_domain {
            value.push_str("; Domain=");
            value.push_str(domain);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secure_configuration_uses_the_host_prefix() {
        let config = SessionConfig::default().with_secure(true);
        assert_eq!(config.cookie_name, SECURE_COOKIE_NAME);
        assert!(config.clear_cookie().contains("; Secure"));
    }

    #[test]
    fn a_plain_http_configuration_uses_the_plain_name() {
        let config = SessionConfig::default().with_secure(false);
        assert_eq!(config.cookie_name, INSECURE_COOKIE_NAME);
        assert!(!config.clear_cookie().contains("; Secure"));
    }

    #[test]
    fn a_domain_never_keeps_the_host_prefix() {
        let config = SessionConfig::default()
            .with_secure(true)
            .with_cookie_domain("example.com");
        assert_eq!(config.cookie_name, INSECURE_COOKIE_NAME);
        assert!(config.clear_cookie().contains("; Domain=example.com"));
    }

    #[test]
    fn clearing_matches_the_name_path_and_security() {
        let config = SessionConfig::default().with_secure(true);
        let header = config.clear_cookie();
        assert!(header.starts_with(SECURE_COOKIE_NAME));
        assert!(header.contains("Path=/"));
        assert!(header.contains("Max-Age=0"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Lax"));
    }

    /// The `Secure` decision is TLS **or** an HTTPS public origin: a proxy that
    /// terminates TLS forwards plain HTTP, so `tls.is_some()` cannot be the
    /// only signal.
    #[test]
    fn secure_is_detected_from_tls_or_an_https_public_url() {
        assert!(
            SessionConfig::detect_secure(true, None),
            "in-process TLS always marks the cookie Secure"
        );
        assert!(
            SessionConfig::detect_secure(false, Some("https://erp.example.com")),
            "an HTTPS public origin behind a proxy marks the cookie Secure"
        );
        assert!(
            !SessionConfig::detect_secure(false, Some("http://localhost:8000")),
            "plain HTTP in dev must not set Secure"
        );
        assert!(
            !SessionConfig::detect_secure(false, None),
            "no TLS and no public URL is the plain dev profile"
        );
        assert!(
            SessionConfig::detect_secure(false, Some(" HTTPS://erp.example.com ")),
            "the scheme is case-insensitive and tolerates surrounding whitespace"
        );
    }
}
