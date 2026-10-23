//! Error values and the retry classification attached to HTTP responses.

use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};

/// A response error that deliberately keeps structured information instead of
/// asking callers to parse a display string.
#[derive(Debug, thiserror::Error)]
#[error("HTTP {status} {kind}: {message}")]
pub(crate) struct HttpError {
    status: u16,
    kind: String,
    message: String,
}

impl HttpError {
    pub(crate) fn new(status: u16, kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// Classifies an HTTP failure without retrying it.
#[must_use]
pub(crate) const fn classify_status(status: u16, intent: StatementIntent) -> ErrorClass {
    match status {
        429 | 503 if !intent.is_mutating() => ErrorClass::Transient,
        500..=599 if intent.is_mutating() => ErrorClass::Ambiguous,
        408 | 429 | 502 | 503 | 504 if intent.is_mutating() => ErrorClass::Ambiguous,
        408 | 429 | 502 | 503 | 504 => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

/// Wraps a sanitized HTTP error in the shared error envelope.
#[must_use]
pub(crate) fn response_error(
    status: u16,
    kind: impl Into<String>,
    message: impl Into<String>,
    intent: StatementIntent,
) -> OxynError {
    OxynError::driver(
        DriverId::elasticsearch(),
        classify_status(status, intent),
        HttpError::new(status, kind, message),
    )
}

/// A lost response after a request may have reached the server.
#[must_use]
pub(crate) fn transport_error(intent: StatementIntent, message: impl Into<String>) -> OxynError {
    let class = if intent.is_mutating() {
        ErrorClass::Ambiguous
    } else {
        ErrorClass::Transient
    };
    OxynError::driver(
        DriverId::elasticsearch(),
        class,
        HttpError::new(0, "transport", message),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_timeout_or_gateway_failure_is_ambiguous() {
        assert_eq!(
            classify_status(504, StatementIntent::Write),
            ErrorClass::Ambiguous
        );
        assert_eq!(
            classify_status(503, StatementIntent::Unknown),
            ErrorClass::Ambiguous
        );
        assert_eq!(
            classify_status(503, StatementIntent::Read),
            ErrorClass::Transient
        );
    }

    #[test]
    fn a_version_conflict_is_permanent() {
        assert_eq!(
            classify_status(409, StatementIntent::Write),
            ErrorClass::Permanent
        );
    }
}
