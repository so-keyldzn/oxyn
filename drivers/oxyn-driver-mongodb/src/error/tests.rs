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
            class_of_code(code, WRITE, &[]),
            ErrorClass::Permanent,
            "{code}"
        );
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

#[test]
fn the_server_limits_name_their_remedy() {
    assert!(remedy(MEMORY_LIMIT_NO_DISK_USE).is_some_and(|text| text.contains("allowDiskUse")));
    assert!(remedy(BSON_OBJECT_TOO_LARGE).is_some_and(|text| text.contains("16 MiB")));
    assert_eq!(remedy(2), None);
}
