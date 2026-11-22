//! Client error reporting: `POST /api/log/client`.
//!
//! The Desk's form UI has no console the operator can read: an error that only
//! reaches the browser console disappears when the user closes the tab, and a
//! save that failed looks the same as one that is merely slow. This endpoint
//! turns that feedback into a server-side log line, under one stable target
//! (`ferris_client`) so a deployment can filter exactly these reports out of
//! the rest of the server's output.
//!
//! Two limits are part of what a caller may rely on:
//!
//! - **It writes nothing to a database.** This is a log sink, not an audit
//!   trail; inventing stored rows here would make every render retry a write
//!   and would put unvalidated client text in the document store.
//! - **It echoes nothing back.** The response says only that the report was
//!   accepted, so the endpoint cannot be used as a reflection surface.
//!
//! The payload is bounded and flattened before it reaches `tracing`: a client
//! is untrusted input, and an unbounded or multi-line message would let one
//! request flood the log, forge a second entry, or drive a terminal when an
//! operator tails it.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_core::{Error, Result, ValidationError};
use serde::Deserialize;

/// The one `tracing` target every client report is emitted under.
///
/// A filter on `target=ferris_client` selects these lines without matching the
/// module path, and the path can move without breaking an operator's filter.
pub const CLIENT_LOG_TARGET: &str = "ferris_client";

/// Longest message kept, in bytes.
const MAX_MESSAGE_BYTES: usize = 2 * 1024;

/// Longest context kept, in bytes.
const MAX_CONTEXT_BYTES: usize = 4 * 1024;

/// Appended to a clipped field so the log shows that text was dropped.
const TRUNCATION_MARKER: &str = "…[truncated]";

/// The body of `POST /api/log/client`.
#[derive(Debug, Deserialize)]
pub struct ClientLogRequest {
    /// `"warn"` or `"error"`; anything else is a validation error.
    pub level: String,
    /// What went wrong, as the client saw it.
    pub message: String,
    /// Extra detail (state, ids, a stack trace). Optional and bounded.
    #[serde(default)]
    pub context: Option<String>,
}

/// The severity a client report is allowed to claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientLogLevel {
    Warn,
    Error,
}

impl ClientLogLevel {
    /// Parse the wire spelling of a level.
    ///
    /// Only `warn` and `error` are accepted: a client report is about a
    /// *problem*, and letting it claim `info`/`debug` would let a noisy build
    /// hide its own failures at a level the operator filters out.
    ///
    /// # Errors
    ///
    /// A validation error for any other spelling. The message does not repeat
    /// the supplied text: it is caller-controlled and the reply must not become
    /// a reflection surface.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(Error::Validation(ValidationError::Invalid(
                "client log level must be `warn` or `error`".to_string(),
            ))),
        }
    }
}

/// `POST /api/log/client` — accept one client-side warning or error.
///
/// The session is the callback address: a report is attributed to the user who
/// sent it, and an anonymous caller is refused by the `AuthenticatedUser`
/// extractor before the body is read. Nothing here needs the API state, so the
/// handler takes no `State`: a log line has no database to open.
pub async fn client_report(
    user: AuthenticatedUser,
    Json(request): Json<ClientLogRequest>,
) -> Response {
    let level = match ClientLogLevel::parse(&request.level) {
        Ok(level) => level,
        Err(e) => return crate::router::error_response(e),
    };

    let message = clip(&single_line(&request.message), MAX_MESSAGE_BYTES);
    if message.is_empty() {
        return crate::router::error_response(Error::Validation(ValidationError::Invalid(
            "a client log entry needs a message".to_string(),
        )));
    }
    let context = request
        .context
        .as_deref()
        .map(|raw| clip(&single_line(raw), MAX_CONTEXT_BYTES))
        .unwrap_or_default();

    // The identity travels into the log line, never into the response: it is
    // what makes a report attributable, and the caller already knows who they
    // are.
    let user_id = user.0.id;
    match level {
        ClientLogLevel::Warn => tracing::warn!(
            target: CLIENT_LOG_TARGET,
            user = %user_id,
            context = %context,
            message = %message
        ),
        ClientLogLevel::Error => tracing::error!(
            target: CLIENT_LOG_TARGET,
            user = %user_id,
            context = %context,
            message = %message
        ),
    }

    StatusCode::NO_CONTENT.into_response()
}

/// Flatten a client string to one printable line.
///
/// The text is untrusted and ends up in a log file: a newline would let a
/// client forge a second entry, and a terminal escape sequence would run when
/// an operator tails the log. Every control character becomes a single space.
fn single_line(raw: &str) -> String {
    raw.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Cut `text` to at most `max_bytes`, on a character boundary, and mark the cut.
///
/// The marker is counted inside the budget, so the result never exceeds the
/// documented limit whatever the client sent, and a multi-byte character is
/// never split (a torn UTF-8 sequence would be lost by the log sink anyway and
/// would make the field unreadable).
fn clip(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let budget = max_bytes.saturating_sub(TRUNCATION_MARKER.len());
    let mut end = budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + TRUNCATION_MARKER.len());
    out.push_str(&text[..end]);
    out.push_str(TRUNCATION_MARKER);
    out
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    /// A level the client is not allowed to claim is a validation error, and
    /// the router answers it with `400`: a `500` would blame the server for a
    /// malformed report.
    #[test]
    fn an_unknown_level_is_a_validation_error_mapped_to_400() {
        for level in ["debug", "trace", "", "warning", "ERROR!"] {
            let err = ClientLogLevel::parse(level).expect_err("an unknown level is refused");
            assert!(matches!(
                err,
                Error::Validation(ValidationError::Invalid(_))
            ));
            let response = crate::router::error_response(err);
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        assert_eq!(
            ClientLogLevel::parse("warn").ok(),
            Some(ClientLogLevel::Warn)
        );
        assert_eq!(
            ClientLogLevel::parse("WARN").ok(),
            Some(ClientLogLevel::Warn)
        );
        assert_eq!(
            ClientLogLevel::parse(" ERROR ").ok(),
            Some(ClientLogLevel::Error)
        );
    }

    /// The context is optional, so a minimal report is a valid body.
    #[test]
    fn context_is_optional() {
        let request: ClientLogRequest = serde_json::from_value(serde_json::json!({
            "level": "error",
            "message": "save failed"
        }))
        .expect("a report without context parses");
        assert_eq!(request.level, "error");
        assert!(request.context.is_none());
    }

    /// A long payload is bounded and cut on a character boundary.
    #[test]
    fn a_long_payload_is_clipped_without_splitting_a_character() {
        let long = "é".repeat(MAX_MESSAGE_BYTES);
        let clipped = clip(&long, MAX_MESSAGE_BYTES);
        assert!(clipped.len() <= MAX_MESSAGE_BYTES);
        assert!(clipped.ends_with(TRUNCATION_MARKER));
        assert_eq!(clip("short", MAX_MESSAGE_BYTES), "short");
    }

    /// Control characters would forge log entries or drive a terminal, so they
    /// become one line before they reach `tracing`.
    #[test]
    fn control_characters_collapse_to_one_line() {
        assert_eq!(single_line("a\nb\u{1b}[31m"), "a b [31m");
        assert_eq!(single_line("  padded  "), "padded");
    }
}
