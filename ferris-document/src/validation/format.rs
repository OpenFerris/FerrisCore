pub fn is_valid_email(email: &str) -> bool {
    if email.len() > 254 {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty() || local.len() > 64 || domain.is_empty() {
        return false;
    }
    // A dotless domain (`admin@localhost`, `ops@intranet`) is allowed on
    // purpose: Ferris ships to private installs, and the fixtures of the whole
    // workspace use `@local`. The structural checks below still reject the
    // malformed shapes.
    if domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    if domain.contains("..") {
        return false;
    }
    true
}

/// A plausible phone number: at least 10 digits and nothing but digits and the
/// usual separators, with an optional leading `+`.
///
/// DOC12: this used to strip *every* non-digit and only require ten of them, so
/// `"abcdefghij0123456789"` and `"0000000000"` passed — the function validated
/// nothing except the digit count. It now also rejects any character that is not
/// part of a phone number, which is still lenient enough for
/// `"+1 (555) 123-4567"` and Frappe's own stored formats.
pub fn is_valid_phone(phone: &str) -> bool {
    let mut digits = 0usize;
    for (i, c) in phone.chars().enumerate() {
        match c {
            '0'..='9' => digits += 1,
            // A `+` is only meaningful as the leading country-code marker.
            '+' if i == 0 => {}
            '-' | ' ' | '.' | '(' | ')' | '/' | '\u{a0}' => {}
            _ => return false,
        }
    }
    digits >= 10
}

pub fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}
