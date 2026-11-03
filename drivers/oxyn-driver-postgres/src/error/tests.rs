use super::fakes::{FakeDatabase, cut, server_error};
use super::*;

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
fn concurrency_and_resource_errors_follow_the_statement_intent() {
    for code in [
        SQLSTATE_SERIALIZATION_FAILURE,
        SQLSTATE_DEADLOCK,
        SQLSTATE_LOCK_NOT_AVAILABLE,
        SQLSTATE_OUT_OF_MEMORY,
        SQLSTATE_DISK_FULL,
    ] {
        for intent in [StatementIntent::Read, StatementIntent::Write] {
            assert_eq!(
                classify(&server_error(code), intent),
                ErrorClass::Transient,
                "{code}, {intent:?}"
            );
        }
        for intent in [
            StatementIntent::Unknown,
            StatementIntent::Ddl,
            StatementIntent::Grant,
        ] {
            assert_eq!(
                classify(&server_error(code), intent),
                ErrorClass::Ambiguous,
                "{code}, {intent:?}"
            );
        }
    }
}

#[test]
fn connection_admission_errors_are_transient_for_every_intent() {
    for code in [
        SQLSTATE_TOO_MANY_CONNECTIONS,
        SQLSTATE_CONFIG_LIMIT,
        SQLSTATE_CANNOT_CONNECT_NOW,
        SQLSTATE_IDLE_TIMEOUT,
    ] {
        for intent in [
            StatementIntent::Read,
            StatementIntent::Write,
            StatementIntent::Unknown,
            StatementIntent::Ddl,
            StatementIntent::Grant,
        ] {
            assert_eq!(
                classify(&server_error(code), intent),
                ErrorClass::Transient,
                "{code}, {intent:?}"
            );
        }
    }
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
fn a_57014_the_stop_did_not_ask_for_proves_a_rollback_only_for_data_writes_and_reads() {
    // A role's `statement_timeout` interrupts a `CALL` between two of its
    // commits: the cursor sees a plain stream error, no Stop is involved.
    for intent in [StatementIntent::Read, StatementIntent::Write] {
        let error = map_stream_error(
            &DriverId::postgres(),
            intent,
            false,
            Bound::Internal,
            server_error(SQLSTATE_QUERY_CANCELED),
        );
        assert!(error.is_cancelled(), "{intent:?}: {error:?}");
    }
    for intent in [
        StatementIntent::Unknown,
        StatementIntent::Ddl,
        StatementIntent::Grant,
    ] {
        let error = map_stream_error(
            &DriverId::postgres(),
            intent,
            false,
            Bound::Internal,
            server_error(SQLSTATE_QUERY_CANCELED),
        );
        assert!(!error.is_cancelled(), "{intent:?}: {error:?}");
        assert!(!error.is_retryable(), "{intent:?}: {error:?}");
        assert_eq!(
            error.class(),
            ErrorClass::Ambiguous,
            "{intent:?}: {error:?}"
        );
    }
}

#[test]
fn only_the_server_s_57014_answer_proves_an_interruption() {
    // The proof is a SQLSTATE the server sent. A transport error, or any
    // other server answer, proves nothing about what became of a write.
    assert!(is_query_canceled(&server_error(SQLSTATE_QUERY_CANCELED)));
    for other in ["23505", "40001", "57P01", "08006"] {
        assert!(!is_query_canceled(&server_error(other)), "{other}");
    }
    assert!(!is_query_canceled(&cut()));
    assert!(!is_query_canceled(&sqlx::Error::WorkerCrashed));
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
