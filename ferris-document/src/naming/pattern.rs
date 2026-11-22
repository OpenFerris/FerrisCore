//! Pattern formatters, date tokens and the naming-provider registry.

use crate::engine::EngineContext;
use chrono::{DateTime, Utc};
use ferris_core::{Result, Value};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

pub fn format_series(series: &str, counter: u64) -> String {
    {
        let mut result = String::new();
        let mut chars = series.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '.' {
                // Check for special patterns
                if let Some(&next) = chars.peek() {
                    match next {
                        '#' => {
                            // Count consecutive # characters
                            let mut hash_count = 0;
                            while chars.next() == Some('#') {
                                hash_count += 1;
                            }
                            // Format counter with leading zeros
                            let width = hash_count.max(1);
                            result.push_str(&format!("{:0width$}", counter, width = width));
                        }
                        'Y' => {
                            // Check for YY or YYYY
                            let mut year_format = String::new();
                            year_format.push(chars.next().unwrap()); // consume Y
                            while chars.peek() == Some(&'Y') {
                                year_format.push(chars.next().unwrap());
                            }
                            let now = Utc::now();
                            match year_format.len() {
                                2 => result.push_str(&now.format("%y").to_string()),
                                4 => result.push_str(&now.format("%Y").to_string()),
                                _ => result.push_str(&now.format("%Y").to_string()),
                            }
                        }
                        'M' => {
                            // MM and M format the same way, so consume the
                            // run without branching on it.
                            chars.next();
                            let now = Utc::now();
                            result.push_str(&now.format("%m").to_string());
                        }
                        'D' => {
                            // DD and D format the same way, so consume the
                            // run without branching on it.
                            chars.next();
                            let now = Utc::now();
                            result.push_str(&now.format("%d").to_string());
                        }
                        _ => {
                            result.push('.');
                        }
                    }
                } else {
                    result.push('.');
                }
            } else {
                result.push(c);
            }
        }

        result
    }
}

/// Render one date/time token of a naming pattern.
///
/// The token set mirrors Frappe's `naming_series` placeholders
/// (`YYYY`, `YY`, `MM`, `DD`, `WW`, `HH`, `mm`, `ss`) plus `timestamp` for the
/// Unix epoch. `WW` uses `strftime("%W")` (Monday-first week number) because
/// that is what Frappe emits; `MM` is the month and `mm` the minute, so the two
/// cannot be confused by case-folding the pattern first.
///
/// An unknown token returns `None`, which lets the expression formatters treat
/// it as a literal or a field reference instead of guessing.
#[must_use]
pub(crate) fn date_token(token: &str, now: DateTime<Utc>) -> Option<String> {
    let format = match token {
        "YYYY" => "%Y",
        "YY" => "%y",
        "MM" => "%m",
        "DD" => "%d",
        "WW" => "%W",
        "HH" => "%H",
        "mm" => "%M",
        "ss" => "%S",
        "timestamp" => return Some(now.timestamp().to_string()),
        _ => return None,
    };
    Some(now.format(format).to_string())
}

/// A registered naming strategy for [`NamingType::Script`].
///
/// Frappe runs a Python `autoname` method; Ferris has no scripting sandbox, so a
/// plugin registers a Rust function instead. The document is the value about to
/// be inserted (after the before-naming hooks), and the context carries the
/// DocType and the acting user. Re-registering a DocType replaces the provider,
/// which keeps a dev-server reload idempotent.
pub type NamingProvider = fn(&Value, &EngineContext<'_>) -> Result<String>;

static NAMING_PROVIDERS: OnceLock<RwLock<HashMap<String, NamingProvider>>> = OnceLock::new();

fn providers() -> &'static RwLock<HashMap<String, NamingProvider>> {
    NAMING_PROVIDERS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register the provider that mints names for a DocType declaring
/// [`NamingType::Script`].
pub fn register_naming_provider(doctype: &str, provider: NamingProvider) {
    providers()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(doctype.to_string(), provider);
}

/// The provider registered for `doctype`, if any.
pub(crate) fn naming_provider(doctype: &str) -> Option<NamingProvider> {
    providers()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(doctype)
        .copied()
}

/// The text a naming pattern substitutes for a document value.
///
/// A naming pattern is a string; a non-string field (Int, Check, …) is rendered
/// without JSON quotes so `{priority}` in a pattern produces `3`, not `"3"`.
pub(crate) fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Render a date token, allowing the surrounding hyphens of a `.`-separated
/// part to stay literal (`MM-` in the documented `EXAMPLE-.MM.-…` example is
/// the month followed by a hyphen, not a token named `MM-`).
fn dated_part(part: &str, now: DateTime<Utc>) -> Option<String> {
    let core = part.trim_matches('-');
    if core.is_empty() || core == part {
        return date_token(part, now);
    }
    let value = date_token(core, now)?;
    let leading = part.len() - part.trim_start_matches('-').len();
    let trailing = part.len() - part.trim_end_matches('-').len();
    Some(format!(
        "{}{}{}",
        "-".repeat(leading),
        value,
        "-".repeat(trailing)
    ))
}

/// Replace every brace token in `part`, leaving literal text untouched.
///
/// `{#####}` is the counter (rendered once per pattern), `{YYYY}`/`{MM}`/… are
/// date tokens and `{fieldname}` is a document value. An unknown token is kept
/// verbatim instead of being silently dropped, so a typo in a pattern is
/// visible in the generated name.
fn render_braces(
    part: &str,
    fields: &HashMap<String, Value>,
    counter: u64,
    counter_used: &mut bool,
    now: DateTime<Utc>,
) -> String {
    let mut rendered = String::with_capacity(part.len());
    let mut rest = part;
    while let Some(open) = rest.find('{') {
        rendered.push_str(&rest[..open]);
        let Some(close) = rest[open + 1..].find('}') else {
            // An unterminated brace is literal text; returning it verbatim is
            // the only outcome that does not invent a value.
            rendered.push_str(&rest[open..]);
            return rendered;
        };
        let close = open + 1 + close;
        let token = &rest[open + 1..close];
        if token.chars().all(|c| c == '#') {
            if !*counter_used {
                rendered.push_str(&format!("{:0width$}", counter, width = token.len()));
                *counter_used = true;
            }
        } else if let Some(value) = date_token(token, now) {
            rendered.push_str(&value);
        } else if let Some(value) = fields.get(token) {
            rendered.push_str(&text(value));
        } else {
            rendered.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    rendered.push_str(rest);
    rendered
}

/// Format a **Expression** pattern: parts separated by `.`, concatenated
/// without a separator (Frappe's `parse_naming_series` appends each part
/// directly, which is why the documented example carries its own hyphens).
///
/// Supports `#` runs (counter), date tokens, `{fieldname}` and bare field names.
pub(crate) fn format_expression(
    pattern: &str,
    fields: &HashMap<String, Value>,
    counter: u64,
) -> String {
    let now = Utc::now();
    let mut counter_used = false;
    let mut rendered = String::new();
    for part in pattern.split('.') {
        if part.is_empty() {
            continue;
        }
        if part.chars().all(|c| c == '#') {
            if !counter_used {
                rendered.push_str(&format!("{:0width$}", counter, width = part.len()));
                counter_used = true;
            }
        } else if let Some(date) = dated_part(part, now) {
            rendered.push_str(&date);
        } else if let Some(value) = fields.get(part) {
            rendered.push_str(&text(value));
        } else {
            rendered.push_str(&render_braces(
                part,
                fields,
                counter,
                &mut counter_used,
                now,
            ));
        }
    }
    rendered
}

/// Format an **Expression (old style)** pattern: one literal string whose
/// `{fieldname}`, `{MM}` and `{#####}` tokens are replaced in place.
pub(crate) fn format_expression_old_style(
    pattern: &str,
    fields: &HashMap<String, Value>,
    counter: u64,
) -> String {
    let mut counter_used = false;
    render_braces(pattern, fields, counter, &mut counter_used, Utc::now())
}

/// The 10-character random name Frappe's `Random` method mints.
///
/// Frappe uses `base64.b32hexencode(token_bytes(10))[:10].lower()`; the alphabet
/// is `0-9a-v` (base32hex, no padding). The same alphabet is reproduced here
/// over the 16 random bytes of a v4 UUID, which keeps the name lowercase and
/// database-collation safe without a second random-number dependency.
pub(crate) fn random_name() -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let mut out = String::with_capacity(10);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 && out.len() < 10 {
            bits -= 5;
            // `idx` is masked to five bits, so it is always < 32.
            let index = usize::try_from((buffer >> bits) & 0x1f).unwrap_or_default();
            out.push(char::from(ALPHABET[index]));
        }
        if out.len() == 10 {
            break;
        }
    }
    out
}
