//! URL allow-listing for every `href`/`src` the Desk renders.
//!
//! Document and metadata values reach the views as data: a `help_link`, a
//! workspace URL, a field value the user typed. Interpolating one straight into
//! `href` turns `javascript:` into script execution on click, and `data:` into
//! a document that replaces the Desk. The browser has no "is this safe"
//! question to ask, so the check has to happen before the attribute exists.
//!
//! The policy is an allow-list, not a deny-list: relative references
//! (`/x`, `#x`, `?x`, `x`) and the four schemes the Desk actually uses
//! (`http`, `https`, `mailto`, `tel`). Everything else — including a scheme
//! nobody has thought of yet — is rejected. The check is pure so both the
//! browser and the SSR renderer apply the same rule.

/// A relative reference or one of the allowed schemes.
///
/// The returned value is the original text with the surrounding whitespace
/// removed, so a caller can use it as the attribute value as-is.
///
/// # Examples
///
/// ```
/// use ferris_web_shared::url::safe_href;
///
/// assert_eq!(safe_href("/desk/Task").as_deref(), Some("/desk/Task"));
/// assert_eq!(safe_href("https://example.com/a?b#c").as_deref(), Some("https://example.com/a?b#c"));
/// assert_eq!(safe_href("javascript:alert(1)"), None);
/// ```
#[must_use]
pub fn safe_href(value: &str) -> Option<String> {
    safe_url(value)
}

/// The same policy as [`safe_href`], for `src` attributes.
///
/// `mailto:`/`tel:` pass the shared allow-list even though an `<img>` cannot
/// use them: the check answers "may this URL be navigated or fetched", and the
/// element decides what it can do with a fetched resource.
#[must_use]
pub fn safe_src(value: &str) -> Option<String> {
    safe_url(value)
}

/// Shared scheme check.
///
/// Relative references have no scheme and are always allowed. A candidate is
/// only a scheme when a `:` appears before the first `/`, `?` or `#`, which is
/// exactly how a browser splits `scheme:path`.
fn safe_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Browsers strip ASCII tab/newline/CR while parsing a URL, so
    // `java\nscript:` still reaches the `javascript:` scheme. Control
    // characters anywhere are a smuggling signal; reject the value instead of
    // normalising it and hoping the browser agrees.
    if trimmed.chars().any(char::is_control) {
        return None;
    }

    // The first delimiter decides: after a `/`, `?` or `#` the colon is part
    // of the path/query/fragment, not a scheme.
    let delimiter = trimmed
        .char_indices()
        .find(|(_, ch)| matches!(ch, ':' | '/' | '?' | '#'));

    match delimiter {
        Some((index, ':')) => {
            let scheme = trimmed[..index].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto" | "tel")
                .then(|| trimmed.to_string())
        }
        _ => Some(trimmed.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{safe_href, safe_src};

    #[test]
    fn relative_references_are_allowed() {
        for value in [
            "/desk",
            "/desk/People/User",
            "#top",
            "?page=2",
            "about",
            "./a",
        ] {
            assert_eq!(safe_href(value).as_deref(), Some(value), "{value}");
        }
    }

    #[test]
    fn known_schemes_are_allowed_case_insensitively() {
        assert_eq!(
            safe_href("https://example.com/a?b#c").as_deref(),
            Some("https://example.com/a?b#c")
        );
        assert_eq!(
            safe_href("HTTP://example.com").as_deref(),
            Some("HTTP://example.com")
        );
        assert_eq!(
            safe_href("mailto:ada@example.com").as_deref(),
            Some("mailto:ada@example.com")
        );
        assert_eq!(
            safe_href("tel:+34123456789").as_deref(),
            Some("tel:+34123456789")
        );
        assert_eq!(
            safe_src("/static/logo.svg").as_deref(),
            Some("/static/logo.svg")
        );
    }

    #[test]
    fn javascript_and_data_are_rejected() {
        assert_eq!(safe_href("javascript:alert(1)"), None);
        assert_eq!(safe_href("JaVaScRiPt:alert(1)"), None);
        assert_eq!(safe_href("data:text/html,<script>alert(1)</script>"), None);
        assert_eq!(safe_src("data:image/svg+xml,<svg/>"), None);
        assert_eq!(safe_href("vbscript:msgbox(1)"), None);
    }

    #[test]
    fn unknown_schemes_are_rejected() {
        assert_eq!(safe_href("file:///etc/passwd"), None);
        assert_eq!(safe_href("blob:https://example.com/x"), None);
        assert_eq!(safe_href("custom-scheme:value"), None);
    }

    #[test]
    fn control_characters_cannot_smuggle_a_scheme() {
        // The browser removes these before parsing, which is why the raw text
        // must not pass the check.
        assert_eq!(safe_href("java\tscript:alert(1)"), None);
        assert_eq!(safe_href("java\nscript:alert(1)"), None);
        assert_eq!(safe_href(" javascript:alert(1)"), None);
    }

    #[test]
    fn a_colon_after_a_delimiter_is_not_a_scheme() {
        assert_eq!(safe_href("/a:b").as_deref(), Some("/a:b"));
        assert_eq!(safe_href("#a:b").as_deref(), Some("#a:b"));
        assert_eq!(safe_href("?a:b").as_deref(), Some("?a:b"));
        assert_eq!(safe_href("a/b:c").as_deref(), Some("a/b:c"));
    }

    #[test]
    fn empty_values_have_no_url() {
        assert_eq!(safe_href(""), None);
        assert_eq!(safe_href("   "), None);
        assert_eq!(safe_src(""), None);
    }
}
