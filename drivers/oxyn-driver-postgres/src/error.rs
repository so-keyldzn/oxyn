//! Classifying an `sqlx` error, and above all: not classifying it transient
//! when it is ambiguous.
//!
//! [DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md) gives three families.
//! The only costly one is the **ambiguous** one: a cut during an `INSERT` does
//! not say whether the server applied the write. Classified transient, it is
//! replayed by a conscientious caller, and creates a duplicate nobody will ever
//! see in an error message ([I-13](../../../CLAUDE.md#i-13)).
//!
//! **That is why classification takes the intent as a parameter.** The same
//! network cut is transient during a `SELECT` — nothing can have changed — and
//! ambiguous during an `UPDATE`. A classification that ignored the intent would
//! have to choose, and choosing "transient" is the wrong side.
//!
//! # The server's message quotes what it was bound
//!
//! `invalid input syntax for type integer: "…"` on a mistyped `$1`, a `RAISE`
//! that concatenates `NEW.column`: PostgreSQL copies a bound value into its
//! primary message. That message is displayed, logged and **persisted** by the
//! history and the journal. It is therefore propagated only if the statement
//! carried no value bound by the caller — see [`Bound`].
//!
//! # What this module does not do
//!
//! It retries nothing. The retry policy belongs to the caller, the only one who
//! knows whether the operation can be replayed
//! ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)).
//!
//! It does not **filter** the text of the values in the message either: the
//! server truncates, escapes and transforms them, so a `replace` would look
//! like a protection without being one.

use oxyn_core::{DriverId, ErrorClass, OxynError, ScalarValue, StatementIntent};

/// `query_canceled`: the server confirms the cancellation it was asked for.
const SQLSTATE_QUERY_CANCELED: &str = "57014";
/// `admin_shutdown`: the connection was cut by the administrator.
const SQLSTATE_ADMIN_SHUTDOWN: &str = "57P01";
/// `crash_shutdown`.
const SQLSTATE_CRASH_SHUTDOWN: &str = "57P02";
/// `cannot_connect_now`: the server is still starting.
const SQLSTATE_CANNOT_CONNECT_NOW: &str = "57P03";
/// `idle_session_timeout` / `idle_in_transaction_session_timeout`.
const SQLSTATE_IDLE_TIMEOUT: &str = "57P05";
/// `serialization_failure`.
const SQLSTATE_SERIALIZATION_FAILURE: &str = "40001";
/// `deadlock_detected`.
const SQLSTATE_DEADLOCK: &str = "40P01";
/// `lock_not_available`.
const SQLSTATE_LOCK_NOT_AVAILABLE: &str = "55P03";
/// `too_many_connections`.
const SQLSTATE_TOO_MANY_CONNECTIONS: &str = "53300";
/// `configuration_limit_exceeded`.
const SQLSTATE_CONFIG_LIMIT: &str = "53400";
/// `out_of_memory` on the server.
const SQLSTATE_OUT_OF_MEMORY: &str = "53200";
/// `disk_full` on the server.
const SQLSTATE_DISK_FULL: &str = "53100";
/// Class `08`: `connection_exception`.
const SQLSTATE_CLASS_CONNECTION: &str = "08";
/// Class `28`: `invalid_authorization_specification`.
const SQLSTATE_CLASS_AUTHORIZATION: &str = "28";
/// Class `53`: `insufficient_resources`.
const SQLSTATE_CLASS_RESOURCES: &str = "53";
/// `read_only_sql_transaction`: the read-only transaction did its job.
const SQLSTATE_READ_ONLY_TRANSACTION: &str = "25006";

/// Where the values bound to the statement concerned come from.
///
/// The server's primary message can quote a bound value; it is therefore
/// propagated only when nothing it can quote comes from the caller
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    /// No value coming from an [`ExecRequest`](oxyn_core::ExecRequest). The
    /// introspection queries do bind identifiers — a schema name, a relation
    /// name —, but those come from the catalog already displayed. The property
    /// held is "nothing the server can quote comes from the caller", not
    /// "nothing was bound".
    Internal,
    /// At least one value of the caller's
    /// [`ExecRequest`](oxyn_core::ExecRequest) was bound.
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
///
/// Explicit rather than reassuring: without this sentence, the user would
/// believe in a silent error and look for a defect in their SQL.
const WITHHELD_MESSAGE: &str = "the server message is withheld because the statement carried \
                                bound values, which PostgreSQL can quote verbatim";

/// The driver error as wrapped in [`OxynError::Driver`].
///
/// A named type rather than an anonymous `Box<dyn Error>`: the caller who wants
/// the SQLSTATE can read it through `downcast_ref`, without parsing a message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PostgresError {
    /// What will be shown, logged and persisted.
    ///
    /// It is the server's message — or the transport's — **only** if the
    /// statement carried no value bound by the caller. Otherwise, it is
    /// [`WITHHELD_MESSAGE`] followed by the SQLSTATE: the server copies a bound
    /// value into its primary message, and this field ends up in the history
    /// and the journal ([I-03](../../../CLAUDE.md#i-03)).
    ///
    /// The original `sqlx` error is never kept alongside: `#[derive(Debug)]`
    /// would re-expose it to the first `tracing::debug!` that comes along.
    message: String,
    /// The SQLSTATE, when the error comes from the server.
    sqlstate: Option<String>,
}

impl PostgresError {
    /// The five-character SQLSTATE, when the server gave one.
    ///
    /// Always present, including when the message is withheld: it is a code, it
    /// quotes nothing.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    /// The message as it will be shown.
    ///
    /// "The audience reads PostgreSQL error messages": the server's is never
    /// paraphrased. It is however **withheld**, and says so, when the statement
    /// carried bound values.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Translates an `sqlx` error that occurred on a statement **composed by the
/// driver**.
///
/// The server's message is propagated as is: such a statement binds only
/// literals the driver wrote. For the execution of an
/// [`ExecRequest`](oxyn_core::ExecRequest), go through [`map_stream_error`],
/// which takes stock of the bound values ([I-03](../../../CLAUDE.md#i-03)).
///
/// `intent` decides the fate of transport errors: ambiguous if the statement
/// could write, transient otherwise. When in doubt —
/// [`StatementIntent::Unknown`] — the statement counts as mutating, as
/// everywhere else.
#[must_use]
pub(crate) fn map_exec_error(
    driver: &DriverId,
    intent: StatementIntent,
    err: sqlx::Error,
) -> OxynError {
    map_bound_error(driver, intent, Bound::Internal, err)
}

/// Translates an `sqlx` error that occurred **during an execution**, knowing
/// whether the statement carried values bound by the caller.
///
/// `bound` changes **only** the message: the family is read from the SQLSTATE
/// and the intent, both taken from the original error.
#[must_use]
fn map_bound_error(
    driver: &DriverId,
    intent: StatementIntent,
    bound: Bound,
    err: sqlx::Error,
) -> OxynError {
    // A cancellation confirmed by the server is not a failure: it is the
    // "Cancel" button that worked.
    if sqlstate_of(&err).as_deref() == Some(SQLSTATE_QUERY_CANCELED) {
        return OxynError::Cancelled;
    }
    if let Some(error) = as_config_error(&err) {
        return error;
    }
    let class = classify(&err, intent);
    OxynError::driver(driver.clone(), class, wrap(err, bound))
}

/// Translates an `sqlx` error that occurred **while opening a session**.
///
/// No write can have happened: nothing is ambiguous here. On the other hand, a
/// credentials refusal must be distinguished from an unreachable server — one
/// is fixed in the form, the other is not.
#[must_use]
pub(crate) fn map_connect_error(err: &sqlx::Error) -> OxynError {
    if let Some(error) = as_config_error(err) {
        return error;
    }
    match sqlstate_of(err) {
        Some(code) if code.starts_with(SQLSTATE_CLASS_AUTHORIZATION) => {
            OxynError::Authentication(message_of(err))
        }
        // `invalid_catalog_name`: the database does not exist. It is a
        // configuration error, not a credentials refusal.
        Some(code) if code == "3D000" => OxynError::Config(message_of(err)),
        _ => OxynError::Connection(message_of(err)),
    }
}

/// The family of an `sqlx` error, in the sense of [`ErrorClass`].
#[must_use]
pub(crate) fn classify(err: &sqlx::Error, intent: StatementIntent) -> ErrorClass {
    // A timeout or a cut during a statement that could write leaves the effect
    // unknown. It is the only place in the driver where the intent changes a
    // decision.
    let transport = if intent.is_mutating() {
        ErrorClass::Ambiguous
    } else {
        ErrorClass::Transient
    };

    match err {
        // The server spoke: its SQLSTATE is authoritative.
        sqlx::Error::Database(_) => classify_sqlstate(sqlstate_of(err).as_deref(), transport),

        // The transport gave way. What was sent may have been executed.
        sqlx::Error::Io(_) | sqlx::Error::Protocol(_) | sqlx::Error::WorkerCrashed => transport,

        // Nothing was sent: acquiring a connection failed before.
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => ErrorClass::Transient,

        // Everything else comes from what was asked: replaying would give the
        // same result.
        _ => ErrorClass::Permanent,
    }
}

/// The family associated with an SQLSTATE.
fn classify_sqlstate(code: Option<&str>, transport: ErrorClass) -> ErrorClass {
    let Some(code) = code else {
        return ErrorClass::Permanent;
    };
    match code {
        // Concurrency conflicts: replaying is exactly the right answer.
        SQLSTATE_SERIALIZATION_FAILURE
        | SQLSTATE_DEADLOCK
        | SQLSTATE_LOCK_NOT_AVAILABLE
        | SQLSTATE_TOO_MANY_CONNECTIONS
        | SQLSTATE_CONFIG_LIMIT
        | SQLSTATE_CANNOT_CONNECT_NOW
        | SQLSTATE_IDLE_TIMEOUT
        | SQLSTATE_OUT_OF_MEMORY
        | SQLSTATE_DISK_FULL => ErrorClass::Transient,

        // The server stopped midway: like a cut.
        SQLSTATE_ADMIN_SHUTDOWN | SQLSTATE_CRASH_SHUTDOWN => transport,

        other if other.starts_with(SQLSTATE_CLASS_CONNECTION) => transport,
        other if other.starts_with(SQLSTATE_CLASS_RESOURCES) => ErrorClass::Transient,

        // Syntax, missing object, violated constraint, rights, read-only
        // transaction: display it, do not retry.
        _ => ErrorClass::Permanent,
    }
}

/// An error that belongs to configuration rather than to the server.
fn as_config_error(err: &sqlx::Error) -> Option<OxynError> {
    match err {
        sqlx::Error::Configuration(_) => Some(OxynError::Config(message_of(err))),
        // A TLS failure is a configuration or trust problem, not a network
        // failure to retry in a loop.
        sqlx::Error::Tls(_) => Some(OxynError::Connection(message_of(err))),
        _ => None,
    }
}

/// The SQLSTATE carried by the error, when it comes from the server.
#[must_use]
pub(crate) fn sqlstate_of(err: &sqlx::Error) -> Option<String> {
    match err {
        sqlx::Error::Database(base) => base.code().map(|code| code.into_owned()),
        _ => None,
    }
}

/// The message of an error, preferring the server's to ours.
fn message_of(err: &sqlx::Error) -> String {
    match err {
        sqlx::Error::Database(base) => base.message().to_owned(),
        other => other.to_string(),
    }
}

/// Wraps the `sqlx` error in the driver's named type.
///
/// Only two variants are withheld when the caller had bound values: the
/// **server's**, whose primary message copies them, and the **encoder's**,
/// which is composed from the value it could not encode. The others — cut, TLS,
/// exhausted pool — are written by `sqlx` from the transport, without ever
/// pouring the arguments into them: withholding them would cost a diagnostic
/// without protecting anything.
fn wrap(err: sqlx::Error, bound: Bound) -> PostgresError {
    let sqlstate = sqlstate_of(&err);
    let message = match (bound, &err) {
        (Bound::Caller, sqlx::Error::Database(_) | sqlx::Error::Encode(_)) => {
            withheld(sqlstate.as_deref())
        }
        _ => message_of(&err),
    };
    PostgresError { message, sqlstate }
}

/// The replacement message, which keeps the only safe identifier: the SQLSTATE.
fn withheld(sqlstate: Option<&str>) -> String {
    match sqlstate {
        Some(code) => format!("{WITHHELD_MESSAGE} (SQLSTATE {code})"),
        None => WITHHELD_MESSAGE.to_owned(),
    }
}

/// Is the error that of a read-only transaction having refused a write?
#[must_use]
pub(crate) fn is_read_only_rejection(err: &sqlx::Error) -> bool {
    sqlstate_of(err).as_deref() == Some(SQLSTATE_READ_ONLY_TRANSACTION)
}

/// What the user is told when their execution bounds refused a write.
///
/// The message exists so that nobody concludes to a rights defect on their
/// database: Oxyn asked for read-only, not the administrator.
const READ_ONLY_MESSAGE: &str = "this execution is bounded to read-only: \
                                 the server rejected a statement that writes";

/// Did `prepare` fail because `sqlx` could not describe a result type?
///
/// After Describe, `sqlx` 0.9 reads `pg_type` for every OID it does not know
/// and refuses `typtype = 'm'` (multiranges) and `typcategory = 'Z'`
/// (`pg_node_tree`…): the error names the `pg_type` column it could not decode.
/// Nothing ran on the server (ADR-0048).
pub(crate) fn is_unresolvable_type(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::ColumnDecode { index, .. }
            if index.contains("typtype") || index.contains("typcategory")
    )
}

/// Translates an execution error taking into account the requested bounds and
/// the bound values.
///
/// A single place decides this message, because it is produced on two paths —
/// preparation and streaming — and two wordings would diverge.
///
/// It is the gateway of the **caller's** statement: `bound` is mandatory there
/// so that neither of these two paths can forget it.
#[must_use]
pub(crate) fn map_stream_error(
    driver: &DriverId,
    intent: StatementIntent,
    read_only: bool,
    bound: Bound,
    err: sqlx::Error,
) -> OxynError {
    if read_only && is_read_only_rejection(&err) {
        return OxynError::Query(READ_ONLY_MESSAGE.to_owned());
    }
    map_bound_error(driver, intent, bound, err)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the test implements this trait: at the module root, it would be an
    // unused import in the `lib` target.
    use sqlx::error::DatabaseError;

    /// A fake database error carrying a given SQLSTATE.
    ///
    /// `sqlx` does not allow building a `PgDatabaseError` from outside; the
    /// trait is therefore implemented, which is enough for `classify`.
    #[derive(Debug)]
    struct FakeDatabase {
        code: &'static str,
        message: &'static str,
    }

    impl std::fmt::Display for FakeDatabase {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for FakeDatabase {}

    impl DatabaseError for FakeDatabase {
        fn message(&self) -> &str {
            self.message
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(std::borrow::Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn server_error(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDatabase {
            code,
            message: "fake error",
        }))
    }

    fn cut() -> sqlx::Error {
        sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
    }

    #[test]
    fn a_cut_during_a_write_is_ambiguous() {
        // It is the most important test of the module: classified transient,
        // this error produces a silent duplicate in the user's data (I-13).
        assert_eq!(
            classify(&cut(), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
        assert!(!ErrorClass::Ambiguous.is_retryable());
    }

    #[test]
    fn a_cut_during_a_read_is_transient() {
        assert_eq!(
            classify(&cut(), StatementIntent::Read),
            ErrorClass::Transient
        );
    }

    #[test]
    fn an_unknown_intent_counts_as_mutating() {
        // `ExecRequest`'s default: a statement no analyzer could classify may
        // write.
        assert_eq!(
            classify(&cut(), StatementIntent::Unknown),
            ErrorClass::Ambiguous
        );
    }

    #[test]
    fn a_deadlock_is_retried() {
        assert_eq!(
            classify(&server_error(SQLSTATE_DEADLOCK), StatementIntent::Write),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(
                &server_error(SQLSTATE_SERIALIZATION_FAILURE),
                StatementIntent::Write
            ),
            ErrorClass::Transient
        );
    }

    #[test]
    fn a_syntax_error_is_never_retried() {
        // 42601: syntax_error.
        assert_eq!(
            classify(&server_error("42601"), StatementIntent::Read),
            ErrorClass::Permanent
        );
        // 42P01: undefined_table.
        assert_eq!(
            classify(&server_error("42P01"), StatementIntent::Read),
            ErrorClass::Permanent
        );
        // 23505: unique_violation.
        assert_eq!(
            classify(&server_error("23505"), StatementIntent::Write),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn a_cut_announced_by_the_server_follows_the_intent() {
        // 08006: connection_failure.
        assert_eq!(
            classify(&server_error("08006"), StatementIntent::Read),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(&server_error("08006"), StatementIntent::Write),
            ErrorClass::Ambiguous
        );
    }

    #[test]
    fn an_expired_connection_wait_stays_transient_even_for_a_write() {
        // Nothing was sent: acquisition failed before the query.
        assert_eq!(
            classify(&sqlx::Error::PoolTimedOut, StatementIntent::Write),
            ErrorClass::Transient
        );
    }

    #[test]
    fn a_cancellation_confirmed_by_the_server_is_not_a_failure() {
        let error = map_exec_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            server_error(SQLSTATE_QUERY_CANCELED),
        );
        assert!(error.is_cancelled(), "{error:?}");
    }

    #[test]
    fn a_credentials_refusal_is_distinguished_from_an_unreachable_server() {
        // 28P01: invalid_password. One is fixed in the form.
        let refusal = map_connect_error(&server_error("28P01"));
        assert!(
            matches!(refusal, OxynError::Authentication(_)),
            "{refusal:?}"
        );
        assert!(refusal.is_user_error());

        let unreachable = map_connect_error(&cut());
        assert!(
            matches!(unreachable, OxynError::Connection(_)),
            "{unreachable:?}"
        );
        assert!(unreachable.is_retryable());
    }

    #[test]
    fn a_missing_database_is_a_configuration_error() {
        let error = map_connect_error(&server_error("3D000"));
        assert!(matches!(error, OxynError::Config(_)), "{error:?}");
        assert!(
            !error.is_retryable(),
            "replaying will not create the database"
        );
    }

    #[test]
    fn the_class_crosses_the_wrapping_up_to_the_caller() {
        // The caller reads `ErrorClass`, it does not re-read the message.
        let error = map_exec_error(&DriverId::postgres(), StatementIntent::Write, cut());
        assert_eq!(error.class(), ErrorClass::Ambiguous);
        assert!(!error.is_retryable());
    }

    #[test]
    fn the_sqlstate_stays_readable_without_parsing_the_message() {
        let error = map_exec_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            server_error("42P01"),
        );
        let OxynError::Driver { source, .. } = &error else {
            panic!("expected a driver error: {error:?}");
        };
        let postgres = source
            .downcast_ref::<PostgresError>()
            .expect("the driver wraps its errors in PostgresError");
        assert_eq!(postgres.sqlstate(), Some("42P01"));
        assert_eq!(postgres.message(), "fake error");
    }

    #[test]
    fn a_read_only_transaction_refusal_is_recognized() {
        assert!(is_read_only_rejection(&server_error(
            SQLSTATE_READ_ONLY_TRANSACTION
        )));
        assert!(!is_read_only_rejection(&server_error("42601")));
    }

    #[test]
    fn a_write_refused_by_the_bounds_names_the_bounds_not_the_rights() {
        // Without this message, the user concludes to a rights defect on their
        // database and goes to see their administrator.
        let error = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            true,
            Bound::Internal,
            server_error(SQLSTATE_READ_ONLY_TRANSACTION),
        );
        assert!(matches!(error, OxynError::Query(_)), "{error:?}");
        assert!(error.to_string().contains("read-only"), "{error}");
        assert!(error.is_user_error());
    }

    #[test]
    fn the_same_refusal_outside_the_bounds_stays_a_server_error() {
        // The database can be read-only for its own reasons — a standby, a
        // `default_transaction_read_only`. It is then not for Oxyn to claim the
        // refusal.
        let error = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            false,
            Bound::Internal,
            server_error(SQLSTATE_READ_ONLY_TRANSACTION),
        );
        assert!(matches!(error, OxynError::Driver { .. }), "{error:?}");
    }

    /// A server error whose primary message quotes a bound value, as an
    /// invalid cast of `$1` does.
    fn chatty_error() -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDatabase {
            code: "22P02",
            message: "invalid input syntax for type integer: \"S3NT1N3L-42\"",
        }))
    }

    #[test]
    fn the_server_message_is_withheld_as_soon_as_a_value_was_bound() {
        // I-03: this message is displayed, and persisted by
        // `HistoryRecord::failed` and `JournalRecord::failed`, which call
        // `error.to_string()`.
        let error = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Caller,
            chatty_error(),
        );
        for rendered in [format!("{error}"), format!("{error:?}")] {
            assert!(!rendered.contains("S3NT1N3L-42"), "bound value: {rendered}");
            assert!(!rendered.contains("invalid input syntax"), "{rendered}");
        }
        assert!(error.to_string().contains("withheld"), "{error}");
        // The SQLSTATE survives: it is a code, it quotes nothing.
        assert!(error.to_string().contains("22P02"), "{error}");
        let OxynError::Driver { source, .. } = &error else {
            panic!("expected a driver error: {error:?}");
        };
        let postgres = source
            .downcast_ref::<PostgresError>()
            .expect("the driver wraps its errors in PostgresError");
        assert_eq!(postgres.sqlstate(), Some("22P02"));
    }

    #[test]
    fn without_bound_value_the_server_message_passes_unchanged() {
        // Oxyn's audience reads PostgreSQL's messages: a reassuring paraphrase
        // would be a defect.
        let error = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Internal,
            chatty_error(),
        );
        assert!(
            error
                .to_string()
                .contains("invalid input syntax for type integer"),
            "{error}"
        );
    }

    #[test]
    fn withholding_the_message_changes_neither_the_family_nor_the_cancellation() {
        // The family is read from the SQLSTATE and the intent, not from the
        // message.
        let cut_while_writing = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            false,
            Bound::Caller,
            cut(),
        );
        assert_eq!(cut_while_writing.class(), ErrorClass::Ambiguous);
        assert!(!cut_while_writing.is_retryable(), "I-13");

        let deadlock = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Write,
            false,
            Bound::Caller,
            server_error(SQLSTATE_DEADLOCK),
        );
        assert_eq!(deadlock.class(), ErrorClass::Transient);

        let cancelled = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Caller,
            server_error(SQLSTATE_QUERY_CANCELED),
        );
        assert!(cancelled.is_cancelled(), "{cancelled:?}");
    }

    #[test]
    fn an_encoder_error_is_withheld_like_the_server_s() {
        // The encoder composes its message from the value it refused.
        let error = map_stream_error(
            &DriverId::postgres(),
            StatementIntent::Read,
            false,
            Bound::Caller,
            sqlx::Error::Encode("`S3NT1N3L-42` is out of range".into()),
        );
        assert!(!error.to_string().contains("S3NT1N3L-42"), "{error}");
        assert!(error.to_string().contains("withheld"), "{error}");
    }

    #[test]
    fn a_request_without_parameters_withholds_nothing() {
        assert_eq!(Bound::of(&[]), Bound::Internal);
        assert_eq!(
            Bound::of(&[oxyn_core::ScalarValue::Text("S3NT1N3L-42".to_owned())]),
            Bound::Caller
        );
    }
}
