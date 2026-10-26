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
fn concurrency_and_resource_errors_follow_the_statement_intent() {
    for code in [ER_LOCK_DEADLOCK, ER_LOCK_WAIT_TIMEOUT] {
        for intent in [StatementIntent::Read, StatementIntent::Write] {
            assert_eq!(
                classify(&server(code, "HY000", "x"), intent),
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
                classify(&server(code, "HY000", "x"), intent),
                ErrorClass::Ambiguous,
                "{code}, {intent:?}"
            );
        }
    }
    for code in [
        ER_OUT_OF_RESOURCES,
        ER_OUTOFMEMORY,
        ER_OUT_OF_SORTMEMORY,
        ER_DISK_FULL,
    ] {
        assert_eq!(
            classify(&server(code, "HY000", "x"), StatementIntent::Read),
            ErrorClass::Transient,
            "{code}, Read"
        );
        for intent in [
            StatementIntent::Write,
            StatementIntent::Unknown,
            StatementIntent::Ddl,
            StatementIntent::Grant,
        ] {
            assert_eq!(
                classify(&server(code, "HY000", "x"), intent),
                ErrorClass::Ambiguous,
                "{code}, {intent:?}"
            );
        }
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
fn connection_admission_errors_are_transient_for_every_intent() {
    for code in [
        ER_CON_COUNT_ERROR,
        ER_TOO_MANY_USER_CONNECTIONS,
        ER_USER_LIMIT_REACHED,
    ] {
        for intent in [
            StatementIntent::Read,
            StatementIntent::Write,
            StatementIntent::Unknown,
            StatementIntent::Ddl,
            StatementIntent::Grant,
        ] {
            assert_eq!(
                classify(&server(code, "HY000", "x"), intent),
                ErrorClass::Transient,
                "{code}, {intent:?}"
            );
        }
    }
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
