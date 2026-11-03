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

/// Every cause a statement can fail with after an earlier one applied,
/// as the worker would have built it before `after_applied` sees it.
fn causes() -> Vec<(&'static str, OxynError)> {
    vec![
        (
            "busy",
            engine(failure_with(ErrorCode::DatabaseBusy), Effect::Mutating),
        ),
        (
            "locked",
            engine(failure_with(ErrorCode::DatabaseLocked), Effect::Mutating),
        ),
        (
            "schema",
            engine(failure_with(ErrorCode::SchemaChanged), Effect::ReadOnly),
        ),
        (
            "interrupted read",
            engine(
                failure_with(ErrorCode::OperationInterrupted),
                Effect::ReadOnly,
            ),
        ),
        (
            "interrupted write",
            engine(
                failure_with(ErrorCode::OperationInterrupted),
                Effect::Mutating,
            ),
        ),
        (
            "constraint",
            engine(
                failure_with(ErrorCode::ConstraintViolation),
                Effect::Mutating,
            ),
        ),
    ]
}

#[test]
fn after_applied_is_never_retryable_nor_cancelled() {
    // I-13: a failure after an applied statement must not invite a replay
    // of the batch, nor claim that nothing was applied.
    for (name, cause) in causes() {
        let err = after_applied(cause);
        assert!(!err.is_retryable(), "{name}: {err:?}");
        assert!(!err.is_cancelled(), "{name}: {err:?}");
        assert!(
            err.to_string().contains("already applied"),
            "{name}: the message must say what applied: {err}"
        );
    }
}

#[test]
fn after_applied_keeps_ambiguous_and_permanent_causes_as_they_are() {
    let interrupted_write = after_applied(engine(
        failure_with(ErrorCode::OperationInterrupted),
        Effect::Mutating,
    ));
    assert_eq!(
        interrupted_write.class(),
        ErrorClass::Ambiguous,
        "the failing statement may have applied too: {interrupted_write:?}"
    );

    let stopped = after_applied(OxynError::OutcomeUnknown(
        "the write was stopped".to_owned(),
    ));
    assert_eq!(stopped.class(), ErrorClass::Ambiguous, "{stopped:?}");

    let constraint = after_applied(engine(
        failure_with(ErrorCode::ConstraintViolation),
        Effect::Mutating,
    ));
    assert_eq!(
        constraint.class(),
        ErrorClass::Permanent,
        "a constraint is not made more alarming than it is: {constraint:?}"
    );
    assert!(
        constraint.to_string().contains("engine message"),
        "the cause stays readable: {constraint}"
    );
}

#[test]
fn after_applied_turns_transient_and_cancelled_into_permanent() {
    for (name, cause) in causes() {
        let before = cause.class();
        let was_cancelled = cause.is_cancelled();
        if before != ErrorClass::Transient && !was_cancelled {
            continue;
        }
        let err = after_applied(cause);
        assert_eq!(err.class(), ErrorClass::Permanent, "{name}: {err:?}");
        // The family the cause had before is no longer true: it must not be
        // read next to a verdict that forbids replaying.
        let shown = err.to_string();
        assert!(!shown.contains("transient"), "{name}: {shown}");
        assert!(!shown.contains("operation cancelled"), "{name}: {shown}");
    }

    let interrupted_read = after_applied(OxynError::Cancelled);
    assert_eq!(interrupted_read.class(), ErrorClass::Permanent);
    assert!(!interrupted_read.is_cancelled());
    assert!(
        interrupted_read.to_string().contains("interrupted"),
        "{interrupted_read}"
    );
}

#[test]
fn after_applied_keeps_a_withheld_message_withheld() {
    // I-03: the wrapper adds fixed text only. The cause was built with the
    // caller's bound value in the statement: its message stays withheld.
    let cause = engine_bound(chatty_failure(), Effect::Mutating, Bound::Caller);
    let err = after_applied(cause);
    for shown in [format!("{err}"), format!("{err:?}")] {
        assert!(!shown.contains("S3NT1N3L-42"), "bound value: {shown}");
        assert!(!shown.contains("balance"), "engine message: {shown}");
    }
    assert!(err.to_string().contains("withheld"), "{err}");
    assert!(err.to_string().contains("1811"), "{err}");
    assert!(err.to_string().contains("already applied"), "{err}");
}

#[test]
fn without_an_applied_statement_the_error_is_untouched() {
    // The first statement of a batch that hits a lock is replayable: nothing
    // ran before it.
    let busy = after_applied_if(
        false,
        engine(failure_with(ErrorCode::DatabaseBusy), Effect::Mutating),
    );
    assert!(busy.is_retryable(), "{busy:?}");
    assert!(!busy.to_string().contains("already applied"), "{busy}");

    let cancelled = after_applied_if(false, OxynError::Cancelled);
    assert!(cancelled.is_cancelled());

    let wrapped = after_applied_if(true, OxynError::Cancelled);
    assert!(!wrapped.is_cancelled());
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
