//! Classifying what ClickHouse answers, and above all: not calling it
//! permanent or transient when the write may have landed.
//!
//! # Why a server error on a write is ambiguous by default
//!
//! ClickHouse has no transaction in the general case, and an `INSERT` is
//! applied block by block: an `INSERT … SELECT` that fails on its fiftieth
//! block keeps the forty-nine before it. An error on a statement that is not a
//! read therefore says nothing about what was applied — unless it is one the
//! server raises **before running anything** (a syntax error, an unknown
//! table, a missing privilege), and the response's own summary says no row was
//! written. Everything else on a write is [`ErrorClass::Ambiguous`]: never
//! replayed ([I-13](../../../CLAUDE.md#i-13)).
//!
//! The codes are ClickHouse's `ErrorCodes`, read on a 26.3 server with
//! `errorCodeToName` on 2026-10-23
//! ([RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md)). The class is data the
//! driver attaches: the caller never reads the message.
//!
//! # What this module does not do
//!
//! It retries nothing: the retry policy belongs to the caller.

use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};

use crate::measure::DecodeError;

/// The longest server message kept. A message quotes the statement; a
/// statement of megabytes would otherwise travel into every log and history
/// entry.
pub(crate) const MAX_MESSAGE_BYTES: usize = 16 * 1024;

// ── Codes, as `errorCodeToName` names them ──────────────────────────────────
const THERE_IS_NO_COLUMN: u32 = 8;
const DUPLICATE_COLUMN: u32 = 15;
const NO_SUCH_COLUMN_IN_TABLE: u32 = 16;
const BAD_ARGUMENTS: u32 = 36;
const ILLEGAL_TYPE_OF_ARGUMENT: u32 = 43;
const UNKNOWN_FUNCTION: u32 = 46;
const UNKNOWN_IDENTIFIER: u32 = 47;
const NOT_IMPLEMENTED: u32 = 48;
const UNKNOWN_TYPE: u32 = 50;
const TYPE_MISMATCH: u32 = 53;
const TABLE_ALREADY_EXISTS: u32 = 57;
const UNKNOWN_TABLE: u32 = 60;
const SYNTAX_ERROR: u32 = 62;
const UNKNOWN_AGGREGATE_FUNCTION: u32 = 63;
const UNKNOWN_FORMAT: u32 = 73;
const UNKNOWN_DATABASE: u32 = 81;
const DATABASE_ALREADY_EXISTS: u32 = 82;
const UNKNOWN_SETTING: u32 = 115;
const TIMEOUT_EXCEEDED: u32 = 159;
const READONLY: u32 = 164;
const CANNOT_ALLOCATE_MEMORY: u32 = 173;
const UNKNOWN_USER: u32 = 192;
const WRONG_PASSWORD: u32 = 193;
const REQUIRED_PASSWORD: u32 = 194;
const IP_ADDRESS_NOT_ALLOWED: u32 = 195;
const QUOTA_EXCEEDED: u32 = 201;
const TOO_MANY_SIMULTANEOUS_QUERIES: u32 = 202;
const SOCKET_TIMEOUT: u32 = 209;
const NETWORK_ERROR: u32 = 210;
const QUERY_WITH_SAME_ID_IS_ALREADY_RUNNING: u32 = 216;
const NO_ZOOKEEPER: u32 = 225;
const QUERY_IS_TOO_LARGE: u32 = 229;
const ABORTED: u32 = 236;
const MEMORY_LIMIT_EXCEEDED: u32 = 241;
const TABLE_IS_READ_ONLY: u32 = 242;
const TOO_MANY_PARTS: u32 = 252;
const ALL_CONNECTION_TRIES_FAILED: u32 = 279;
const DATABASE_ACCESS_DENIED: u32 = 291;
const SUPPORT_IS_DISABLED: u32 = 344;
const INTO_OUTFILE_NOT_ALLOWED: u32 = 358;
const TABLE_SIZE_EXCEEDS_MAX_DROP_SIZE_LIMIT: u32 = 359;
/// What a `KILL QUERY` makes the killed statement answer.
pub(crate) const QUERY_WAS_CANCELLED: u32 = 394;
const UNKNOWN_MUTATION_COMMAND: u32 = 398;
const CANNOT_SCHEDULE_TASK: u32 = 439;
const UNKNOWN_QUERY_PARAMETER: u32 = 456;
const BAD_QUERY_PARAMETER: u32 = 457;
const DEADLOCK_AVOIDED: u32 = 473;
const ACCESS_DENIED: u32 = 497;
const INVALID_GRANT: u32 = 509;
const UNKNOWN_ROLE: u32 = 511;
const AUTHENTICATION_FAILED: u32 = 516;
const USER_EXPIRED: u32 = 720;
const SERVER_OVERLOADED: u32 = 745;
const KEEPER_EXCEPTION: u32 = 999;

/// Codes the server raises while reading, resolving or authorizing the
/// statement — before it runs anything. On a write, they are permanent only
/// when the response also says no row was written.
const REFUSED_BEFORE_RUNNING: [u32; 31] = [
    THERE_IS_NO_COLUMN,
    DUPLICATE_COLUMN,
    NO_SUCH_COLUMN_IN_TABLE,
    BAD_ARGUMENTS,
    ILLEGAL_TYPE_OF_ARGUMENT,
    UNKNOWN_FUNCTION,
    UNKNOWN_IDENTIFIER,
    NOT_IMPLEMENTED,
    UNKNOWN_TYPE,
    TYPE_MISMATCH,
    TABLE_ALREADY_EXISTS,
    UNKNOWN_TABLE,
    SYNTAX_ERROR,
    UNKNOWN_AGGREGATE_FUNCTION,
    UNKNOWN_FORMAT,
    UNKNOWN_DATABASE,
    DATABASE_ALREADY_EXISTS,
    UNKNOWN_SETTING,
    READONLY,
    QUERY_WITH_SAME_ID_IS_ALREADY_RUNNING,
    QUERY_IS_TOO_LARGE,
    DATABASE_ACCESS_DENIED,
    SUPPORT_IS_DISABLED,
    INTO_OUTFILE_NOT_ALLOWED,
    TABLE_SIZE_EXCEEDS_MAX_DROP_SIZE_LIMIT,
    UNKNOWN_MUTATION_COMMAND,
    UNKNOWN_QUERY_PARAMETER,
    BAD_QUERY_PARAMETER,
    ACCESS_DENIED,
    INVALID_GRANT,
    UNKNOWN_ROLE,
];

/// Codes of a passing condition: load, memory, a lost replica, a network hop
/// between servers. A read may be replayed; a write may have landed.
const PASSING: [u32; 16] = [
    TIMEOUT_EXCEEDED,
    CANNOT_ALLOCATE_MEMORY,
    QUOTA_EXCEEDED,
    SOCKET_TIMEOUT,
    NETWORK_ERROR,
    NO_ZOOKEEPER,
    ABORTED,
    MEMORY_LIMIT_EXCEEDED,
    TABLE_IS_READ_ONLY,
    TOO_MANY_PARTS,
    ALL_CONNECTION_TRIES_FAILED,
    QUERY_WAS_CANCELLED,
    CANNOT_SCHEDULE_TASK,
    DEADLOCK_AVOIDED,
    SERVER_OVERLOADED,
    KEEPER_EXCEPTION,
];

/// Codes that refuse the account rather than the statement.
const AUTHENTICATION: [u32; 6] = [
    UNKNOWN_USER,
    WRONG_PASSWORD,
    REQUIRED_PASSWORD,
    IP_ADDRESS_NOT_ALLOWED,
    AUTHENTICATION_FAILED,
    USER_EXPIRED,
];

/// The driver error as wrapped in [`OxynError::Driver`].
///
/// Named, so that a caller who wants the server's code reads it through
/// `downcast_ref` without parsing a message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ClickhouseError {
    message: String,
    code: Option<u32>,
}

impl ClickhouseError {
    /// The server's error code (`62`, `394`…), when the server gave one.
    #[must_use]
    pub const fn code(&self) -> Option<u32> {
        self.code
    }

    /// The message as it will be shown.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// What the server said when it refused or interrupted a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerError {
    /// `X-ClickHouse-Exception-Code`, or the `Code: N.` the message opens with.
    pub code: Option<u32>,
    /// The server's text, bounded to `MAX_MESSAGE_BYTES`.
    pub message: String,
    /// Rows the response's summary says were written before the error; `None`
    /// when the response carried no summary.
    pub written_rows: Option<u64>,
}

/// What replaces the server's message when the statement carried bound
/// values: ClickHouse quotes a value it could not parse (`Value … cannot be
/// parsed as UInt64 for query parameter 'id'`), and the message ends up on
/// screen, in the history and in the journal ([I-03](../../../CLAUDE.md#i-03)).
/// Explicit, so that nobody looks for a defect in their SQL.
const WITHHELD_MESSAGE: &str = "the server message is withheld because the statement carried \
                                bound values, which ClickHouse can quote verbatim";

impl ServerError {
    /// The same error with its message withheld, its code kept.
    #[must_use]
    pub fn withheld(self) -> Self {
        let message = match self.code {
            Some(code) => format!("{WITHHELD_MESSAGE} (code {code})"),
            None => WITHHELD_MESSAGE.to_owned(),
        };
        Self { message, ..self }
    }

    /// Reads a server message, taking its code from it when no header gave
    /// one: `Code: 395. DB::Exception: …`.
    #[must_use]
    pub fn from_text(header_code: Option<u32>, text: &[u8], written_rows: Option<u64>) -> Self {
        let bounded = text.get(..MAX_MESSAGE_BYTES).unwrap_or(text);
        let message = String::from_utf8_lossy(bounded).trim().to_owned();
        let code = header_code.or_else(|| {
            message
                .strip_prefix("Code: ")
                .and_then(|rest| rest.split('.').next())
                .and_then(|digits| digits.trim().parse().ok())
        });
        Self {
            code,
            message,
            written_rows,
        }
    }
}

/// The family of a server error, for a statement of `intent`.
#[must_use]
pub fn classify(error: &ServerError, intent: StatementIntent) -> ErrorClass {
    let Some(code) = error.code else {
        return if intent.is_read_only() {
            ErrorClass::Permanent
        } else {
            ErrorClass::Ambiguous
        };
    };
    let wrote_nothing = error.written_rows == Some(0);
    if code == TOO_MANY_SIMULTANEOUS_QUERIES && wrote_nothing {
        // Refused when the statement was registered, before it ran.
        return ErrorClass::Transient;
    }
    if intent.is_read_only() {
        if PASSING.contains(&code) {
            ErrorClass::Transient
        } else {
            ErrorClass::Permanent
        }
    } else if REFUSED_BEFORE_RUNNING.contains(&code) && wrote_nothing {
        ErrorClass::Permanent
    } else {
        ErrorClass::Ambiguous
    }
}

/// A server error as the domain error the caller reads.
#[must_use]
pub fn map_server_error(
    driver: &DriverId,
    intent: StatementIntent,
    error: ServerError,
) -> OxynError {
    if error
        .code
        .is_some_and(|code| AUTHENTICATION.contains(&code))
    {
        return OxynError::Authentication(error.message);
    }
    let class = classify(&error, intent);
    OxynError::driver(
        driver.clone(),
        class,
        ClickhouseError {
            message: error.message,
            code: error.code,
        },
    )
}

/// A failure of the HTTP exchange itself.
///
/// The message is the driver's, fixed: `reqwest`'s own carries the URL, and
/// the URL names the host ([I-03](../../../CLAUDE.md#i-03), as
/// [`OxynError::OutcomeUnknown`] asks).
#[must_use]
pub fn map_transport_error(
    driver: &DriverId,
    intent: StatementIntent,
    sent: bool,
    error: &reqwest::Error,
) -> OxynError {
    if !sent || error.is_connect() {
        return OxynError::Connection(if error.is_timeout() {
            "the server did not accept the connection in time".to_owned()
        } else {
            "the server cannot be reached".to_owned()
        });
    }
    if intent.is_read_only() {
        return OxynError::driver(
            driver.clone(),
            ErrorClass::Transient,
            ClickhouseError {
                message: "the connection to the server was lost during a read".to_owned(),
                code: None,
            },
        );
    }
    OxynError::OutcomeUnknown(
        "the connection to the server was lost after the statement was sent: ClickHouse may \
         have applied it, in whole or in part — check the data before running it again"
            .to_owned(),
    )
}

/// A response the driver cannot read, after the statement was sent.
#[must_use]
pub fn map_decode_error(
    driver: &DriverId,
    intent: StatementIntent,
    error: &DecodeError,
) -> OxynError {
    // The decoder refuses what the server sent; for a read, replaying sends it
    // again. A write's response is never decoded past its end.
    let class = if intent.is_read_only() {
        ErrorClass::Permanent
    } else {
        ErrorClass::Ambiguous
    };
    OxynError::driver(
        driver.clone(),
        class,
        ClickhouseError {
            message: error.to_string(),
            code: None,
        },
    )
}

/// An error the driver raises about what the server sent.
#[must_use]
pub fn protocol_error(driver: &DriverId, intent: StatementIntent, message: &str) -> OxynError {
    let class = if intent.is_read_only() {
        ErrorClass::Permanent
    } else {
        ErrorClass::Ambiguous
    };
    OxynError::driver(
        driver.clone(),
        class,
        ClickhouseError {
            message: message.to_owned(),
            code: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn server(code: u32, written_rows: Option<u64>) -> ServerError {
        ServerError {
            code: Some(code),
            message: format!("Code: {code}. DB::Exception: x"),
            written_rows,
        }
    }

    #[rstest]
    #[case(SYNTAX_ERROR, StatementIntent::Read, ErrorClass::Permanent)]
    #[case(UNKNOWN_TABLE, StatementIntent::Write, ErrorClass::Permanent)]
    #[case(ACCESS_DENIED, StatementIntent::Ddl, ErrorClass::Permanent)]
    #[case(MEMORY_LIMIT_EXCEEDED, StatementIntent::Read, ErrorClass::Transient)]
    #[case(MEMORY_LIMIT_EXCEEDED, StatementIntent::Write, ErrorClass::Ambiguous)]
    #[case(TIMEOUT_EXCEEDED, StatementIntent::Unknown, ErrorClass::Ambiguous)]
    #[case(TOO_MANY_PARTS, StatementIntent::Write, ErrorClass::Ambiguous)]
    #[case(
        TOO_MANY_SIMULTANEOUS_QUERIES,
        StatementIntent::Write,
        ErrorClass::Transient
    )]
    #[case(395, StatementIntent::Read, ErrorClass::Permanent)]
    #[case(395, StatementIntent::Write, ErrorClass::Ambiguous)]
    fn the_class_follows_the_code_and_the_intent(
        #[case] code: u32,
        #[case] intent: StatementIntent,
        #[case] expected: ErrorClass,
    ) {
        assert_eq!(classify(&server(code, Some(0)), intent), expected);
    }

    #[test]
    fn a_write_that_wrote_rows_is_ambiguous_whatever_the_code() {
        // An `INSERT … SELECT` failing on a later block keeps the earlier ones.
        assert_eq!(
            classify(&server(TYPE_MISMATCH, Some(49_000)), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
        // Without a summary, nothing proves that nothing was written.
        assert_eq!(
            classify(&server(SYNTAX_ERROR, None), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
        // A read never wrote anything.
        assert_eq!(
            classify(&server(SYNTAX_ERROR, None), StatementIntent::Read),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn the_code_is_read_from_the_message_when_no_header_carries_it() {
        let error = ServerError::from_text(None, b"Code: 395. DB::Exception: boom", None);
        assert_eq!(error.code, Some(395));
        let error = ServerError::from_text(Some(62), b"Code: 1. whatever", None);
        assert_eq!(error.code, Some(62), "the header wins");
        let error = ServerError::from_text(None, b"no code here", None);
        assert_eq!(error.code, None);
        assert_eq!(
            classify(&error, StatementIntent::Write),
            ErrorClass::Ambiguous
        );
    }

    #[test]
    fn a_huge_message_is_bounded() {
        let text = vec![b'x'; MAX_MESSAGE_BYTES * 4];
        let error = ServerError::from_text(Some(62), &text, Some(0));
        assert!(error.message.len() <= MAX_MESSAGE_BYTES);
    }

    #[test]
    fn a_refused_account_is_an_authentication_error() {
        let mapped = map_server_error(
            &DriverId::new("clickhouse").expect("valid"),
            StatementIntent::Read,
            server(AUTHENTICATION_FAILED, Some(0)),
        );
        assert!(matches!(mapped, OxynError::Authentication(_)), "{mapped:?}");
    }

    #[test]
    fn the_code_travels_as_data() {
        let mapped = map_server_error(
            &DriverId::new("clickhouse").expect("valid"),
            StatementIntent::Read,
            server(UNKNOWN_TABLE, Some(0)),
        );
        let OxynError::Driver { source, class, .. } = &mapped else {
            panic!("a driver error: {mapped:?}");
        };
        assert_eq!(*class, ErrorClass::Permanent);
        let error = source
            .downcast_ref::<ClickhouseError>()
            .expect("the driver's own error type");
        assert_eq!(error.code(), Some(UNKNOWN_TABLE));
    }

    #[test]
    fn a_withheld_message_keeps_the_code_and_drops_the_value() {
        let error = ServerError::from_text(
            Some(457),
            b"Code: 457. DB::Exception: Value hunter2 cannot be parsed as UInt64 \
              for query parameter 'id'",
            Some(0),
        )
        .withheld();
        assert!(!error.message.contains("hunter2"), "{}", error.message);
        assert!(error.message.ends_with("(code 457)"), "{}", error.message);
        assert_eq!(error.code, Some(457));
        assert_eq!(error.written_rows, Some(0));
    }
}
