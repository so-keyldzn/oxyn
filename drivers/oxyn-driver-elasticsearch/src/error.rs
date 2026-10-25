//! The server's refusals, in its own terms, and their class as data.
//!
//! A professional reads `index_not_found_exception: no such index [logs]`,
//! not "the request failed": the message is the server's `error.type` and
//! `error.reason`, with the first cause it names. A reason can quote the
//! request it refuses — the user's own query, never a credential, which
//! travels only in the `Authorization` header. It is bounded, so that a
//! server quoting a whole document cannot fill a dialog.

use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};
use serde_json::Value;

/// The most of a server's reason a message keeps, in characters.
const REASON_CHARS: usize = 2_000;

/// A response error. What a caller decides on is the [`ErrorClass`] the
/// envelope carries; the message names the status and the server's
/// `error.type` for the person reading it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct HttpError(String);

/// Classifies an HTTP failure. Nothing here retries.
///
/// A rejection under load (`429`, `503`) or a gateway that gave up (`502`,
/// `504`) is transient for a read. For anything that may change data, every
/// one of them — and any `5xx` — is ambiguous: the cluster, or the node
/// behind the gateway, may have applied part or all of it (I-13).
#[must_use]
pub(crate) const fn classify_status(status: u16, intent: StatementIntent) -> ErrorClass {
    let mutating = intent.is_mutating();
    match status {
        408 | 429 | 500..=599 if mutating => ErrorClass::Ambiguous,
        408 | 429 | 502 | 503 | 504 => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

/// The error of a refused request, from its status and its body.
#[must_use]
pub(crate) fn response_error(status: u16, body: &[u8], intent: StatementIntent) -> OxynError {
    let error = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").cloned());
    let (kind, reason, cause) = match &error {
        Some(Value::Object(error)) => {
            let text = |key: &str| error.get(key).and_then(Value::as_str).map(str::to_owned);
            let cause = error
                .get("caused_by")
                .and_then(|cause| {
                    Some((cause.get("type")?.as_str()?, cause.get("reason")?.as_str()?))
                })
                .map(|(kind, reason)| format!("{kind}: {}", bounded(reason)));
            (
                text("type").unwrap_or_else(|| "error".to_owned()),
                text("reason"),
                cause,
            )
        }
        // Some proxies and old nodes answer `{"error": "text"}`.
        Some(Value::String(text)) => ("error".to_owned(), Some(text.clone()), None),
        _ => ("http".to_owned(), None, None),
    };
    let mut message = match reason {
        Some(reason) => format!("HTTP {status} {kind}: {}", bounded(&reason)),
        None => format!("HTTP {status}: the answer carries no Elasticsearch error"),
    };
    if let Some(cause) = cause {
        message.push_str(" (caused by ");
        message.push_str(&cause);
        message.push(')');
    }
    if status == 401 {
        return OxynError::Authentication(message);
    }
    OxynError::driver(
        DriverId::elasticsearch(),
        classify_status(status, intent),
        HttpError(message),
    )
}

/// A lost exchange: the request may or may not have reached the cluster.
#[must_use]
pub(crate) fn transport_error(intent: StatementIntent, message: &str) -> OxynError {
    let class = if intent.is_mutating() {
        ErrorClass::Ambiguous
    } else {
        ErrorClass::Transient
    };
    let message = if intent.is_mutating() {
        format!("{message}; the request may have been applied, in part or whole")
    } else {
        message.to_owned()
    };
    OxynError::driver(DriverId::elasticsearch(), class, HttpError(message))
}

/// At most [`REASON_CHARS`] characters, cut on a character boundary.
fn bounded(text: &str) -> String {
    match text.char_indices().nth(REASON_CHARS) {
        Some((end, _)) => format!("{}…", text.get(..end).unwrap_or_default()),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_timeout_or_gateway_failure_is_ambiguous_and_a_read_one_transient() {
        for status in [408, 429, 500, 502, 503, 504] {
            assert_eq!(
                classify_status(status, StatementIntent::Write),
                ErrorClass::Ambiguous,
                "{status}"
            );
            assert_eq!(
                classify_status(status, StatementIntent::Unknown),
                ErrorClass::Ambiguous,
                "{status}"
            );
        }
        for status in [408, 429, 502, 503, 504] {
            assert_eq!(
                classify_status(status, StatementIntent::Read),
                ErrorClass::Transient,
                "{status}"
            );
        }
        assert_eq!(
            classify_status(500, StatementIntent::Read),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn a_version_conflict_is_permanent_and_named() {
        let error = response_error(
            409,
            br#"{"error":{"type":"version_conflict_engine_exception","reason":"[1]: version conflict"},"status":409}"#,
            StatementIntent::Write,
        );
        assert_eq!(error.class(), ErrorClass::Permanent);
        assert!(
            error
                .to_string()
                .contains("version_conflict_engine_exception: [1]: version conflict")
        );
    }

    #[test]
    fn the_first_cause_follows_the_reason() {
        let error = response_error(
            400,
            br#"{"error":{"type":"search_phase_execution_exception","reason":"all shards failed","caused_by":{"type":"query_shard_exception","reason":"failed to create query"}}}"#,
            StatementIntent::Read,
        );
        assert!(
            error.to_string().contains(
                "all shards failed (caused by query_shard_exception: failed to create query)"
            ),
            "{error}"
        );
    }

    #[test]
    fn a_refused_credential_is_an_authentication_failure() {
        let error = response_error(
            401,
            br#"{"error":{"type":"security_exception","reason":"unable to authenticate user [ana]"}}"#,
            StatementIntent::Read,
        );
        assert!(matches!(error, OxynError::Authentication(_)), "{error}");
    }

    #[test]
    fn a_body_that_is_not_an_elasticsearch_error_is_said_so() {
        for body in [&b"<html>bad gateway</html>"[..], b"", b"\xff\xfe"] {
            let error = response_error(502, body, StatementIntent::Read);
            assert!(error.to_string().contains("HTTP 502"), "{error}");
            assert_eq!(error.class(), ErrorClass::Transient);
        }
    }

    #[test]
    fn a_long_reason_is_bounded_on_a_character_boundary() {
        let reason = "é".repeat(REASON_CHARS + 50);
        let body = serde_json::json!({"error": {"type": "x", "reason": reason}}).to_string();
        let error = response_error(400, body.as_bytes(), StatementIntent::Read).to_string();
        assert!(error.ends_with('…'), "{error}");
        assert!(error.chars().count() < REASON_CHARS + 100);
    }
}
