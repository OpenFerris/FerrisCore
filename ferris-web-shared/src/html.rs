//! HTML text escaping and sanitizing, shared by every producer of markup.
//!
//! Four modules in `ferris-web` interpolate user or metadata text into markup:
//! the website page renderer, the Markdown renderer, the rich-text editor's
//! bridge and the widget-preview frame. Each used to carry its own copy of the
//! same four `replace` calls, and a copy that drifts is a cross-site-scripting
//! bug in exactly the place it is easiest to miss. The functions are pure and
//! platform-neutral, so they live in the leaf crate both the server and the
//! browser compile against.
//!
//! [`escape_html`] is the *text* answer: it assumes the caller interpolates a
//! value into markup and wants no markup at all. [`sanitize_html`] is the
//! answer for a value that is already HTML — a `Text Editor` value stored as
//! rich text — where the caller wants the safe subset kept and everything else
//! removed before it reaches `inner_html`.

/// Escape `&`, `<`, `>` and `"` so `value` can be interpolated into element
/// text or a double-quoted attribute.
///
/// `&` is replaced first, otherwise the ampersands of the escapes would be
/// escaped again. Single quotes are *not* escaped: a caller that interpolates
/// into a single-quoted attribute needs an XML-flavoured escaper instead, and
/// this repository never emits one.
///
/// # Examples
///
/// ```
/// use ferris_web_shared::html::escape_html;
///
/// assert_eq!(escape_html("plain & safe"), "plain &amp; safe");
/// assert_eq!(
///     escape_html(r#"<img src=x onerror="pwn()">"#),
///     "&lt;img src=x onerror=&quot;pwn()&quot;&gt;"
/// );
/// ```
#[must_use]
pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Elements the sanitizer keeps. Everything else is unwrapped: its text stays,
/// its tag and attributes do not.
const ALLOWED_TAGS: &[&str] = &[
    "a",
    "blockquote",
    "br",
    "code",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ol",
    "p",
    "pre",
    "strong",
    "ul",
];

/// Elements dropped **with their content**: their body is script, CSS or an
/// embedded document, never text a reader should see.
const DROPPED_TAGS: &[&str] = &["script", "style", "iframe", "object"];

/// Keep the safe subset of `value` as HTML.
///
/// A `Text Editor` value is stored as HTML and the Desk paints it with
/// `inner_html`, so a document body can otherwise smuggle `<script>` or
/// `<img onerror>` into the admin's page — the CSP is a second line of
/// defence, not the place to rely on (CMP-10). The allowlist is deliberate and
/// small: block text, lists, headings, blockquote, inline code and links.
///
/// * Tags outside [`ALLOWED_TAGS`] are unwrapped (or, for [`DROPPED_TAGS`],
///   removed with their content).
/// * Only `href` survives, and only for `http`, `https` and `mailto`; a
///   `javascript:`/`data:` target leaves the anchor without a target.
/// * Every `on*` handler and every `style` attribute disappears because no
///   attribute other than `href` is ever emitted.
///
/// Text is passed through as it was written, so an entity the author typed
/// (`&amp;`) still renders as the character it names.
///
/// This is a sanitizer for *fragments*, not a full HTML parser: it does not
/// balance tags, and a fragment truncated mid-tag loses the tail. Both are
/// fine for a stored editor value, which is what it is used on.
///
/// # Examples
///
/// ```
/// use ferris_web_shared::html::sanitize_html;
///
/// assert_eq!(sanitize_html("<p>Hola <strong>mundo</strong></p>"), "<p>Hola <strong>mundo</strong></p>");
/// assert_eq!(sanitize_html(r#"<p onclick="pwn()">x</p>"#), "<p>x</p>");
/// assert_eq!(sanitize_html("<script>alert(1)</script>"), "");
/// assert_eq!(
///     sanitize_html(r#"<a href="javascript:alert(1)">x</a>"#),
///     "<a>x</a>"
/// );
/// ```
#[must_use]
pub fn sanitize_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    // The element whose content is currently being skipped, if any.
    let mut dropped: Option<&'static str> = None;
    let mut depth = 0usize;

    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if dropped.is_none() {
                out.push_str(rest);
            }
            break;
        };
        if dropped.is_none() {
            out.push_str(&rest[..open]);
        }
        let after = &rest[open..];
        // An unterminated tag is not something a browser would parse either;
        // dropping the tail keeps a truncated `<script` from leaking.
        let Some(close) = after.find('>') else {
            break;
        };
        let raw = &after[1..close];
        rest = &after[close + 1..];

        if raw.starts_with('!') {
            // Comments, doctypes and CDATA: never text, never markup we keep.
            if raw.starts_with("!--")
                && let Some(end) = rest.find("-->")
            {
                rest = &rest[end + 3..];
            }
            continue;
        }

        let trimmed = raw.trim();
        let closing = trimmed.starts_with('/');
        let body = trimmed.trim_start_matches('/').trim();
        let name_end = body
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(body.len());
        let name = body[..name_end].to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }

        if let Some(target) = DROPPED_TAGS.iter().copied().find(|tag| *tag == name) {
            if closing {
                if dropped == Some(target) {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        dropped = None;
                    }
                }
            } else if dropped == Some(target) {
                depth += 1;
            } else if dropped.is_none() {
                dropped = Some(target);
                depth = 1;
            }
            continue;
        }
        // Inside a dropped element nothing else is markup we look at.
        if dropped.is_some() {
            continue;
        }
        if !ALLOWED_TAGS.contains(&name.as_str()) {
            continue;
        }
        if closing {
            // `br` is void: a stray closing tag would be fixed up by the
            // browser into something we did not emit.
            if name != "br" {
                out.push_str("</");
                out.push_str(&name);
                out.push('>');
            }
            continue;
        }
        out.push('<');
        out.push_str(&name);
        if name == "a"
            && let Some(href) = href_attribute(body)
        {
            out.push_str(" href=\"");
            out.push_str(&escape_html(href));
            out.push('"');
        }
        out.push('>');
    }

    out
}

/// The `href` of an anchor's attribute list, when it is a permitted scheme.
///
/// The whole attribute list is parsed but only `href` is ever returned, which
/// is what removes `on*` handlers and `style` without naming them.
fn href_attribute(body: &str) -> Option<&str> {
    let name_end = body
        .find(|c: char| c.is_ascii_whitespace() || c == '/')
        .unwrap_or(body.len());
    let mut rest = &body[name_end..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let attribute = rest[..name_end].to_ascii_lowercase();
        rest = &rest[name_end..];
        let tail = rest.trim_start();
        let value = tail.strip_prefix('=').map_or("", |after_equals| {
            attribute_value(&mut rest, after_equals.trim_start())
        });
        if attribute == "href" {
            return safe_url(value);
        }
    }
}

/// Read one attribute value from `after_equals`, advancing `rest` past it.
///
/// A value may be double-quoted, single-quoted or bare. An unterminated quote
/// consumes the rest of the tag: nothing after it can be trusted to be an
/// attribute, and leaving the cursor in place would rescan the same bytes.
fn attribute_value<'a>(rest: &mut &'a str, after_equals: &'a str) -> &'a str {
    if let Some(quote) = after_equals
        .chars()
        .next()
        .filter(|quote| *quote == '"' || *quote == '\'')
    {
        let inner = &after_equals[1..];
        let end = inner.find(quote);
        *rest = end.map_or("", |end| &inner[end + 1..]);
        end.map_or("", |end| &inner[..end])
    } else {
        let end = after_equals
            .find(char::is_whitespace)
            .unwrap_or(after_equals.len());
        *rest = &after_equals[end..];
        &after_equals[..end]
    }
}

/// A link target the Desk may emit: an absolute `http(s)` or `mailto` URL.
///
/// Relative targets are rejected on purpose — the editor value has no base to
/// resolve them against, and `/api/...` in a document body is a request the
/// reader did not ask for. That is stricter than [`crate::url::safe_href`],
/// which also allows relative references and `tel:` for views that interpolate
/// a link the host itself built.
fn safe_url(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let lowered = value.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lowered.starts_with(scheme))
        .then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{escape_html, sanitize_html};

    #[test]
    fn markup_and_quotes_are_escaped() {
        assert_eq!(
            escape_html(r#"<b class="x">&</b>"#),
            "&lt;b class=&quot;x&quot;&gt;&amp;&lt;/b&gt;"
        );
    }

    #[test]
    fn a_clean_string_is_returned_unchanged() {
        assert_eq!(escape_html("sin marcas"), "sin marcas");
        assert_eq!(escape_html(""), "");
    }

    #[test]
    fn escaping_is_idempotent_only_per_pass() {
        // The ampersand of the first escape must survive the second pass
        // untouched as text, which is what ordering `&` first guarantees.
        assert_eq!(escape_html("&lt;"), "&amp;lt;");
    }

    /// CMP-10: the allowlist keeps block text and drops every way to run code.
    #[test]
    fn a_hostile_fragment_loses_every_executable_part() {
        // `<img onerror>` is not an allowed element: the tag disappears.
        assert_eq!(
            sanitize_html(r#"<img src=x onerror="pwn()">visible"#),
            "visible"
        );
        // A script is dropped **with its body**, not unwrapped into text.
        assert_eq!(sanitize_html("<script>alert(1)</script>ok"), "ok");
        assert_eq!(sanitize_html("<style>body{}</style>x"), "x");
        assert_eq!(sanitize_html("<iframe src=//evil>x</iframe>"), "");
        assert_eq!(sanitize_html("<object data=x></object>"), "");
        // A handler on an allowed element is dropped, the element stays.
        assert_eq!(
            sanitize_html(r#"<p onclick="pwn()" style="color:red">x</p>"#),
            "<p>x</p>"
        );
        // A `javascript:` target leaves the anchor without one.
        assert_eq!(
            sanitize_html(r#"<a href="javascript:alert(1)">x</a>"#),
            "<a>x</a>"
        );
        assert_eq!(
            sanitize_html(r#"<a href="JaVaScRiPt:alert(1)">x</a>"#),
            "<a>x</a>"
        );
        // An unterminated tag cannot smuggle the rest through as text.
        assert_eq!(sanitize_html("<script alert(1)"), "");
    }

    /// The subset the editor actually produces survives untouched, with the
    /// link target escaped for the attribute it lands in.
    #[test]
    fn the_safe_subset_round_trips() {
        let body = "<h2>Título</h2><p>Hola <strong>mundo</strong> y <em>todos</em></p>\
                    <ul><li>uno</li><li>dos</li></ul><blockquote>cita</blockquote>\
                    <pre><code>let x = 1;</code></pre><br>";
        assert_eq!(sanitize_html(body), body);
        assert_eq!(
            sanitize_html(r#"<a href="https://example.test/?a=1&b=2">enlace</a>"#),
            r#"<a href="https://example.test/?a=1&amp;b=2">enlace</a>"#
        );
        assert_eq!(
            sanitize_html(r#"<a href="mailto:ada@example.test">escríbeme</a>"#),
            r#"<a href="mailto:ada@example.test">escríbeme</a>"#
        );
        // Free text (entities included) is not escaped twice: it is text.
        assert_eq!(
            sanitize_html("5 &lt; 7 &amp; 7 &gt; 5"),
            "5 &lt; 7 &amp; 7 &gt; 5"
        );
        assert_eq!(sanitize_html(""), "");
    }
}
