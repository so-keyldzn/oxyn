use oxyn_core::{ErrorClass, OxynError, StatementIntent};

use super::*;

const READ: StatementIntent = StatementIntent::Read;
const WRITE: StatementIntent = StatementIntent::Write;

#[test]
fn a_step_down_is_transient_for_a_read_and_ambiguous_for_a_write() {
    for code in [
        PRIMARY_STEPPED_DOWN,
        NOT_WRITABLE_PRIMARY,
        SHUTDOWN_IN_PROGRESS,
        INTERRUPTED_REPL_STATE,
        NETWORK_TIMEOUT,
        LOCK_TIMEOUT,
    ] {
        assert_eq!(
            class_of_code(code, READ, &[]),
            ErrorClass::Transient,
            "{code}"
        );
        assert_eq!(
            class_of_code(code, WRITE, &[]),
            ErrorClass::Ambiguous,
            "{code}"
        );
        assert_eq!(
            class_of_code(code, StatementIntent::Unknown, &[]),
            ErrorClass::Ambiguous,
            "an unknown intent may write: {code}"
        );
    }
}

#[test]
fn a_retryable_write_label_is_no_licence_to_replay_without_a_transaction_number() {
    let labels = vec![RETRYABLE_WRITE.to_owned()];
    assert_eq!(class_of_code(2, WRITE, &labels), ErrorClass::Ambiguous);
    assert_eq!(class_of_code(2, READ, &labels), ErrorClass::Transient);
}

#[test]
fn an_unknown_commit_result_is_ambiguous_even_for_a_read() {
    let labels = vec![UNKNOWN_COMMIT_RESULT.to_owned()];
    assert_eq!(class_of_code(50, READ, &labels), ErrorClass::Ambiguous);
}

#[test]
fn what_the_command_itself_got_wrong_is_permanent() {
    for code in [
        2,
        9,
        13,
        26,
        59,
        11000,
        121,
        MAX_TIME_MS_EXPIRED,
        MEMORY_LIMIT_NO_DISK_USE,
    ] {
        assert_eq!(
            class_of_code(code, READ, &[]),
            ErrorClass::Permanent,
            "a read that failed applied nothing: {code}"
        );
    }
    for code in [9, 13, 26, 59, 72, 73, 168] {
        assert_eq!(
            class_of_code(code, WRITE, &[]),
            ErrorClass::Permanent,
            "refused before running: {code}"
        );
    }
}

#[test]
fn a_write_that_failed_while_running_is_ambiguous() {
    // A multi-document update stopped by its own `maxTimeMS` or a `killOp`,
    // a `$merge` that met a duplicate key or a type error halfway: part of
    // it may be applied, whatever the cause.
    for code in [
        MAX_TIME_MS_EXPIRED,
        INTERRUPTED,
        46841, // ClientMarkedKilled
        11000,
        2,
        14,
        MEMORY_LIMIT_NO_DISK_USE,
        BSON_OBJECT_TOO_LARGE,
    ] {
        for intent in [WRITE, StatementIntent::Ddl, StatementIntent::Unknown] {
            assert_eq!(
                class_of_code(code, intent, &[]),
                ErrorClass::Ambiguous,
                "{code} {intent}"
            );
        }
    }
}

#[test]
fn a_stopped_read_is_cancelled_and_a_stopped_write_is_unknown() {
    assert!(matches!(stopped(READ), OxynError::Cancelled));
    for intent in [WRITE, StatementIntent::Ddl, StatementIntent::Unknown] {
        let error = stopped(intent);
        assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{intent}");
        assert!(!error.is_retryable());
    }
}

/// The password a command carries, which the hostile server quotes back.
const SECRET: &str = "oxyn-hostile-pw";

/// A command error as the server would answer it, every text field its own.
fn server_error(code: i32, code_name: &str, message: &str) -> MongoLibError {
    let command: mongodb::error::CommandError = bson::deserialize_from_document(
        bson::doc! { "code": code, "codeName": code_name, "errmsg": message },
    )
    .expect("a command error reads from its wire fields");
    MongoLibError::from(ErrorKind::Command(command))
}

#[test]
fn a_server_quoting_a_password_is_not_shown() {
    let driver = DriverId::mongodb();
    // A hostile server quotes the `pwd` of a `createUser` in every field it
    // chooses, for a refusal (permanent) and a failure while writing
    // (ambiguous), the second with a known remedy.
    for (code, intent) in [(2, StatementIntent::Ddl), (BSON_OBJECT_TOO_LARGE, WRITE)] {
        let error = server_error(code, &format!("Bad{SECRET}"), &format!("password={SECRET}"));
        let mapped = map_statement_error(&driver, intent, &error, false, true);
        for shown in [mapped.to_string(), format!("{mapped:?}")] {
            assert!(!shown.contains(SECRET), "{code}: {shown}");
            assert!(shown.contains("withheld"), "{code}: {shown}");
            assert!(shown.contains(&format!("code {code}")), "{code}: {shown}");
        }
        if let OxynError::Driver { source, .. } = &mapped {
            let source = source
                .downcast_ref::<MongoError>()
                .expect("the driver's own error type");
            assert_eq!(source.code(), Some(code), "the code is kept");
            assert_eq!(source.code_name(), None, "the server chose the name");
        }
    }
    // A failed authentication never shows the server's words, whatever the
    // command: they are not the user's to read.
    let error = server_error(AUTHENTICATION_FAILED, "AuthenticationFailed", SECRET);
    let mapped = map_error(&driver, READ, &error, false);
    assert!(matches!(mapped, OxynError::Authentication(_)), "{mapped:?}");
    for shown in [mapped.to_string(), format!("{mapped:?}")] {
        assert!(!shown.contains(SECRET), "{shown}");
    }
}

#[test]
fn a_command_without_password_keeps_the_servers_words() {
    let error = server_error(2, "BadValue", "unknown operator: $bad");
    let mapped = map_statement_error(&DriverId::mongodb(), READ, &error, false, false);
    assert_eq!(
        mapped.to_string(),
        "query rejected: BadValue (2): unknown operator: $bad"
    );
}

#[test]
fn the_server_limits_name_their_remedy() {
    assert!(remedy(MEMORY_LIMIT_NO_DISK_USE).is_some_and(|text| text.contains("allowDiskUse")));
    assert!(remedy(BSON_OBJECT_TOO_LARGE).is_some_and(|text| text.contains("16 MiB")));
    assert_eq!(remedy(2), None);
}
