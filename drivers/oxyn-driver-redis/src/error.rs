//! The driver's errors, and the family each belongs to.
//!
//! # A Redis error names itself
//!
//! An error reply starts with an uppercase word, its *prefix*: `WRONGTYPE`,
//! `NOPERM`, `LOADING`. The protocol documents the convention and the prefix
//! is stable where the text is not ([RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md)),
//! so the class is read from it — data, never a match on the message
//! ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).
//!
//! # Transient only when nothing ran
//!
//! The transient prefixes are those Redis answers **before** running the
//! command — `processCommand` refuses it while loading, while a script is
//! busy, out of memory, when persistence or replicas fail: retrying later
//! cannot apply anything twice. Every other refusal is permanent. Two cases
//! are ambiguous, never retried ([I-13](../../../CLAUDE.md#i-13)): the
//! connection lost or the time spent after a command that may write was sent,
//! and **any** error of a script but `NOSCRIPT`: Redis does not undo what a
//! script wrote before it failed, and the script chooses its error's prefix —
//! a `TRYAGAIN` it returns after a `DECRBY` is not the server's refusal.

use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};

use crate::resp::WireError;

/// What can go wrong between the driver and the server.
///
/// No variant carries a password, a URL or a connection string: the
/// configuration never reaches this type ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RedisError {
    /// The server answered an error.
    #[error("{prefix} {message}")]
    Server {
        /// The error's first word: `ERR`, `WRONGTYPE`, `NOPERM`…
        prefix: String,
        /// The rest of the server's text.
        message: String,
    },
    /// The connection failed or the bytes were not RESP.
    #[error(transparent)]
    Wire(#[from] WireError),
    /// A reply did not have the shape its command documents.
    #[error("unexpected reply to {command}: {detail}")]
    UnexpectedReply {
        /// The command whose reply was unexpected.
        command: &'static str,
        /// What was wrong with it.
        detail: String,
    },
}

impl RedisError {
    /// Builds a server error from the bytes of an error reply.
    #[must_use]
    pub fn server(text: &[u8]) -> Self {
        let text = String::from_utf8_lossy(text);
        let (prefix, message) = text.split_once(' ').unwrap_or((&text, ""));
        Self::Server {
            prefix: prefix.to_owned(),
            message: message.to_owned(),
        }
    }

    /// The prefix of a server error.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        match self {
            Self::Server { prefix, .. } => Some(prefix),
            Self::Wire(_) | Self::UnexpectedReply { .. } => None,
        }
    }

    /// Whether this is the error a script stopped by `SCRIPT KILL` or
    /// `FUNCTION KILL` answers: both say `Script killed by user`.
    #[must_use]
    pub fn is_script_kill(&self) -> bool {
        matches!(self, Self::Server { prefix, message }
            if prefix == "ERR" && message.starts_with("Script killed by user"))
    }
}

/// The family of a server error, by its prefix.
///
/// `scripted` says the command runs a script (`@scripting`): its error may
/// come after writes the script made, which Redis keeps.
#[must_use]
pub fn class_of(prefix: &str, scripted: bool) -> ErrorClass {
    if scripted {
        // A script chooses its error's prefix — `redis.error_reply("TRYAGAIN
        // …")` — and an inner `redis.call` raises `WRONGTYPE` after the writes
        // before it: no prefix proves what the script left behind. Only a
        // script the server does not have ran nothing.
        return if prefix == "NOSCRIPT" {
            ErrorClass::Permanent
        } else {
            ErrorClass::Ambiguous
        };
    }
    match prefix {
        // Refused before running: nothing was applied.
        "LOADING" | "BUSY" | "OOM" | "MISCONF" | "NOREPLICAS" | "MASTERDOWN" | "TRYAGAIN"
        | "CLUSTERDOWN" => ErrorClass::Transient,
        // Refusals that say nothing ran, whatever the command.
        "NOSCRIPT" | "NOPERM" | "NOAUTH" | "WRONGPASS" | "WRONGTYPE" | "READONLY" | "EXECABORT"
        | "CROSSSLOT" | "MOVED" | "ASK" | "NOPROTO" | "UNBLOCKED" => ErrorClass::Permanent,
        // `MIGRATE`'s transfer failed or timed out: "the key may be on both
        // the instances", or only on the source.
        "IOERR" => ErrorClass::Ambiguous,
        _ => ErrorClass::Permanent,
    }
}

/// Turns a failure during an execution into the error the caller reads.
///
/// `sent` says whether the command reached the connection: a failure before
/// that applied nothing. After it, losing the connection is transient for a
/// read and **ambiguous** for anything else — the server may have applied the
/// write ([I-13](../../../CLAUDE.md#i-13)).
#[must_use]
pub fn during_execution(
    driver: &DriverId,
    error: RedisError,
    intent: Option<StatementIntent>,
    scripted: bool,
    sent: bool,
) -> OxynError {
    let class = match &error {
        RedisError::Server { prefix, .. } => class_of(prefix, scripted),
        RedisError::Wire(_) if !sent => ErrorClass::Transient,
        RedisError::Wire(_) | RedisError::UnexpectedReply { .. } => {
            if intent == Some(StatementIntent::Read) {
                ErrorClass::Transient
            } else {
                return OxynError::OutcomeUnknown(format!(
                    "the connection failed after the command was sent ({error}); the server \
                     may have applied it: check the data before running it again"
                ));
            }
        }
    };
    OxynError::driver(driver.clone(), class, error)
}

/// Turns a failure while opening a connection into the error the caller
/// reads: authentication refusals are [`OxynError::Authentication`], the rest
/// a [`OxynError::Connection`] — nothing was run yet.
#[must_use]
pub fn during_connect(error: &RedisError) -> OxynError {
    match error.prefix() {
        Some("WRONGPASS" | "NOAUTH" | "NOPERM") => OxynError::Authentication(error.to_string()),
        _ => OxynError::Connection(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn redis() -> DriverId {
        DriverId::new("redis").expect("a valid identifier")
    }

    #[test]
    fn a_server_error_splits_into_its_prefix_and_text() {
        let error = RedisError::server(
            b"WRONGTYPE Operation against a key holding the wrong kind of value",
        );
        assert_eq!(error.prefix(), Some("WRONGTYPE"));
        assert_eq!(
            error.to_string(),
            "WRONGTYPE Operation against a key holding the wrong kind of value"
        );
        let bare = RedisError::server(b"ERR");
        assert_eq!(bare.prefix(), Some("ERR"));
    }

    /// The text observed on `redis:8.10.2` for a killed `EVAL` and a killed
    /// `FCALL`; a script's own error is not a kill.
    #[test]
    fn only_the_servers_kill_error_proves_a_killed_script() {
        for killed in [
            &b"ERR Script killed by user with SCRIPT KILL... script: 694a5fe1, on @user_script:1."
                [..],
            b"ERR Script killed by user with SCRIPT KILL... script: spin, on @user_function:2.",
        ] {
            assert!(RedisError::server(killed).is_script_kill());
        }
        for other in [
            &b"ERR user_script:1: Script attempted to access a non local key"[..],
            b"BUSY Redis is busy running a script.",
            b"Script killed by user",
        ] {
            assert!(!RedisError::server(other).is_script_kill());
        }
    }

    #[test]
    fn an_error_that_is_not_utf8_is_still_shown() {
        let error = RedisError::server(b"ERR bad key \xff");
        assert_eq!(error.prefix(), Some("ERR"));
    }

    #[rstest]
    #[case("LOADING", false, ErrorClass::Transient)]
    #[case("BUSY", false, ErrorClass::Transient)]
    #[case("OOM", false, ErrorClass::Transient)]
    #[case("MISCONF", false, ErrorClass::Transient)]
    #[case("ERR", false, ErrorClass::Permanent)]
    #[case("WRONGTYPE", false, ErrorClass::Permanent)]
    #[case("NOPERM", false, ErrorClass::Permanent)]
    // A script's error, whatever prefix it chose, may follow its writes.
    #[case("TRYAGAIN", true, ErrorClass::Ambiguous)]
    #[case("MISCONF", true, ErrorClass::Ambiguous)]
    #[case("WRONGTYPE", true, ErrorClass::Ambiguous)]
    #[case("NOPERM", true, ErrorClass::Ambiguous)]
    #[case("NOSCRIPT", true, ErrorClass::Permanent)]
    #[case("IOERR", false, ErrorClass::Ambiguous)]
    #[case("MOVED", false, ErrorClass::Permanent)]
    #[case("UNBLOCKED", false, ErrorClass::Permanent)]
    #[case("ERR", true, ErrorClass::Ambiguous)]
    #[case("SOMETHINGNEW", false, ErrorClass::Permanent)]
    #[case("SOMETHINGNEW", true, ErrorClass::Ambiguous)]
    fn the_class_is_read_from_the_prefix(
        #[case] prefix: &str,
        #[case] scripted: bool,
        #[case] expected: ErrorClass,
    ) {
        assert_eq!(class_of(prefix, scripted), expected);
    }

    #[rstest]
    #[case(Some(StatementIntent::Write))]
    #[case(Some(StatementIntent::Grant))]
    #[case(None)]
    fn a_connection_lost_after_sending_a_write_is_never_retryable(
        #[case] intent: Option<StatementIntent>,
    ) {
        let error = during_execution(
            &redis(),
            RedisError::Wire(WireError::Truncated),
            intent,
            false,
            true,
        );
        assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
        assert_eq!(error.class(), ErrorClass::Ambiguous);
        assert!(!error.is_retryable());
    }

    #[test]
    fn a_connection_lost_during_a_read_or_before_sending_is_transient() {
        let read = during_execution(
            &redis(),
            RedisError::Wire(WireError::Truncated),
            Some(StatementIntent::Read),
            false,
            true,
        );
        assert_eq!(read.class(), ErrorClass::Transient);
        let unsent = during_execution(
            &redis(),
            RedisError::Wire(WireError::Truncated),
            None,
            false,
            false,
        );
        assert_eq!(unsent.class(), ErrorClass::Transient);
    }

    #[test]
    fn an_authentication_refusal_at_connect_is_an_authentication_error() {
        let refused =
            RedisError::server(b"WRONGPASS invalid username-password pair or user is disabled.");
        assert!(matches!(
            during_connect(&refused),
            OxynError::Authentication(_)
        ));
        let lost = RedisError::Wire(WireError::Truncated);
        assert!(matches!(during_connect(&lost), OxynError::Connection(_)));
    }
}
