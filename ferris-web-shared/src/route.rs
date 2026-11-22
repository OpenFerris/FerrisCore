//! Route helpers shared by the Desk and the metadata layer.
//!
//! The Desk addresses documents by path segment (`/desk/:module/:doctype/:name`)
//! and doctype and document names are *data*: they may contain spaces, `#`, `?`
//! or a slash. Every route built from metadata goes through
//! [`path_segment`], so "Sales Invoice" or a document named `a/b` cannot turn
//! into a different route — or a second path segment — by accident.

#![forbid(unsafe_code)]

use std::fmt::Write as _;

/// Percent-encode one path segment.
///
/// RFC 3986 unreserved characters (`A-Z a-z 0-9 - _ . ~`) pass through; every
/// other byte is `%XX`. Non-ASCII names are encoded byte by byte, which is what
/// a browser decodes back to the original UTF-8.
///
/// # Examples
///
/// ```
/// use ferris_web_shared::route::path_segment;
///
/// assert_eq!(path_segment("Sales Invoice"), "Sales%20Invoice");
/// assert_eq!(path_segment("TASK-0001"), "TASK-0001");
/// assert_eq!(path_segment("ada@example.com"), "ada%40example.com");
/// ```
#[must_use]
pub fn path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                // Writing into the existing buffer avoids one allocation per
                // escaped byte of every encoded segment.
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Decode the `%XX` escapes of one path segment.
///
/// Every other byte passes through, and an incomplete or invalid escape is kept
/// verbatim rather than dropped, so a literal `%` never swallows the text after
/// it. The inverse of [`path_segment`]; the Desk uses it to turn the segment it
/// received back into the label a user should read.
///
/// # Examples
///
/// ```
/// use ferris_web_shared::route::percent_decode;
///
/// assert_eq!(percent_decode("Sales%20Invoice"), "Sales Invoice");
/// assert_eq!(percent_decode("100%"), "100%");
/// assert_eq!(percent_decode("%C3%B1"), "ñ");
/// ```
#[must_use]
pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = match (hex_val(bytes.get(index + 1)), hex_val(bytes.get(index + 2))) {
            (Some(high), Some(low)) if bytes[index] == b'%' => Some((high << 4) | low),
            _ => None,
        };
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of one hexadecimal digit, or `None` for anything else.
fn hex_val(byte: Option<&u8>) -> Option<u8> {
    match byte.copied()? {
        digit @ b'0'..=b'9' => Some(digit - b'0'),
        digit @ b'a'..=b'f' => Some(digit - b'a' + 10),
        digit @ b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{path_segment, percent_decode};

    #[test]
    fn decoding_reverses_encoding_and_keeps_bad_escapes() {
        assert_eq!(percent_decode("Sales%20Invoice"), "Sales Invoice");
        assert_eq!(percent_decode("ada%40example.com"), "ada@example.com");
        assert_eq!(percent_decode("a%2Fb%3Fc%23d"), "a/b?c#d");
        assert_eq!(percent_decode("%C3%B1"), "ñ");
        // An incomplete escape is text, not a decoder error.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn unreserved_characters_are_kept() {
        assert_eq!(path_segment("TASK-0001_a.b~c"), "TASK-0001_a.b~c");
    }

    #[test]
    fn separators_and_reserved_characters_are_escaped() {
        assert_eq!(path_segment("Sales Invoice"), "Sales%20Invoice");
        assert_eq!(path_segment("a/b?c#d"), "a%2Fb%3Fc%23d");
        assert_eq!(path_segment("ada@example.com"), "ada%40example.com");
    }

    #[test]
    fn non_ascii_is_encoded_per_utf8_byte() {
        assert_eq!(path_segment("ñ"), "%C3%B1");
    }
}
