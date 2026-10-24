//! Classifying a `mongodb` error from its code and labels, and above all: not
//! classifying it transient when it is ambiguous.
//!
//! [DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md) gives three families.
//! The class is **data**: the server's numeric `code`, its `codeName` and the
//! error labels it attaches, never the wording of a message. The intent of the
//! command decides between the transient and the ambiguous reading of the same
//! failure: a cut during a `find` is transient, during an `update` it is
//! ambiguous ([I-13](../../../CLAUDE.md#i-13)).
//!
//! # The library's retries are off, and its labels say why it matters
//!
//! `mongodb` retries reads and writes once by default. The session disables
//! both (`options.rs`): the retry policy belongs to the caller. A
//! `RetryableWriteError` label then still says the server *could* have been
//! asked again **under the same transaction number**, which makes a replay
//! idempotent — a guarantee Oxyn does not send, since console commands carry no
//! `txnNumber`. For a write, that label is therefore ambiguous, not transient.
//!
//! # No address in a message
//!
//! The library's own messages for selection, DNS and I/O failures quote the
//! topology — host names, ports, replica set members. The messages here are
//! the driver's, built from the kind of failure; the server's `errmsg` is kept
//! for command errors only, where it names what the user's command did.
//!
//! # No secret the server was given
//!
//! The server's words are shown, code included
//! ([UX-SPEC](../../../docs/UX-SPEC.md#errors-are-addressed-to-a-professional)),
//! except where they could quote a secret Oxyn sent it. The connection's
//! password never reaches the server: the session authenticates by SCRAM
//! only, which proves the password without sending it (`options.rs`). A
//! failed authentication still reads as a fixed message, the server's text
//! dropped. What remains is a password the command itself carries — the
//! `pwd` of `createUser` or `updateUser`: a hostile or compromised server may
//! quote it in any error, cut or whole. For such a command, the error keeps
//! only its numeric code and the labels Oxyn reads
//! ([`map_statement_error`]), like the ClickHouse driver withholds the message
//! of a statement that carries a password.

use mongodb::error::{Error as MongoLibError, ErrorKind};
use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};

/// `Interrupted`: an operation killed by `killOp` or `killSessions`.
pub(crate) const INTERRUPTED: i32 = 11601;
/// `InterruptedAtShutdown`.
const INTERRUPTED_AT_SHUTDOWN: i32 = 11600;
/// `InterruptedDueToReplStateChange`.
const INTERRUPTED_REPL_STATE: i32 = 11602;
/// `CursorKilled`: a cursor killed while a `getMore` read it.
pub(crate) const CURSOR_KILLED: i32 = 237;
/// `CursorNotFound`.
const CURSOR_NOT_FOUND: i32 = 43;
/// `MaxTimeMSExpired`: the user's own `maxTimeMS` bound.
const MAX_TIME_MS_EXPIRED: i32 = 50;
/// `AuthenticationFailed`.
const AUTHENTICATION_FAILED: i32 = 18;
/// `HostUnreachable`.
const HOST_UNREACHABLE: i32 = 6;
/// `HostNotFound`.
const HOST_NOT_FOUND: i32 = 7;
/// `NetworkTimeout`.
const NETWORK_TIMEOUT: i32 = 89;
/// `ShutdownInProgress`.
const SHUTDOWN_IN_PROGRESS: i32 = 91;
/// `PrimarySteppedDown`.
const PRIMARY_STEPPED_DOWN: i32 = 189;
/// `ExceededTimeLimit`.
const EXCEEDED_TIME_LIMIT: i32 = 262;
/// `SocketException`.
const SOCKET_EXCEPTION: i32 = 9001;
/// `NotWritablePrimary`.
const NOT_WRITABLE_PRIMARY: i32 = 10107;
/// `NotPrimaryNoSecondaryOk`.
const NOT_PRIMARY_NO_SECONDARY_OK: i32 = 13435;
/// `NotPrimaryOrSecondary`.
const NOT_PRIMARY_OR_SECONDARY: i32 = 13436;
/// `LockTimeout`.
const LOCK_TIMEOUT: i32 = 24;
/// `WriteConflict`.
const WRITE_CONFLICT: i32 = 112;
/// `ExceededMemoryLimit`.
const EXCEEDED_MEMORY_LIMIT: i32 = 146;
/// `QueryExceededMemoryLimitNoDiskUseAllowed`.
const MEMORY_LIMIT_NO_DISK_USE: i32 = 292;
/// `BSONObjectTooLarge`.
const BSON_OBJECT_TOO_LARGE: i32 = 10334;

/// The labels a server or the library attaches, read as data.
const TRANSIENT_TRANSACTION: &str = "TransientTransactionError";
const UNKNOWN_COMMIT_RESULT: &str = "UnknownTransactionCommitResult";
const RETRYABLE_WRITE: &str = "RetryableWriteError";

/// The driver error as wrapped in [`OxynError::Driver`].
///
/// A named type: a caller that wants the code reads it through
/// `downcast_ref`, without parsing a message. The library's error is never
/// kept alongside — its `Display` quotes the topology.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct MongoError {
    message: String,
    code: Option<i32>,
    code_name: Option<String>,
    labels: Vec<String>,
}

impl MongoError {
    /// The server's numeric error code, when the server gave one.
    #[must_use]
    pub const fn code(&self) -> Option<i32> {
        self.code
    }

    /// The server's `codeName`, when it gave one.
    #[must_use]
    pub fn code_name(&self) -> Option<&str> {
        self.code_name.as_deref()
    }

    /// The error labels, sorted.
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// An error the driver raises about what the server sent.
    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            code_name: None,
            labels: Vec::new(),
        }
    }
}

/// The server's code of a command error.
pub(crate) fn server_code(error: &MongoLibError) -> Option<i32> {
    match error.kind.as_ref() {
        ErrorKind::Command(command) => Some(command.code),
        _ => None,
    }
}

/// Did the server report the operation interrupted, the way `killSessions`
/// and `killCursors` make it? A `getMore` on a cursor `killSessions` closed
/// between two batches finds no cursor at all: `CursorNotFound`.
fn is_interruption(error: &MongoLibError) -> bool {
    matches!(
        server_code(error),
        Some(INTERRUPTED | CURSOR_KILLED | CURSOR_NOT_FOUND)
    )
}

/// Is the failure a cut of the transport, after which the request's fate is
/// not known?
fn is_transport(error: &MongoLibError) -> bool {
    matches!(
        error.kind.as_ref(),
        ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. }
    )
}

/// The class of a server code, for a command of `intent`.
fn class_of_code(code: i32, intent: StatementIntent, labels: &[String]) -> ErrorClass {
    let has = |label: &str| labels.iter().any(|l| l == label);
    if has(UNKNOWN_COMMIT_RESULT) {
        return ErrorClass::Ambiguous;
    }
    let transient_for_reads = matches!(
        code,
        HOST_UNREACHABLE
            | HOST_NOT_FOUND
            | NETWORK_TIMEOUT
            | SHUTDOWN_IN_PROGRESS
            | PRIMARY_STEPPED_DOWN
            | SOCKET_EXCEPTION
            | NOT_WRITABLE_PRIMARY
            | NOT_PRIMARY_NO_SECONDARY_OK
            | NOT_PRIMARY_OR_SECONDARY
            | INTERRUPTED_AT_SHUTDOWN
            | INTERRUPTED_REPL_STATE
            | LOCK_TIMEOUT
            | WRITE_CONFLICT
            | EXCEEDED_TIME_LIMIT
    ) || has(TRANSIENT_TRANSACTION)
        || has(RETRYABLE_WRITE);
    if intent.is_read_only() {
        return if transient_for_reads {
            ErrorClass::Transient
        } else {
            ErrorClass::Permanent
        };
    }
    // A command that may write and failed for any reason but a refusal the
    // server makes before touching a document may have applied part of its
    // work: a multi-document update stopped by its own `maxTimeMS` or a
    // `killOp`, a `$merge` that hit a duplicate key halfway. The verdict
    // follows the statement, not the cause (DRIVER-CONTRACT §4): replaying
    // it would apply that part twice.
    if PRE_EXECUTION.contains(&code) {
        ErrorClass::Permanent
    } else {
        ErrorClass::Ambiguous
    }
}

/// The refusals the server makes while reading a command, before it runs:
/// nothing was applied. A code that can also be raised while documents are
/// written — `BadValue` and `TypeMismatch` from an expression, `DuplicateKey`
/// from a `$merge`, `BSONObjectTooLarge` from a stage — is not in the list.
const PRE_EXECUTION: &[i32] = &[
    9,   // FailedToParse
    13,  // Unauthorized
    20,  // IllegalOperation
    22,  // InvalidBSON
    26,  // NamespaceNotFound
    48,  // NamespaceExists
    59,  // CommandNotFound
    72,  // InvalidOptions
    73,  // InvalidNamespace
    85,  // IndexOptionsConflict
    86,  // IndexKeySpecsConflict
    166, // CommandNotSupportedOnView
    167, // OptionNotSupportedOnView
    168, // InvalidPipelineOperator
    182, // InvalidViewDefinition
    323, // APIStrictError
];

/// A user-facing hint for the server limits that have a known remedy.
fn remedy(code: i32) -> Option<&'static str> {
    match code {
        MEMORY_LIMIT_NO_DISK_USE | EXCEEDED_MEMORY_LIMIT => Some(
            "the aggregation stage exceeded the server's memory limit; add \
             \"allowDiskUse\": true to the command, or reduce what the stage holds",
        ),
        BSON_OBJECT_TOO_LARGE => {
            Some("a document would exceed the server's maximum BSON document size (16 MiB)")
        }
        MAX_TIME_MS_EXPIRED => Some("the command's own maxTimeMS bound was reached"),
        _ => None,
    }
}

/// Maps a library error for a command of `intent`.
///
/// `cancelled` says the session itself sent `killSessions` for this command:
/// the server's `Interrupted` is then the stop the user asked for, and its
/// verdict depends on whether the command could have written
/// ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md)): a read is
/// [`OxynError::Cancelled`], anything else [`OxynError::OutcomeUnknown`].
pub(crate) fn map_error(
    driver: &DriverId,
    intent: StatementIntent,
    error: &MongoLibError,
    cancelled: bool,
) -> OxynError {
    map_statement_error(driver, intent, error, cancelled, false)
}

/// What replaces the server's message when the command carried a password.
const WITHHELD_CREDENTIAL: &str = "the server message is withheld because the command carries a \
                                   password (`pwd`), which a server can quote verbatim";

/// [`map_error`] for a console command: when it `carries_credential`, no text
/// the server chose — message, code name, unknown label — reaches the error,
/// its `Display` or its `Debug`.
pub(crate) fn map_statement_error(
    driver: &DriverId,
    intent: StatementIntent,
    error: &MongoLibError,
    cancelled: bool,
    carries_credential: bool,
) -> OxynError {
    let mut labels: Vec<String> = error
        .labels()
        .iter()
        .filter(|label| {
            !carries_credential
                || [
                    TRANSIENT_TRANSACTION,
                    UNKNOWN_COMMIT_RESULT,
                    RETRYABLE_WRITE,
                ]
                .contains(&label.as_str())
        })
        .cloned()
        .collect();
    labels.sort_unstable();
    // `Custom` is the session's own marker of a stop the server did not answer
    // within its grace period.
    if cancelled
        && (is_interruption(error)
            || is_transport(error)
            || matches!(error.kind.as_ref(), ErrorKind::Custom(_)))
    {
        return stopped(intent);
    }
    match error.kind.as_ref() {
        ErrorKind::Authentication { .. } => OxynError::Authentication(
            "the server refused the credentials or the authentication mechanism".to_owned(),
        ),
        ErrorKind::Command(command) if command.code == AUTHENTICATION_FAILED => {
            OxynError::Authentication(format!(
                "the server refused the credentials (code {AUTHENTICATION_FAILED})"
            ))
        }
        ErrorKind::Command(command) => {
            let class = class_of_code(command.code, intent, &labels);
            let mut message = if carries_credential {
                format!("{WITHHELD_CREDENTIAL} (code {})", command.code)
            } else {
                format!(
                    "{} ({}): {}",
                    command.code_name, command.code, command.message
                )
            };
            if let Some(remedy) = remedy(command.code) {
                message.push_str(" — ");
                message.push_str(remedy);
            }
            let source = MongoError {
                message,
                code: Some(command.code),
                code_name: (!carries_credential).then(|| command.code_name.clone()),
                labels,
            };
            if class == ErrorClass::Permanent {
                OxynError::Query(source.message)
            } else {
                OxynError::driver(driver.clone(), class, source)
            }
        }
        ErrorKind::ServerSelection { .. } => OxynError::Connection(
            "no server suitable for the operation answered in time: nothing was sent".to_owned(),
        ),
        ErrorKind::DnsResolve { .. } => {
            OxynError::Connection("the DNS seed list (`srv`) could not be resolved".to_owned())
        }
        ErrorKind::InvalidTlsConfig { .. } => OxynError::Config(
            "the TLS configuration is unusable: check the CA and certificate files".to_owned(),
        ),
        // The library's text can quote a DNS seed list's TXT options: not
        // shown, like every topology message.
        ErrorKind::InvalidArgument { .. } => OxynError::Query(
            "the MongoDB client refused an option of the command or of the connection as invalid"
                .to_owned(),
        ),
        ErrorKind::Io(io) => transport(driver, intent, &format!("{:?}", io.kind()), labels),
        ErrorKind::ConnectionPoolCleared { .. } => {
            transport(driver, intent, "connection pool cleared", labels)
        }
        ErrorKind::InvalidResponse { .. } | ErrorKind::Bson(_) => {
            let class = if intent.is_read_only() {
                ErrorClass::Permanent
            } else {
                ErrorClass::Ambiguous
            };
            OxynError::driver(
                driver.clone(),
                class,
                MongoError {
                    message: "the server sent a reply the client could not read".to_owned(),
                    code: None,
                    code_name: None,
                    labels,
                },
            )
        }
        ErrorKind::IncompatibleServer { .. } => OxynError::Connection(
            "the server's wire version is outside what the client supports".to_owned(),
        ),
        _ => OxynError::driver(
            driver.clone(),
            if intent.is_read_only() {
                ErrorClass::Permanent
            } else {
                ErrorClass::Ambiguous
            },
            MongoError {
                message: "the MongoDB client failed".to_owned(),
                code: server_code(error),
                code_name: None,
                labels,
            },
        ),
    }
}

/// A transport cut: transient for a read, ambiguous otherwise.
fn transport(
    driver: &DriverId,
    intent: StatementIntent,
    what: &str,
    labels: Vec<String>,
) -> OxynError {
    if intent.is_read_only() {
        OxynError::driver(
            driver.clone(),
            ErrorClass::Transient,
            MongoError {
                message: format!("the connection to the server failed during a read ({what})"),
                code: None,
                code_name: None,
                labels,
            },
        )
    } else {
        OxynError::OutcomeUnknown(format!(
            "the connection failed after the command was sent ({what}): the server may have \
             applied it, entirely or in part"
        ))
    }
}

/// The verdict of a write whose reply says `ok: 1` with a
/// `writeConcernError`: applied on the primary, not confirmed where the
/// concern asked. A failover may roll it back; a replay may apply it twice.
/// `None` when the reply holds none.
pub(crate) fn write_concern_failure(
    reply: &bson::RawDocument,
    withheld: bool,
) -> Option<OxynError> {
    let concern = reply.get_document("writeConcernError").ok()?;
    Some(OxynError::OutcomeUnknown(format!(
        "the write was applied but its write concern was not satisfied ({}): it may be rolled \
         back by a failover",
        concern
            .get_str("codeName")
            .ok()
            .filter(|_| !withheld)
            .unwrap_or("writeConcernError")
    )))
}

/// The verdict of a command the user stopped.
pub(crate) fn stopped(intent: StatementIntent) -> OxynError {
    if intent.is_read_only() {
        OxynError::Cancelled
    } else {
        OxynError::OutcomeUnknown(
            "the command was stopped on the server, after it was sent: a multi-document write \
             keeps the documents it changed before the interruption"
                .to_owned(),
        )
    }
}

#[cfg(test)]
mod tests;
