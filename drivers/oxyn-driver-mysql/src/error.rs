//! Classifying a `mysql_async` error, and above all: not classifying it
//! transient when it is ambiguous.
//!
//! [DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md) gives three families.
//! The costly one is the **ambiguous** one: a cut during an `INSERT` does not
//! say whether the server applied the write. Classified transient, it is
//! replayed by a conscientious caller and creates a duplicate nobody sees
//! ([I-13](../../../CLAUDE.md#i-13)). That is why classification takes the
//! intent: the same cut is transient during a `SELECT`, ambiguous during an
//! `UPDATE`.
//!
//! # The server's message quotes what it was bound
//!
//! `Incorrect integer value: 'abc' for column 'n' at row 1` (1366) copies a
//! bound value into the message, which is displayed, logged and persisted by
//! the history. It is therefore propagated only when the statement carried no
//! value bound by the caller — see [`Bound`]; otherwise the error code and the
//! SQLSTATE stand in for it.
//!
//! # What this module does not do
//!
//! It retries nothing: the retry policy belongs to the caller, the only one who
//! knows whether the operation can be replayed.

use mysql_async::{DriverError, Error as MyError, IoError};
use oxyn_core::{DriverId, ErrorClass, OxynError, ScalarValue, StatementIntent};

/// `ER_CON_COUNT_ERROR`: too many connections.
const ER_CON_COUNT_ERROR: u16 = 1040;
/// `ER_OUT_OF_RESOURCES`.
const ER_OUT_OF_RESOURCES: u16 = 1041;
/// `ER_OUTOFMEMORY`.
const ER_OUTOFMEMORY: u16 = 1037;
/// `ER_OUT_OF_SORTMEMORY`.
const ER_OUT_OF_SORTMEMORY: u16 = 1038;
/// `ER_DISK_FULL`.
const ER_DISK_FULL: u16 = 1021;
/// `ER_DBACCESS_DENIED_ERROR`: the account may not use the database.
const ER_DBACCESS_DENIED_ERROR: u16 = 1044;
/// `ER_ACCESS_DENIED_ERROR`: the credentials are refused.
const ER_ACCESS_DENIED_ERROR: u16 = 1045;
/// `ER_BAD_DB_ERROR`: the database does not exist.
const ER_BAD_DB_ERROR: u16 = 1049;
/// `ER_SERVER_SHUTDOWN`: the server is going down.
const ER_SERVER_SHUTDOWN: u16 = 1053;
/// `ER_TOO_MANY_USER_CONNECTIONS`.
const ER_TOO_MANY_USER_CONNECTIONS: u16 = 1203;
/// `ER_LOCK_WAIT_TIMEOUT`: the statement waited for a lock too long.
const ER_LOCK_WAIT_TIMEOUT: u16 = 1205;
/// `ER_LOCK_DEADLOCK`: the transaction was chosen as the deadlock victim.
const ER_LOCK_DEADLOCK: u16 = 1213;
/// `ER_USER_LIMIT_REACHED`.
const ER_USER_LIMIT_REACHED: u16 = 1226;
/// `ER_UNSUPPORTED_PS`: the statement cannot be prepared (ADR-0050 §3).
pub(crate) const ER_UNSUPPORTED_PS: u16 = 1295;
/// `ER_QUERY_INTERRUPTED`: a `KILL QUERY` stopped the statement.
pub(crate) const ER_QUERY_INTERRUPTED: u16 = 1317;
/// `ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION`.
const ER_READ_ONLY_TRANSACTION: u16 = 1792;
/// MariaDB's `ER_CONNECTION_KILLED`.
const ER_CONNECTION_KILLED: u16 = 1927;
/// The SQLSTATE class of connection exceptions.
const SQLSTATE_CLASS_CONNECTION: &str = "08";
/// `serialization_failure`, which MySQL gives a deadlock.
const SQLSTATE_SERIALIZATION_FAILURE: &str = "40001";

/// Where the values bound to the statement concerned come from.
///
/// The server's message can quote a bound value; it is propagated only when
/// nothing it can quote comes from the caller ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    /// No value of an [`ExecRequest`](oxyn_core::ExecRequest). Introspection
    /// binds names taken from the catalog already displayed: nothing the
    /// server can quote comes from the caller.
    Internal,
    /// At least one value of the caller's request was bound.
    Caller,
}

impl Bound {
    /// What a request's parameters imply for the server's message.
    pub(crate) const fn of(params: &[ScalarValue]) -> Self {
        if params.is_empty() {
            Self::Internal
        } else {
            Self::Caller
        }
    }
}

/// What replaces the server's message when the statement carried bound values.
const WITHHELD_MESSAGE: &str = "the server message is withheld because the statement carried \
                                bound values, which MySQL can quote verbatim";

/// What the user is told when their execution bounds refused a write.
const READ_ONLY_MESSAGE: &str = "this execution is bounded to read-only: \
                                 the server rejected a statement that writes";

/// What a `LOAD DATA LOCAL INFILE` request from the server becomes.
///
/// The session has no file handler, on purpose (ADR-0050 §1): the server asked
/// for a file of this machine, nothing was sent, and `mysql_async` closed the
/// connection.
const LOCAL_INFILE_REFUSED: &str = "the server asked for a local file (`LOAD DATA LOCAL \
                                    INFILE`): Oxyn never sends files of this machine, nothing \
                                    was loaded, and the connection was closed";

/// The driver error as wrapped in [`OxynError::Driver`].
///
/// A named type rather than a `Box<dyn Error>`: the caller who wants the error
/// code reads it through `downcast_ref`, without parsing a message. The
/// original `mysql_async` error is never kept alongside: its `Debug` would
/// re-expose a withheld message to the first `tracing::debug!` that comes
/// along.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct MysqlError {
    /// What will be shown, logged and persisted: the server's message, or
    /// [`WITHHELD_MESSAGE`] with the code when the caller had bound values.
    message: String,
    /// The server's error number, when the error comes from the server.
    code: Option<u16>,
    /// The SQLSTATE, when the error comes from the server.
    sqlstate: Option<String>,
}

impl MysqlError {
    /// The server's error number (`1064`, `1792`…), when the server gave one.
    #[must_use]
    pub const fn code(&self) -> Option<u16> {
        self.code
    }

    /// The five-character SQLSTATE, when the server gave one.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    /// The message as it will be shown.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// An error the driver itself raises about what the server sent.
    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            sqlstate: None,
        }
    }
}

/// The server's error number, when the error comes from the server.
#[must_use]
pub(crate) const fn server_code(err: &MyError) -> Option<u16> {
    match err {
        MyError::Server(server) => Some(server.code),
        _ => None,
    }
}

/// Did the server refuse the text because it cannot prepare it (1295)?
#[must_use]
pub(crate) const fn is_unsupported_prepare(err: &MyError) -> bool {
    matches!(server_code(err), Some(ER_UNSUPPORTED_PS))
}

/// Is it the `KILL QUERY` the driver sent, confirmed by the server?
#[must_use]
pub(crate) const fn is_query_interrupted(err: &MyError) -> bool {
    matches!(server_code(err), Some(ER_QUERY_INTERRUPTED))
}

/// Did the server refuse the connection's `LOAD DATA LOCAL INFILE` request?
fn is_local_infile(err: &MyError) -> bool {
    matches!(err, MyError::Driver(DriverError::LocalInfile(_)))
}

/// Does the error mean the connection itself is gone or unusable?
///
/// The session then forgets it and opens a fresh one on the next operation;
/// what the server held on it — an open transaction first of all — is lost.
#[must_use]
pub(crate) fn breaks_connection(err: &MyError) -> bool {
    match err {
        MyError::Io(_) => true,
        MyError::Driver(
            DriverError::ConnectionClosed
            | DriverError::PacketOutOfOrder
            | DriverError::UnexpectedPacket { .. }
            | DriverError::PacketTooLarge
            | DriverError::BadCompressedPacketHeader
            | DriverError::LocalInfile(_),
        ) => true,
        MyError::Server(server) => {
            matches!(server.code, ER_SERVER_SHUTDOWN | ER_CONNECTION_KILLED)
                || server.state.starts_with(SQLSTATE_CLASS_CONNECTION)
        }
        _ => false,
    }
}

/// Translates an error met while **opening** a connection.
///
/// No write can have happened: nothing is ambiguous. A credentials refusal is
/// told apart from an unreachable server — one is fixed in the form, the other
/// is not.
#[must_use]
pub(crate) fn map_connect_error(err: &MyError) -> OxynError {
    match err {
        MyError::Server(server) => match server.code {
            ER_ACCESS_DENIED_ERROR | ER_DBACCESS_DENIED_ERROR => {
                OxynError::Authentication(server.message.clone())
            }
            ER_BAD_DB_ERROR => OxynError::Config(server.message.clone()),
            _ => OxynError::Connection(server.message.clone()),
        },
        MyError::Url(url) => OxynError::Config(url.to_string()),
        MyError::Driver(DriverError::NoClientSslFlagFromServer) => OxynError::Connection(
            "the server does not offer TLS, and this connection requires it \
             (`sslmode` other than `verify-full` is only accepted on a connection \
             marked Local)"
                .to_owned(),
        ),
        MyError::Driver(DriverError::CleartextPluginDisabled) => OxynError::Authentication(
            "the server asks for the password in clear text (`mysql_clear_password`), \
             which Oxyn never sends"
                .to_owned(),
        ),
        MyError::Driver(driver) => OxynError::Connection(driver.to_string()),
        MyError::Io(IoError::Tls(tls)) => OxynError::Connection(format!("TLS: {tls}")),
        MyError::Io(IoError::Io(io)) => OxynError::Connection(io.to_string()),
        MyError::Other(other) => OxynError::Connection(other.to_string()),
    }
}

/// Translates an error on a statement **composed by the driver**, which binds
/// only values it wrote or read from the catalog.
#[must_use]
pub(crate) fn map_exec_error(
    driver: &DriverId,
    intent: StatementIntent,
    err: &MyError,
) -> OxynError {
    map_bound_error(driver, intent, Bound::Internal, err)
}

/// Translates an error met **during the execution** of the caller's request.
///
/// `read_only` turns the server's 1792 into the sentence that says Oxyn asked
/// for it; `bound` decides whether the server's message may be shown.
#[must_use]
pub(crate) fn map_stream_error(
    driver: &DriverId,
    intent: StatementIntent,
    read_only: bool,
    bound: Bound,
    err: &MyError,
) -> OxynError {
    if read_only && server_code(err) == Some(ER_READ_ONLY_TRANSACTION) {
        return OxynError::Query(READ_ONLY_MESSAGE.to_owned());
    }
    map_bound_error(driver, intent, bound, err)
}

fn map_bound_error(
    driver: &DriverId,
    intent: StatementIntent,
    bound: Bound,
    err: &MyError,
) -> OxynError {
    if is_local_infile(err) {
        // ADR-0050 §1: reported as a lost connection, which it is.
        return OxynError::Connection(LOCAL_INFILE_REFUSED.to_owned());
    }
    if let MyError::Url(url) = err {
        return OxynError::Config(url.to_string());
    }
    OxynError::driver(driver.clone(), classify(err, intent), wrap(err, bound))
}

/// The family of a `mysql_async` error, in the sense of [`ErrorClass`].
#[must_use]
pub(crate) fn classify(err: &MyError, intent: StatementIntent) -> ErrorClass {
    // A cut during a statement that could write leaves the effect unknown: the
    // only place where the intent changes a decision.
    let transport = if intent.is_mutating() {
        ErrorClass::Ambiguous
    } else {
        ErrorClass::Transient
    };
    match err {
        MyError::Server(server) => classify_server(server.code, &server.state, transport),
        MyError::Io(_) => transport,
        MyError::Driver(driver) => match driver {
            // The protocol lost its footing after the request left: whatever
            // was sent may have run.
            DriverError::ConnectionClosed
            | DriverError::PacketOutOfOrder
            | DriverError::UnexpectedPacket { .. }
            | DriverError::BadCompressedPacketHeader => transport,
            // Everything else comes from what was asked: replaying gives the
            // same result.
            _ => ErrorClass::Permanent,
        },
        MyError::Other(_) | MyError::Url(_) => ErrorClass::Permanent,
    }
}

/// The family of a server error number.
fn classify_server(code: u16, sqlstate: &str, transport: ErrorClass) -> ErrorClass {
    match code {
        // The statement was rolled back and nothing else: replaying is exactly
        // the right answer. A deadlock rolls the whole transaction back, which
        // the caller learns from the transaction state, not from here.
        ER_LOCK_WAIT_TIMEOUT
        | ER_LOCK_DEADLOCK
        | ER_CON_COUNT_ERROR
        | ER_TOO_MANY_USER_CONNECTIONS
        | ER_USER_LIMIT_REACHED
        | ER_OUT_OF_RESOURCES
        | ER_OUTOFMEMORY
        | ER_OUT_OF_SORTMEMORY
        | ER_DISK_FULL => ErrorClass::Transient,
        // The server stopped midway: like a cut.
        ER_SERVER_SHUTDOWN | ER_CONNECTION_KILLED => transport,
        _ if sqlstate.starts_with(SQLSTATE_CLASS_CONNECTION) => transport,
        _ if sqlstate == SQLSTATE_SERIALIZATION_FAILURE => ErrorClass::Transient,
        // Syntax, missing object, violated constraint, rights, read-only:
        // display it, do not retry.
        _ => ErrorClass::Permanent,
    }
}

/// Wraps the error in the driver's named type, withholding the server's
/// message when the caller bound values.
fn wrap(err: &MyError, bound: Bound) -> MysqlError {
    match err {
        MyError::Server(server) => {
            let message = match bound {
                Bound::Caller => format!(
                    "{WITHHELD_MESSAGE} (error {}, SQLSTATE {})",
                    server.code, server.state
                ),
                Bound::Internal => server.message.clone(),
            };
            MysqlError {
                message,
                code: Some(server.code),
                sqlstate: Some(server.state.clone()),
            }
        }
        // A parameter conversion error is composed from the value it could not
        // convert.
        MyError::Driver(DriverError::FromValue { .. } | DriverError::Params(_))
            if bound == Bound::Caller =>
        {
            MysqlError::protocol(WITHHELD_MESSAGE)
        }
        // Written by the library from the transport or the protocol, never from
        // the arguments: withholding them would cost a diagnostic for nothing.
        other => MysqlError::protocol(other.to_string()),
    }
}

/// The error of a local-infile request, for the tests that hold ADR-0050 §1.
#[cfg(test)]
pub(crate) fn local_infile_error() -> MyError {
    MyError::Driver(DriverError::LocalInfile(
        mysql_async::LocalInfileError::NoHandler,
    ))
}

#[cfg(test)]
mod tests {
    use mysql_async::ServerError;

    use super::*;

    fn server(code: u16, state: &str, message: &str) -> MyError {
        MyError::Server(ServerError {
            code,
            message: message.to_owned(),
            state: state.to_owned(),
        })
    }

    fn io_cut() -> MyError {
        MyError::Io(IoError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset",
        )))
    }

    #[test]
    fn a_cut_during_a_write_is_ambiguous_never_transient() {
        // I-13: replaying it would duplicate the write.
        assert_eq!(
            classify(&io_cut(), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
        assert_eq!(
            classify(&io_cut(), StatementIntent::Unknown),
            ErrorClass::Ambiguous,
            "an unknown intent counts as mutating"
        );
        assert_eq!(
            classify(&io_cut(), StatementIntent::Read),
            ErrorClass::Transient
        );
    }

    #[test]
    fn concurrency_conflicts_are_transient_and_syntax_is_permanent() {
        for code in [ER_LOCK_DEADLOCK, ER_LOCK_WAIT_TIMEOUT, ER_CON_COUNT_ERROR] {
            assert_eq!(
                classify(&server(code, "HY000", "x"), StatementIntent::Write),
                ErrorClass::Transient,
                "{code}"
            );
        }
        assert_eq!(
            classify(&server(1064, "42000", "syntax"), StatementIntent::Read),
            ErrorClass::Permanent
        );
        assert_eq!(
            classify(&server(1146, "42S02", "no table"), StatementIntent::Write),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn a_server_going_down_during_a_write_is_ambiguous() {
        assert_eq!(
            classify(
                &server(ER_SERVER_SHUTDOWN, "08S01", "down"),
                StatementIntent::Write
            ),
            ErrorClass::Ambiguous
        );
        assert!(breaks_connection(&server(
            ER_SERVER_SHUTDOWN,
            "08S01",
            "down"
        )));
    }

    #[test]
    fn a_bound_value_quoted_by_the_server_is_withheld() {
        // I-03: 1366 copies the value into its message.
        let secret = "4111111111111111";
        let err = server(
            1366,
            "HY000",
            &format!("Incorrect integer value: '{secret}' for column 'n' at row 1"),
        );
        let shown = map_stream_error(
            &DriverId::mysql(),
            StatementIntent::Write,
            false,
            Bound::Caller,
            &err,
        );
        let text = shown.to_string();
        assert!(!text.contains(secret), "leak: {text}");
        assert!(text.contains("1366"), "{text}");

        let internal = map_exec_error(&DriverId::mysql(), StatementIntent::Read, &err);
        assert!(internal.to_string().contains("Incorrect integer value"));
    }

    #[test]
    fn a_read_only_refusal_says_oxyn_asked_for_it() {
        let err = server(
            ER_READ_ONLY_TRANSACTION,
            "25006",
            "Cannot execute statement in a READ ONLY transaction.",
        );
        let shown = map_stream_error(
            &DriverId::mysql(),
            StatementIntent::Write,
            true,
            Bound::Internal,
            &err,
        );
        assert!(matches!(shown, OxynError::Query(_)), "{shown:?}");
        assert!(shown.to_string().contains("read-only"), "{shown}");
    }

    #[test]
    fn a_local_infile_request_is_reported_as_a_lost_connection() {
        // ADR-0050 §1.
        let err = local_infile_error();
        assert!(breaks_connection(&err));
        let shown = map_exec_error(&DriverId::mysql(), StatementIntent::Write, &err);
        assert!(matches!(shown, OxynError::Connection(_)), "{shown:?}");
        assert!(shown.to_string().contains("LOAD DATA LOCAL"), "{shown}");
    }

    #[test]
    fn connection_errors_tell_credentials_from_network() {
        assert!(matches!(
            map_connect_error(&server(ER_ACCESS_DENIED_ERROR, "28000", "denied")),
            OxynError::Authentication(_)
        ));
        assert!(matches!(
            map_connect_error(&server(ER_BAD_DB_ERROR, "42000", "unknown db")),
            OxynError::Config(_)
        ));
        assert!(matches!(
            map_connect_error(&io_cut()),
            OxynError::Connection(_)
        ));
    }

    #[test]
    fn the_fallback_opens_on_1295_only() {
        assert!(is_unsupported_prepare(&server(
            1295,
            "HY000",
            "unsupported"
        )));
        assert!(!is_unsupported_prepare(&server(1064, "42000", "syntax")));
        assert!(is_query_interrupted(&server(1317, "70100", "interrupted")));
    }
}
