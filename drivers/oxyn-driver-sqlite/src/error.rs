//! The driver's errors, and the **family** it attaches them to.
//!
//! [`DRIVER-CONTRACT` §4](../../../docs/DRIVER-CONTRACT.md) requires a driver to
//! classify its errors: transient, permanent, ambiguous. The class is **data**
//! carried by the error, never an inference the caller makes from the message.
//!
//! # The most expensive case
//!
//! `sqlite3_interrupt` during a write. SQLite guarantees atomicity at the
//! statement level, but not the restoration of an explicit transaction: a
//! statement interrupted **outside** a transaction is rolled back, an interrupted
//! transaction may leave the already applied statements in place. An
//! interruption is therefore returned as:
//!
//! * [`OxynError::Cancelled`] if the statement could modify nothing
//!   (`sqlite3_stmt_readonly`) — the effect is known: none;
//! * [`ErrorClass::Ambiguous`] otherwise. Ambiguity is **never** retried
//!   ([I-13](../../../CLAUDE.md#i-13)).
//!
//! # What never leaves this module
//!
//! No message repeats a **bound value** or the **database file path**: the path
//! is a connection parameter value, and a driver has no right to log it
//! ([I-03](../../../CLAUDE.md#i-03)). That is the reason for
//! [`SqliteError::Path`], which replaces `rusqlite::Error::InvalidPath` — the only
//! `rusqlite` variant whose `Display` contains the path.
//!
//! The engine's message, however, **quotes what was just bound**: a trigger
//! `RAISE(ABORT, 'balance: ' || NEW.amount)` or a violated `CHECK` constraint
//! repeat the value passed to `sqlite3_bind_*`. This message is displayed, logged
//! and persisted by the history. It is therefore propagated only if the
//! statement carried **no** value bound by the caller: otherwise, the extended
//! result code replaces the engine's text.

use oxyn_core::{DriverId, ErrorClass, OxynError, ScalarValue};
use rusqlite::ErrorCode;

/// What the statement concerned could do to the database.
///
/// Used only to decide whether an interruption is a clean cancellation or an
/// ambiguity. The value comes from `sqlite3_stmt_readonly`, not from an analysis
/// of the text: the engine answers, not us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    /// The statement could modify nothing.
    ReadOnly,
    /// The statement could write.
    Mutating,
}

/// Where the values bound to the statement concerned come from.
///
/// The engine's message may quote a bound value; it is therefore propagated only
/// when nothing it can quote comes from the caller
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    /// No value from an [`ExecRequest`](oxyn_core::ExecRequest). An
    /// introspection query does bind identifiers — a schema name, a table
    /// name —, but those come from the catalog already displayed, not from what
    /// the user typed. The property held is therefore "nothing the engine can
    /// quote comes from the caller", not "nothing was bound": writing it
    /// otherwise would make the audit wrong for the first reader who opened
    /// `catalog.rs`.
    Internal,
    /// At least one value of the caller's
    /// [`ExecRequest`](oxyn_core::ExecRequest) was bound.
    Caller,
}

impl Bound {
    /// What a request's parameters imply for the engine's message.
    pub(crate) const fn of(params: &[ScalarValue]) -> Self {
        if params.is_empty() {
            Self::Internal
        } else {
            Self::Caller
        }
    }
}

/// The result code of a failure, the only part of an engine message that cannot
/// quote a value.
///
/// `ffi::Error` renders "Error code 19: constraint failed": the label is
/// derived from the **number**, not from the text SQLite composed.
fn code_of(code: &Option<rusqlite::ffi::Error>) -> String {
    match code {
        Some(failure) => failure.to_string(),
        // The `rusqlite` variants that do not come from the engine (column
        // index, refused conversion) have no result code.
        None => "SQLite driver error".to_owned(),
    }
}

/// An error specific to the SQLite driver.
///
/// It travels as the `source` of [`OxynError::Driver`], which carries the family.
/// It is public because a caller may want to find it by `downcast_ref` — to tell
/// a type conflict from an engine error, for instance.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SqliteError {
    /// Error returned by the SQLite engine itself.
    ///
    /// SQLite's extended message is in it: it is produced only for a statement
    /// without a value bound by the caller, where it can quote only the
    /// submitted SQL. Otherwise, it is [`Withheld`](Self::Withheld).
    /// **No `#[from]`**: a `?` on a `rusqlite::Error` would build this
    /// unredacted variant without ever checking where the values come from. The
    /// first `?` added in `params.rs` — the function that binds the values —
    /// would thus skip the redaction without any reviewer seeing it.
    /// Construction therefore goes through the module's two translation
    /// functions, where the choice is made at writing time.
    #[error("{0}")]
    Engine(rusqlite::Error),

    /// The engine refused a statement that carried values bound by the caller:
    /// its message is withheld.
    ///
    /// Withheld and not filtered: SQLite may quote a bound value truncated,
    /// escaped or transformed — a `RAISE(ABORT, …)` trigger concatenates it, a
    /// `CHECK` constraint repeats it —, and searching for the value's text in
    /// the message would look like protection without being one
    /// ([I-03](../../../CLAUDE.md#i-03)). The original error is not kept in the
    /// variant either: `#[derive(Debug)]` would re-expose it to the first
    /// `tracing::debug!` that came along.
    ///
    /// Only the result code survives, which is a number.
    #[error(
        "{}: the SQLite message is withheld because the statement carried \
         bound values",
        code_of(.code)
    )]
    Withheld {
        /// The extended result code, when the failure comes from the engine.
        code: Option<rusqlite::ffi::Error>,
    },

    /// The database file path is not usable.
    ///
    /// The path **is not** repeated in the message: it is a connection parameter
    /// value (I-03).
    #[error("the database path is not usable")]
    Path,

    /// The thread holding the connection is gone: the session is closed, or its
    /// thread has ended.
    #[error("the connection thread is gone: the session is closed")]
    Closed,

    /// A value has no lossless representation in the Arrow type chosen for its
    /// column.
    ///
    /// The column name is not repeated — it comes from the server, and an error
    /// message ends up in a log. The index is enough to find the field in the
    /// cursor's schema.
    #[error(
        "column #{column} was typed as `{resolved}` from the first batch, \
         but a `{found}` value appeared further in the stream and cannot be \
         rendered there without loss"
    )]
    ColumnConflict {
        /// Index of the column in the schema, from zero.
        column: usize,
        /// Arrow type chosen for the column.
        resolved: &'static str,
        /// Storage class encountered.
        found: &'static str,
    },

    /// A bound parameter has no SQLite storage class.
    #[error("bound parameter #{index} is a `{type_name}`, which SQLite cannot store")]
    Parameter {
        /// Position of the parameter, from 1.
        index: usize,
        /// Name of the scalar type, as `ScalarValue::type_name` gives it.
        type_name: &'static str,
    },

    /// The number of bound parameters does not match the statement.
    #[error("the statement expects {expected} bound parameter(s), {given} given")]
    ParameterCount {
        /// What the statement expects.
        expected: usize,
        /// What was provided.
        given: usize,
    },

    /// Bound parameters come with a batch of several statements.
    ///
    /// Nothing says which statement they relate to: splitting belongs to
    /// `oxyn-query`, and the driver refuses rather than guess.
    #[error("bound parameters cannot be used with a multi-statement batch: split it first")]
    ParametersWithBatch,

    /// Building the `RecordBatch` failed.
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

/// The family of an engine error.
///
/// Only four situations are **transient**: the file lock held by another process
/// (`SQLITE_BUSY`), the table lock held by another connection (`SQLITE_LOCKED`),
/// the locking protocol failure, and the schema change under the feet of a
/// prepared statement. Everything else — syntax, violated constraint, unreadable
/// database, full disk — does not improve by retrying.
///
/// `SQLITE_INTERRUPT` does not appear here: it is handled upstream by `engine`
/// — named and not linked: the function is internal to the crate —, because
/// its family depends on what the statement was doing.
#[must_use]
pub fn classify(error: &rusqlite::Error) -> ErrorClass {
    let rusqlite::Error::SqliteFailure(inner, _) = error else {
        // The other variants are library usage errors (wrong column index,
        // refused conversion): replaying does not fix them.
        return ErrorClass::Permanent;
    };
    match inner.code {
        ErrorCode::DatabaseBusy
        | ErrorCode::DatabaseLocked
        | ErrorCode::FileLockingProtocolFailed
        | ErrorCode::SchemaChanged => ErrorClass::Transient,
        _ => ErrorClass::Permanent,
    }
}

/// Translates an engine error for a statement **composed by the driver**.
///
/// The engine's message is propagated as is: such a statement binds only
/// literals the driver wrote, so nothing the engine can quote comes from the
/// caller. For the execution of an [`ExecRequest`](oxyn_core::ExecRequest),
/// [`engine_bound`] is the one to call: there, the engine quotes bound values
/// ([I-03](../../../CLAUDE.md#i-03)).
pub(crate) fn engine(error: rusqlite::Error, effect: Effect) -> OxynError {
    engine_bound(error, effect, Bound::Internal)
}

/// Translates an engine error into the domain vocabulary.
///
/// `effect` says what the statement could do; it serves only the interruption
/// case, see the module documentation. `bound` decides the fate of the message:
/// withheld as soon as a caller's value was bound.
///
/// The error's family, however, does **not** depend on `bound`: it is read from
/// the result code, taken before the original error is dropped.
pub(crate) fn engine_bound(error: rusqlite::Error, effect: Effect, bound: Bound) -> OxynError {
    let code = match &error {
        rusqlite::Error::SqliteFailure(inner, _) => Some(*inner),
        _ => None,
    };
    if code.is_some_and(|inner| inner.code == ErrorCode::OperationInterrupted) {
        return match effect {
            Effect::ReadOnly => OxynError::Cancelled,
            Effect::Mutating => driver(hide(error, code, bound), ErrorClass::Ambiguous),
        };
    }
    if matches!(error, rusqlite::Error::InvalidPath(_)) {
        // The `Display` of this variant contains the path (I-03).
        return driver(SqliteError::Path, ErrorClass::Permanent);
    }
    if let Some(module) = missing_module(&error) {
        // Named as data for the interface, which explains this failure rather
        // than parsing a message. Kept even when the message is otherwise
        // withheld: the engine composes it from the schema's module name,
        // never from a bound value (I-03). Permanent: reading again changes
        // nothing until the module is loaded (I-13).
        return OxynError::driver(
            DriverId::sqlite(),
            ErrorClass::Permanent,
            oxyn_core::MissingModule { module },
        );
    }
    let class = classify(&error);
    driver(hide(error, code, bound), class)
}

/// The module of an engine `no such module: <name>` failure.
///
/// The driver is the one place that may read its engine's words: the
/// `SQLITE_ERROR` code alone does not say which failure it is, and the caller
/// then reads the module as data, never the text.
fn missing_module(error: &rusqlite::Error) -> Option<String> {
    let rusqlite::Error::SqliteFailure(failure, Some(message)) = error else {
        return None;
    };
    if failure.code != ErrorCode::Unknown {
        return None;
    }
    let module = message.strip_prefix("no such module: ")?.trim();
    (!module.is_empty()).then(|| module.to_owned())
}

/// The error the driver returns: the engine's, or its code alone when the
/// message could quote a caller's value.
///
/// Only the text from `sqlite3_errmsg` can repeat what was just bound — that is
/// what `SqliteFailure(_, Some(_))` carries. The other `rusqlite` variants are
/// composed by the library from indexes and type names: withholding them would
/// make the diagnostic of a **driver bug** disappear without protecting
/// anything, and nobody could reproduce it. The PostgreSQL driver makes the same
/// distinction for the same reason.
pub(crate) fn hide(
    error: rusqlite::Error,
    code: Option<rusqlite::ffi::Error>,
    bound: Bound,
) -> SqliteError {
    match bound {
        Bound::Caller if matches!(error, rusqlite::Error::SqliteFailure(_, Some(_))) => {
            SqliteError::Withheld { code }
        }
        Bound::Internal | Bound::Caller => SqliteError::Engine(error),
    }
}

/// Wraps a driver error, naming its family.
pub(crate) fn driver(error: SqliteError, class: ErrorClass) -> OxynError {
    OxynError::driver(DriverId::sqlite(), class, error)
}

/// The error of a session whose worker thread has disappeared.
pub(crate) fn closed() -> OxynError {
    driver(SqliteError::Closed, ErrorClass::Permanent)
}

/// The error of opening a database, classified as a connection error.
///
/// Returned by [`Driver::connect`](oxyn_driver::Driver::connect), which has its
/// own variant: the caller tells "the server is unreachable" from "the server
/// rejected the statement" without reading a message. The path never appears in
/// it.
pub(crate) fn open(error: rusqlite::Error) -> OxynError {
    OxynError::Connection(match &error {
        rusqlite::Error::InvalidPath(_) => "the database path is not usable".to_owned(),
        // SQLite's detailed message embeds the file path
        // ("unable to open database file: /home/…"). A path is a connection
        // parameter: it has no place in a message that may end up in a log, a
        // crash report or an AI prompt ([I-03](../../../CLAUDE.md#i-03)). The
        // `ffi::Error` alone renders "Error code 14: unable to open database
        // file" — the extended code and its canonical label, with nothing of the
        // user's installation. The code stays usable data, not an inference to
        // make from the text ([rust.md](../../../.claude/rules/rust.md)).
        rusqlite::Error::SqliteFailure(code, _) => code.to_string(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rusqlite::ffi;

    use super::*;

    fn failure_with(code: ErrorCode) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            ffi::Error {
                code,
                extended_code: 0,
            },
            Some("engine message".to_owned()),
        )
    }

    /// An engine failure whose message quotes a bound value, as
    /// `RAISE(ABORT, 'balance: ' || NEW.amount)` does.
    fn chatty_failure() -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            ffi::Error {
                code: ErrorCode::ConstraintViolation,
                // `SQLITE_CONSTRAINT_TRIGGER`.
                extended_code: 1_811,
            },
            Some("balance: S3NT1N3L-42".to_owned()),
        )
    }

    #[test]
    fn a_lock_is_transient_a_syntax_error_is_not() {
        assert_eq!(
            classify(&failure_with(ErrorCode::DatabaseBusy)),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(&failure_with(ErrorCode::DatabaseLocked)),
            ErrorClass::Transient
        );
        assert_eq!(
            classify(&failure_with(ErrorCode::Unknown)),
            ErrorClass::Permanent
        );
        assert_eq!(
            classify(&failure_with(ErrorCode::ConstraintViolation)),
            ErrorClass::Permanent
        );
        assert_eq!(
            classify(&failure_with(ErrorCode::DatabaseCorrupt)),
            ErrorClass::Permanent,
            "a corrupted database is not repaired by retrying"
        );
    }

    #[test]
    fn an_interrupted_read_is_a_cancellation() {
        let err = engine(
            failure_with(ErrorCode::OperationInterrupted),
            Effect::ReadOnly,
        );
        assert!(err.is_cancelled(), "{err:?}");
        assert!(!err.is_retryable());
    }

    #[test]
    fn an_interrupted_write_is_ambiguous_and_is_not_retried() {
        // I-13: the server may have applied it. Replaying creates a silent
        // duplicate in the user's data.
        let err = engine(
            failure_with(ErrorCode::OperationInterrupted),
            Effect::Mutating,
        );
        assert_eq!(err.class(), ErrorClass::Ambiguous, "{err:?}");
        assert!(!err.is_retryable());
        assert!(
            !err.is_cancelled(),
            "the effect is unknown: it is not a clean cancellation"
        );
    }

    #[test]
    fn an_offending_path_never_comes_out_in_the_message() {
        // I-03: the file path is a connection parameter value.
        let secret = PathBuf::from("/Users/someone/databases/clients-2026.sqlite");
        let err = engine(
            rusqlite::Error::InvalidPath(secret.clone()),
            Effect::ReadOnly,
        );
        let shown = format!("{err}");
        assert!(!shown.contains("clients-2026"), "path leaked: {shown}");
        assert!(!shown.contains("someone"), "path leaked: {shown}");

        let shown = format!("{}", open(rusqlite::Error::InvalidPath(secret)));
        assert!(!shown.contains("clients-2026"), "path leaked: {shown}");
    }

    #[test]
    fn an_engine_error_carries_the_driver_name() {
        let err = engine(failure_with(ErrorCode::Unknown), Effect::ReadOnly);
        assert!(err.to_string().contains("sqlite"), "{err}");
        assert!(err.to_string().contains("engine message"), "{err}");
    }

    #[test]
    fn a_column_conflict_does_not_name_the_column() {
        // A column name comes from the server: it may carry a terminal escape
        // sequence, and an error message ends up in a log.
        let err = SqliteError::ColumnConflict {
            column: 3,
            resolved: "int64",
            found: "blob",
        };
        let shown = err.to_string();
        assert!(shown.contains("#3"), "{shown}");
        assert!(shown.contains("int64") && shown.contains("blob"), "{shown}");
    }

    #[test]
    fn the_engine_message_is_withheld_as_soon_as_a_value_was_bound() {
        // I-03: this message is displayed, logged and persisted by the history.
        // SQLite copies into it what was just bound.
        let err = engine_bound(chatty_failure(), Effect::Mutating, Bound::Caller);
        for shown in [format!("{err}"), format!("{err:?}")] {
            assert!(!shown.contains("S3NT1N3L-42"), "bound value: {shown}");
            assert!(!shown.contains("balance"), "engine message: {shown}");
        }
        // The withdrawal is stated, rather than suggesting a silent error.
        assert!(err.to_string().contains("withheld"), "{err}");
        // The extended result code survives: it is a number, it quotes nothing.
        assert!(err.to_string().contains("1811"), "{err}");
    }

    #[test]
    fn without_bound_value_the_engine_message_passes_unchanged() {
        // Oxyn's audience reads its engine's messages; a reassuring paraphrase would
        // be a defect.
        let err = engine_bound(chatty_failure(), Effect::Mutating, Bound::Internal);
        assert!(err.to_string().contains("balance: S3NT1N3L-42"), "{err}");
        assert_eq!(
            err.to_string(),
            engine(chatty_failure(), Effect::Mutating).to_string(),
            "`engine` is the case without bound value"
        );
    }

    #[test]
    fn withholding_the_message_changes_neither_family_nor_cancellation() {
        // The class is read from the result code, taken before dropping the
        // original error: withholding it must move nothing.
        for (code, expected_class) in [
            (ErrorCode::DatabaseBusy, ErrorClass::Transient),
            (ErrorCode::ConstraintViolation, ErrorClass::Permanent),
        ] {
            let err = engine_bound(failure_with(code), Effect::Mutating, Bound::Caller);
            assert_eq!(err.class(), expected_class, "{err:?}");
        }

        let interrupted_read = engine_bound(
            failure_with(ErrorCode::OperationInterrupted),
            Effect::ReadOnly,
            Bound::Caller,
        );
        assert!(interrupted_read.is_cancelled(), "{interrupted_read:?}");

        // I-13: an interrupted write stays ambiguous, hence not replayable.
        let interrupted_write = engine_bound(
            failure_with(ErrorCode::OperationInterrupted),
            Effect::Mutating,
            Bound::Caller,
        );
        assert_eq!(
            interrupted_write.class(),
            ErrorClass::Ambiguous,
            "{interrupted_write:?}"
        );
        assert!(!interrupted_write.is_retryable());
        assert!(!format!("{interrupted_write:?}").contains("engine message"));
    }

    #[test]
    fn a_request_without_parameter_withholds_nothing() {
        assert_eq!(Bound::of(&[]), Bound::Internal);
        assert_eq!(
            Bound::of(&[ScalarValue::Text("S3NT1N3L-42".to_owned())]),
            Bound::Caller
        );
    }

    #[test]
    fn a_closed_session_is_a_permanent_error() {
        let err = closed();
        assert_eq!(err.class(), ErrorClass::Permanent);
        assert!(!err.is_retryable());
    }

    /// Withholding the message of a driver defect protects nothing and erases the
    /// diagnostic: `InvalidColumnIndex` is composed by `rusqlite` from an index, it
    /// cannot quote any bound value.
    #[test]
    fn only_the_engine_text_is_withheld_when_values_are_bound() {
        let usage = hide(rusqlite::Error::InvalidColumnIndex(3), None, Bound::Caller);
        assert!(
            matches!(usage, SqliteError::Engine(_)),
            "a usage error keeps its message: {usage}"
        );
        assert!(usage.to_string().contains('3'));

        let engine_failure = hide(
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(19),
                Some("CHECK constraint failed: S3NT1N3L-42".to_owned()),
            ),
            Some(rusqlite::ffi::Error::new(19)),
            Bound::Caller,
        );
        let shown = format!("{engine_failure} {engine_failure:?}");
        assert!(
            !shown.contains("S3NT1N3L"),
            "the engine text does not come out: {shown}"
        );
        assert!(shown.contains("19"), "the result code survives: {shown}");
    }
}
